#!/usr/bin/env node
/**
 * Artifact Smoke Common Harness — 11I-E1
 *
 * Shared offline infrastructure for exact artifact smoke tests.
 * Platform-specific harnesses (E2/E3/E4) import and extend this.
 *
 * Design:
 * - stdlib only, no external dependencies
 * - Deterministic, fail-closed
 * - Evidence model per artifact-smoke-evidence-model.json
 * - Zero network access
 */

import { readFileSync, writeFileSync, existsSync, mkdirSync, rmSync, mkdtempSync, cpSync } from "node:fs";
import { resolve, join, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { randomBytes, createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const REPO_ROOT = resolve(__dirname, "..");
const FIXTURES_DIR = resolve(REPO_ROOT, "src-tauri/tests/fixtures/pdf");
const MANIFEST_PATH = resolve(REPO_ROOT, "src-tauri/tests/fixtures/corpus-manifest.json");

// ─── Bounded Vocabulary ───
export const ARTIFACT_TYPES = Object.freeze([
  "MACOS_DMG",
  "MACOS_APP_BUNDLE",
  "LINUX_DEB",
  "WINDOWS_MSI",
  "WINDOWS_NSIS"
]);

export const PLATFORMS = Object.freeze(["macos", "linux", "windows"]);
export const ARCHITECTURES = Object.freeze(["arm64", "x86_64"]);

export const EVIDENCE_STATUSES = Object.freeze([
  "PASS",
  "FAIL",
  "NOT_RUN",
  "NOT_APPLICABLE",
  "TOOL_UNAVAILABLE",
  "RUNTIME_UNAVAILABLE",
  "ARTIFACT_UNAVAILABLE",
  "TOOL_UNAVAILABLE_OFFLINE"
]);

export const INSTALL_METHODS = Object.freeze([
  "dmg_mount_copy",
  "app_bundle_direct",
  "deb_install",
  "msi_install",
  "nsis_install",
  "portable_extract",
  "none"
]);

export const RUNTIME_SOURCES = Object.freeze([
  "packaged",
  "development_override",
  "staged_development",
  "unknown"
]);

export const WORKER_SPAWN_METHODS = Object.freeze([
  "production_ipc",
  "fake_helper",
  "direct_pdfium",
  "none"
]);

export const EXTERNAL_TOOL_STATUSES = Object.freeze([
  "PASS",
  "FAIL",
  "TOOL_UNAVAILABLE",
  "TOOL_UNAVAILABLE_OFFLINE",
  "NOT_RUN"
]);

// ─── Canonical Fixture Contract ───
export const CANONICAL_SMOKE_FIXTURE = Object.freeze({
  fixtureId: "core-release-smoke",
  filename: "release-smoke.pdf",
  expectedSha256: "727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611",
  expectedByteLength: 4520,
  expectedPageCount: 2,
  requiredTextTokens: [
    "RELEASE_SMOKE_ALPHA_TOKEN",
    "RELEASE_SMOKE_BETA_TOKEN",
    "CANONICAL VERIFICATION TARGET",
    "PAGE_MARKER_PAGE_1",
    "PAGE_MARKER_PAGE_2"
  ],
  expectedDimensions: { width: 595, height: 842 }
});

// ─── Timeout Constants (ms) ───
export const TIMEOUTS = Object.freeze({
  install: 60_000,
  launch: 30_000,
  workerSpawn: 10_000,
  documentOpen: 15_000,
  renderCommand: 15_000,
  shutdown: 10_000
});

// ─── Utility Functions ───

export function generateRunId() {
  return randomBytes(6).toString("hex");
}

export function createTempWorkspace(prefix = "artifact-smoke-") {
  return mkdtempSync(join(tmpdir(), prefix));
}

export function cleanupTempWorkspace(path, bestEffort = true) {
  try {
    rmSync(path, { recursive: true, force: true });
  } catch (e) {
    if (!bestEffort) throw e;
    console.warn(`[cleanup] Failed to remove ${path}: ${e.message}`);
  }
}

export function loadManifest() {
  if (!existsSync(MANIFEST_PATH)) {
    throw new Error(`Corpus manifest not found at ${MANIFEST_PATH}`);
  }
  return JSON.parse(readFileSync(MANIFEST_PATH, "utf-8"));
}

export function verifyFixtureIntegrity(fixturePath, expectedSha256, expectedByteLength) {
  if (!existsSync(fixturePath)) {
    return { ok: false, error: `Fixture not found: ${fixturePath}` };
  }
  const data = readFileSync(fixturePath);
  const actualSha256 = hashSha256(data);
  const actualLength = data.length;

  if (actualSha256 !== expectedSha256) {
    return { ok: false, error: `SHA-256 mismatch: expected ${expectedSha256}, got ${actualSha256}` };
  }
  if (actualLength !== expectedByteLength) {
    return { ok: false, error: `Byte length mismatch: expected ${expectedByteLength}, got ${actualLength}` };
  }
  return { ok: true };
}

export function hashSha256(data) {
  return createHash("sha256").update(data).digest("hex");
}

// Async version for spawn operations
export async function hashSha256Async(data) {
  const { createHash } = await import("node:crypto");
  return createHash("sha256").update(data).digest("hex");
}

export function copyFixtureToWorkspace(fixtureId, workspaceDir) {
  const manifest = loadManifest();
  const fixture = manifest.fixtures.find(f => f.id === fixtureId);
  if (!fixture) {
    throw new Error(`Fixture ${fixtureId} not found in manifest`);
  }
  const srcPath = resolve(REPO_ROOT, fixture.path);
  const destPath = join(workspaceDir, basename(fixture.path));

  // Verify before copy
  const verified = verifyFixtureIntegrity(srcPath, fixture.sha256, fixture.byteLength);
  if (!verified.ok) {
    throw new Error(`Source fixture integrity check failed: ${verified.error}`);
  }

  cpSync(srcPath, destPath);
  return destPath;
}

export function spawnWithTimeout(command, args, options = {}, timeoutMs = 30_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { ...options, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    let timedOut = false;

    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
      reject(new Error(`Process timed out after ${timeoutMs}ms: ${command} ${args.join(" ")}`));
    }, timeoutMs);

    child.stdout.on("data", d => stdout += d.toString());
    child.stderr.on("data", d => stderr += d.toString());

    child.on("close", (code, signal) => {
      clearTimeout(timer);
      if (timedOut) return;
      resolve({ code, signal, stdout, stderr });
    });

    child.on("error", err => {
      clearTimeout(timer);
      if (timedOut) return;
      reject(err);
    });
  });
}

