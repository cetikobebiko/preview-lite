#!/usr/bin/env node
/**
 * Test Evidence Collector — 11I-F2
 *
 * Runs the FAST_LOCAL execution-class targets named in the canonical
 * test-execution-registry.json, captures machine-readable per-target evidence
 * (test count + exit code + vitest aggregate counts), and writes a single
 * evidence JSON file that scripts/verify-test-evidence.mjs consumes
 * (--cargo-json / --vitest-json).
 *
 * Why this exists: "Exit 0 is not PASS." (B5 §14/§20, §30 false-green contract).
 * A `cargo test --test X` that runs 0 tests still exits 0. This collector
 * records the observed count so the fail-closed validator can reject the
 * ZERO_TESTS / FILTER_NO_MATCH masquerade.
 *
 * Design:
 *  - stdlib only, deterministic, offline (no network; `--offline` optional)
 *  - target list derived from the registry (single source of truth), NOT a
 *    parallel hardcoded list
 *  - fail-closed: any command non-zero, any observed 0, or any count below the
 *    registry default lower bound => collector exits 1
 *
 * Usage:
 *   node scripts/collect-test-evidence.mjs --output <evidence.json> [--offline] [--selftest]
 *
 * Emitted evidence schema (consumed by verify-test-evidence.mjs):
 *   {
 *     "targets": [ { "target": "src-tauri/tests/foo.rs", "id": "rust:foo",
 *                    "observedCount": N, "filteredCount": 0, "exitCode": 0 } ],
 *     "numTotalTests": N, "numPassedTests": N, "numSkippedTests": S,
 *     "numFailedTests": F, "exitCode": 0|1
 *   }
 */