// Sync version for simple commands
export function spawnSyncWithTimeout(command, args, options = {}, timeoutMs = 30_000) {
  try {
    const result = spawnSync(command, args, { ...options, timeout: timeoutMs, stdio: ["ignore", "pipe", "pipe"] });
    return {
      code: result.status,
      signal: result.signal,
      stdout: result.stdout?.toString() || "",
      stderr: result.stderr?.toString() || "",
      timedOut: false
    };
  } catch (e) {
    if (e.code === "ETIMEDOUT" || e.message?.includes("timeout")) {
      return { code: null, signal: "SIGKILL", stdout: "", stderr: e.message, timedOut: true };
    }
    throw e;
  }
}

// ─── Evidence Construction ───

export function createEmptyEvidence(platform, architecture, artifactType, artifactPath, buildProvenance) {
  return {
    schemaVersion: "1.0.0",
    platform,
    architecture,
    artifactType,
    artifactIdentity: {
      filePath: artifactPath,
      sha256: null,
      byteLength: null,
      buildProvenance
    },
    installResult: { status: "NOT_RUN", method: "none", installPath: null, durationMs: 0, error: null },
    launchResult: { status: "NOT_RUN", executablePath: null, pid: null, startupDurationMs: 0, error: null },
    runtimeResolutionResult: { status: "NOT_RUN", resolvedPath: null, source: "unknown", sha256: null, error: null },
    smokeDocumentResult: {
      status: "NOT_RUN",
      fixtureId: "core-release-smoke",
      fixturePath: null,
      pageCount: 0,
      textTokensFound: [],
      renderTest: { status: "NOT_RUN", operation: "none", dimensions: "0x0", durationMs: 0 },
      workerUsed: false,
      workerSpawnMethod: "none",
      error: null
    },
    shutdownResult: { status: "NOT_RUN", cleanExit: false, workerCleanup: false, durationMs: 0, error: null },
    hardeningResult: { status: "NOT_APPLICABLE", error: null },
    externalValidationResults: [],
    overallStatus: "FAIL",
    timestamp: new Date().toISOString(),
    durationTotalMs: 0
  };
}

export function writeEvidence(evidence, outputPath) {
  writeFileSync(outputPath, JSON.stringify(evidence, null, 2), "utf-8");
}

export function computeOverallStatus(evidence) {
  const requiredChecks = [
    evidence.installResult.status,
    evidence.launchResult.status,
    evidence.runtimeResolutionResult.status,
    evidence.smokeDocumentResult.status,
    evidence.shutdownResult.status
  ];

  // Platform-specific required hardening (e.g. CWD/PATH DLL contamination).
  // NOT_APPLICABLE means the platform does not require this check; any other
  // non-PASS status (FAIL / NOT_RUN) fails closed.
  if (
    evidence.hardeningResult
    && typeof evidence.hardeningResult.status === "string"
    && evidence.hardeningResult.status !== "NOT_APPLICABLE"
  ) {
    requiredChecks.push(evidence.hardeningResult.status);
  }

  // All required must be PASS
  const allPass = requiredChecks.every(s => s === "PASS");
  // Any FAIL = overall FAIL
  const anyFail = requiredChecks.some(s => s === "FAIL");

  if (anyFail) return "FAIL";
  if (allPass) return "PASS";
  return "FAIL"; // NOT_RUN on required = FAIL
}

export function validateEvidenceSchema(evidence) {
  const required = [
    "schemaVersion", "platform", "architecture", "artifactType",
    "artifactIdentity", "installResult", "launchResult",
    "runtimeResolutionResult", "smokeDocumentResult",
    "shutdownResult", "externalValidationResults",
    "overallStatus", "timestamp", "durationTotalMs"
  ];

  for (const key of required) {
    if (!(key in evidence)) {
      throw new Error(`Evidence missing required field: ${key}`);
    }
  }

  if (!EVIDENCE_STATUSES.includes(evidence.overallStatus)) {
    throw new Error(`Invalid overallStatus: ${evidence.overallStatus}`);
  }

  // Validate runtime source is "packaged" for PASS
  if (evidence.runtimeResolutionResult.status === "PASS" &&
      evidence.runtimeResolutionResult.source !== "packaged") {
    throw new Error(`runtimeResolutionResult.source must be 'packaged' for PASS, got '${evidence.runtimeResolutionResult.source}'`);
  }

  // Validate worker spawn method
  if (evidence.smokeDocumentResult.workerUsed &&
      evidence.smokeDocumentResult.workerSpawnMethod !== "production_ipc") {
    throw new Error(`workerSpawnMethod must be 'production_ipc' when workerUsed=true, got '${evidence.smokeDocumentResult.workerSpawnMethod}'`);
  }

  return true;
}

// ─── CLI Helpers (for platform harness entry points) ───