import { spawnSync } from "node:child_process";
import {
  readFileSync,
  writeFileSync,
  mkdtempSync,
  rmSync,
  existsSync,
} from "node:fs";
import { resolve, join, basename, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const REPO_ROOT = resolve(__dirname, "..");
const SRC_TAURI_DIR = resolve(REPO_ROOT, "src-tauri");
const REGISTRY_PATH = resolve(REPO_ROOT, "docs/engineering/test-execution-registry.json");

// ─── Pure parsers (unit-testable via --selftest) ───

export function parseCargoOutput(stdout, stderr) {
  const text = `${stdout || ""}\n${stderr || ""}`;
  let running = null;
  const runMatches = [...text.matchAll(/\brunning\s+(\d+)\s+tests?/gi)];
  if (runMatches.length > 0) running = parseInt(runMatches[runMatches.length - 1][1], 10);

  const result = { passed: null, failed: null, ignored: null };
  const resMatch = /test\s+result:\s*(\w+)\s*\.\s*(\d+)\s+passed;\s*(\d+)\s+failed;\s*(\d+)\s+ignored;/i.exec(text);
  if (resMatch) {
    result.passed = parseInt(resMatch[2], 10);
    result.failed = parseInt(resMatch[3], 10);
    result.ignored = parseInt(resMatch[4], 10);
  }
  return { running, passed: result.passed, failed: result.failed, ignored: result.ignored };
}

export function minimumExpectedForDefaultProfile(rec) {
  if (rec.expectedTestPresence === "conditional" && rec.expectedTestCountByProfile) {
    if (typeof rec.expectedTestCountByProfile.default === "number") {
      return rec.expectedTestCountByProfile.default;
    }
    if (typeof rec.expectedTestCountByProfile.default_non_macos === "number") {
      return rec.expectedTestCountByProfile.default_non_macos;
    }
    const vals = Object.values(rec.expectedTestCountByProfile).filter((v) => typeof v === "number");
    if (vals.length > 0) return Math.min(...vals);
  }
  if (rec.expectedTestPresence === "at_least_one") return 1;
  if (rec.expectedTestPresence === "exact") return rec.expectedTestCount;
  return rec.expectedTestCount ?? 1;
}

function parseArgs() {
  const argv = process.argv.slice(2);
  const opts = { output: null, offline: false, selftest: false, help: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--output" && argv[i + 1]) opts.output = resolve(argv[++i]);
    else if (a === "--offline") opts.offline = true;
    else if (a === "--selftest") opts.selftest = true;
    else if (a === "--help" || a === "-h") opts.help = true;
  }
  return opts;
}

function selftest() {
  const pass = parseCargoOutput(
    "",
    "   Compiling preview-alternative v0.1.0 (...)\n    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.00s\n     Running tests/document_io_fs.rs (target/debug/deps/document_io_fs-abc)\n\nrunning 1 test\ntest atomic_publication_is_fail_closed ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n",
  );
  const zero = parseCargoOutput(
    "",
    "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
  );
  const checks = [
    ["single-run parse", pass.running === 1 && pass.passed === 1 && pass.failed === 0],
    ["zero-run parse", zero.running === 0 && zero.passed === 0],
    ["minExpected at_least_one", minimumExpectedForDefaultProfile({ expectedTestPresence: "at_least_one", expectedTestCount: 28 }) === 1],
    ["minExpected exact", minimumExpectedForDefaultProfile({ expectedTestPresence: "exact", expectedTestCount: 19 }) === 19],
    ["minExpected conditional default", minimumExpectedForDefaultProfile({ expectedTestPresence: "conditional", expectedTestCountByProfile: { default: 1, with_x: 13 } }) === 1],
  ];
  const failed = checks.filter(([, ok]) => !ok);
  for (const [name, ok] of checks) console.log(`  ${ok ? "ok" : "FAIL"}  ${name}`);
  if (failed.length) {
    console.error(`SELFTEST FAILED: ${failed.length} check(s)`);
    process.exit(1);
  }
  console.log("SELFTEST PASSED");
  process.exit(0);
}

function runOneRustTarget(rec, offline) {
  const name = basename(rec.target).replace(/\.rs$/, "");
  const args = ["test", "--locked"];
  if (offline) args.push("--offline");
  args.push("--test", name);

  const child = spawnSync("cargo", args, { cwd: SRC_TAURI_DIR, encoding: "utf8" });
  const spawnError = child.error ? child.error.message : null;
  const exitCode = child.status ?? (child.error ? 1 : -1);
  const parsed = parseCargoOutput(child.stdout, child.stderr);

  return {
    target: rec.target,
    id: rec.id,
    observedCount: parsed.running ?? 0,
    passed: parsed.passed,
    failed: parsed.failed,
    ignored: parsed.ignored,
    filteredCount: 0,
    exitCode,
    spawnError,
  };
}

function runVitest(outputDir) {
  const reportPath = join(outputDir, "vitest-report.json");
  const args = ["run", "test", "--", "--reporter=json", `--outputFile=${reportPath}`];
  const child = spawnSync("npm", args, { cwd: REPO_ROOT, encoding: "utf8" });
  const spawnError = child.error ? child.error.message : null;
  const exitCode = child.status ?? (child.error ? 1 : -1);

  let vitest = null;
  if (existsSync(reportPath)) {
    try {
      const raw = JSON.parse(readFileSync(reportPath, "utf-8"));
      vitest = {
        numTotalTests: raw.numTotalTests ?? 0,
        numPassedTests: raw.numPassedTests ?? 0,
        numSkippedTests: raw.numPendingTests ?? raw.numSkippedTests ?? 0,
        numFailedTests: raw.numFailedTests ?? 0,
        exitCode,
      };
    } catch (e) {
      vitest = { numTotalTests: -1, numPassedTests: 0, numSkippedTests: 0, numFailedTests: -1, exitCode, parseError: e.message };
    }
  } else {
    vitest = { numTotalTests: -1, numPassedTests: 0, numSkippedTests: 0, numFailedTests: -1, exitCode, missingReport: true };
  }
  return { vitest, spawnError };
}

function run() {
  const opts = parseArgs();
  if (opts.help) {
    console.log("Usage: node scripts/collect-test-evidence.mjs --output <evidence.json> [--offline] [--selftest]");
    process.exit(0);
  }
  if (opts.selftest) selftest();
  if (!opts.output) {
    console.error("ERROR: --output <evidence.json> is required");
    process.exit(1);
  }

  const registry = JSON.parse(readFileSync(REGISTRY_PATH, "utf-8"));
  const fastTargets = (registry.targets || []).filter((t) => t.executionClass === "FAST_LOCAL");

  const outputDir = mkdtempSync(join(tmpdir(), "collect-test-evidence-"));
  const errors = [];
  const targets = [];

  for (const rec of fastTargets) {
    if (rec.language === "rust") {
      const ev = runOneRustTarget(rec, opts.offline);
      targets.push(ev);
      const minExpected = minimumExpectedForDefaultProfile(rec);
      if (ev.spawnError) {
        errors.push(`DEPENDENCY_UNAVAILABLE: '${rec.id}' spawn failed: ${ev.spawnError}`);
      } else if (ev.exitCode !== 0) {
        errors.push(`FAIL: '${rec.id}' exit ${ev.exitCode} (observed ${ev.observedCount})`);
      } else if (ev.observedCount === 0) {
        errors.push(`ZERO_TESTS: '${rec.id}' observed 0 tests but expected >= ${minExpected}`);
      } else if (rec.expectedTestPresence === "exact" && ev.observedCount !== rec.expectedTestCount) {
        errors.push(`EXPECTED_EVIDENCE_MISSING: '${rec.id}' exact expected ${rec.expectedTestCount}, observed ${ev.observedCount}`);
      } else if (ev.observedCount < minExpected) {
        errors.push(`EXPECTED_EVIDENCE_MISSING: '${rec.id}' observed ${ev.observedCount} < default lower bound ${minExpected}`);
      }
    } else if (rec.id === "js:vitest") {
      // handled separately below (single frontend aggregate)
    } else {
      errors.push(`EXPECTED_EVIDENCE_MISSING: '${rec.id}' unhandled FAST_LOCAL language ${rec.language}`);
    }
  }

  const { vitest, spawnError } = runVitest(outputDir);
  if (spawnError) {
    errors.push(`DEPENDENCY_UNAVAILABLE: vitest spawn failed: ${spawnError}`);
  } else if (vitest.exitCode !== 0) {
    errors.push(`FAIL: vitest exit ${vitest.exitCode} (failed ${vitest.numFailedTests})`);
  } else if (vitest.numTotalTests === 0) {
    errors.push(`ZERO_TESTS: vitest reported 0 tests`);
  }

  const evidence = {
    schemaVersion: "1.0.0",
    collectorRef: "collect-test-evidence.mjs",
    generatedAt: new Date().toISOString(),
    executionClass: "FAST_LOCAL",
    targets,
    numTotalTests: vitest?.numTotalTests ?? 0,
    numPassedTests: vitest?.numPassedTests ?? 0,
    numSkippedTests: vitest?.numSkippedTests ?? 0,
    numFailedTests: vitest?.numFailedTests ?? 0,
    exitCode: errors.length > 0 ? 1 : 0,
  };

  writeFileSync(opts.output, JSON.stringify(evidence, null, 2) + "\n", "utf-8");
  rmSync(outputDir, { recursive: true, force: true });

  console.log(`Evidence written: ${opts.output}`);
  console.log(`  rust targets: ${targets.length}`);
  for (const t of targets) console.log(`    ${t.id}: observed=${t.observedCount} exit=${t.exitCode}${t.spawnError ? " spawnError=" + t.spawnError : ""}`);
  console.log(`  vitest: total=${evidence.numTotalTests} passed=${evidence.numPassedTests} skipped=${evidence.numSkippedTests} failed=${evidence.numFailedTests}`);

  if (errors.length > 0) {
    console.error(`\nCollector FAILED (fail-closed, ${errors.length} error(s)):`);
    for (const e of errors) console.error(`  - ${e}`);
    process.exit(1);
  }

  console.log("Collector PASSED (evidence captured; verify-test-evidence.mjs is the fail-closed gate).");
  process.exit(0);
}

run();