export function parseCommonArgs(argv) {
  const args = {
    artifactPath: null,
    outputPath: null,
    keepTemp: false,
    verbose: false,
    help: false
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--artifact" && argv[i + 1]) {
      args.artifactPath = resolve(argv[++i]);
    } else if (arg === "--output" && argv[i + 1]) {
      args.outputPath = resolve(argv[++i]);
    } else if (arg === "--keep-temp") {
      args.keepTemp = true;
    } else if (arg === "--verbose") {
      args.verbose = true;
    } else if (arg === "--help" || arg === "-h") {
      args.help = true;
    }
  }
  return args;
}

export function printCommonUsage() {
  console.log(`
Usage: node <platform-harness>.mjs --artifact <path> [options]

Required:
  --artifact <path>    Path to the exact artifact (.dmg, .deb, .msi, .app, .exe)

Options:
  --output <path>      Write evidence JSON to this path (default: stdout)
  --keep-temp          Do not clean up temporary workspace on exit
  --verbose            Verbose logging
  --help               Show this help

Environment:
  ARTIFACT_SMOKE_TIMEOUT_MULTIPLIER  Multiply all timeouts (default: 1)
`);
}

export function validateRequiredArgs(args) {
  if (!args.artifactPath) {
    throw new Error("Missing required --artifact argument");
  }
  if (!existsSync(args.artifactPath)) {
    throw new Error(`Artifact not found: ${args.artifactPath}`);
  }
}

// ─── Platform Detection ───

export function detectPlatform() {
  const platform = process.platform;
  if (platform === "darwin") return "macos";
  if (platform === "linux") return "linux";
  if (platform === "win32") return "windows";
  throw new Error(`Unsupported platform: ${platform}`);
}

export function detectArchitecture() {
  const arch = process.arch;
  if (arch === "arm64" || arch === "aarch64") return "arm64";
  if (arch === "x64" || arch === "x86_64") return "x86_64";
  throw new Error(`Unsupported architecture: ${arch}`);
}

// ─── Exit Codes ───
export const EXIT_CODES = Object.freeze({
  SUCCESS: 0,
  REQUIRED_CHECK_FAILED: 1,
  ARTIFACT_UNAVAILABLE: 2,
  INVALID_ARGS: 3,
  INTERNAL_ERROR: 4
});

export function exitWithEvidence(evidence, outputPath) {
  const overall = computeOverallStatus(evidence);
  evidence.overallStatus = overall;
  evidence.timestamp = new Date().toISOString();

  if (outputPath) {
    writeEvidence(evidence, outputPath);
  } else {
    console.log(JSON.stringify(evidence, null, 2));
  }

  validateEvidenceSchema(evidence);

  if (overall === "PASS") {
    process.exit(EXIT_CODES.SUCCESS);
  } else if (evidence.installResult.status === "ARTIFACT_UNAVAILABLE" ||
             evidence.launchResult.status === "ARTIFACT_UNAVAILABLE") {
    process.exit(EXIT_CODES.ARTIFACT_UNAVAILABLE);
  } else {
    process.exit(EXIT_CODES.REQUIRED_CHECK_FAILED);
  }
}

export default {
  ARTIFACT_TYPES,
  PLATFORMS,
  ARCHITECTURES,
  EVIDENCE_STATUSES,
  INSTALL_METHODS,
  RUNTIME_SOURCES,
  WORKER_SPAWN_METHODS,
  EXTERNAL_TOOL_STATUSES,
  CANONICAL_SMOKE_FIXTURE,
  TIMEOUTS,
  generateRunId,
  createTempWorkspace,
  cleanupTempWorkspace,
  loadManifest,
  verifyFixtureIntegrity,
  copyFixtureToWorkspace,
  spawnWithTimeout,
  spawnSyncWithTimeout,
  createEmptyEvidence,
  writeEvidence,
  computeOverallStatus,
  validateEvidenceSchema,
  parseCommonArgs,
  printCommonUsage,
  validateRequiredArgs,
  detectPlatform,
  detectArchitecture,
  EXIT_CODES,
  exitWithEvidence
};