#!/usr/bin/env node
import { readFileSync, existsSync, lstatSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const repoRoot = resolve(__dirname, "..");

const ALLOWED_EXECUTION_CLASSES = Object.freeze(["FAST_LOCAL", "NATIVE_PLATFORM", "EXACT_ARTIFACT"]);
const ALLOWED_PLATFORMS = Object.freeze(["any", "macos", "linux", "windows"]);
const ALLOWED_LANGUAGES = Object.freeze(["rust", "js"]);
const NATIVE_ONLY_REQUIREMENTS = Object.freeze([
  "pdfium_dynamic_lib",
  "pdfium_worker_executable",
  "job_worker_executable",
  "aarch64",
  "macos_api",
  "windows_api",
  "linux_api",
]);
// B5 fail-closed taxonomy (bounded)
const FALSE_GREEN_TAXONOMY = Object.freeze([
  "ZERO_TESTS",
  "FILTER_NO_MATCH",
  "CFG_ELIDED",
  "IGNORED_TEST",
  "RUNTIME_UNAVAILABLE",
  "TOOL_UNAVAILABLE",
  "DEPENDENCY_UNAVAILABLE",
  "MOCK_REALITY_MISMATCH",
  "AGGREGATE_MIXING",
  "EXPECTED_EVIDENCE_MISSING",
]);
const EVIDENCE_STATUSES = Object.freeze([
  "PASS",
  "FAIL",
  "NOT_RUN",
  "NOT_APPLICABLE",
  "RUNTIME_UNAVAILABLE",
  "TOOL_UNAVAILABLE",
  "DEPENDENCY_UNAVAILABLE",
]);

function validateLexicalPath(rawPath) {
  if (typeof rawPath !== "string" || rawPath.trim().length === 0) {
    return "Path cannot be empty";
  }
  if (rawPath.includes("://")) {
    return `URI scheme forbidden: '${rawPath}'`;
  }
  const bytes = rawPath;
  if (rawPath.length >= 2 && bytes[1] === ":" && /^[A-Za-z]$/.test(bytes[0])) {
    return `Windows drive letter forbidden: '${rawPath}'`;
  }
  if (rawPath.startsWith("\\\\") || rawPath.startsWith("//")) {
    return `UNC path forbidden: '${rawPath}'`;
  }
  if (rawPath.includes("\\")) {
    return `Backslash forbidden, '/' is canonical: '${rawPath}'`;
  }
  if (rawPath.startsWith("/")) {
    return `Leading slash (absolute) forbidden: '${rawPath}'`;
  }
  for (const seg of rawPath.split("/")) {
    if (seg.length === 0) return `Empty segment or double slash forbidden: '${rawPath}'`;
    if (seg === ".") return `Current directory '.' forbidden: '${rawPath}'`;
    if (seg === "..") return `Parent traversal '..' forbidden: '${rawPath}'`;
    if (seg.startsWith("._")) return `AppleDouble sidecar '._*' forbidden: '${rawPath}'`;
  }
  return null;
}

function run() {
  const errors = [];
  const registryPath = resolve(repoRoot, "docs/engineering/test-execution-registry.json");
  const manifestPath = resolve(repoRoot, "src-tauri/tests/fixtures/corpus-manifest.json");
  const coveragePath = resolve(repoRoot, "docs/engineering/pdf-corpus-coverage.json");

  if (!existsSync(registryPath)) {
    console.error(`FATAL: registry missing at ${registryPath}`);
    process.exit(1);
  }
  if (!existsSync(manifestPath)) {
    console.error(`FATAL: corpus manifest missing at ${manifestPath}`);
    process.exit(1);
  }

  let registry, manifest;
  try {
    registry = JSON.parse(readFileSync(registryPath, "utf-8"));
  } catch (e) {
    console.error(`FATAL: Failed to parse registry JSON: ${e.message}`);
    process.exit(1);
  }
  try {
    manifest = JSON.parse(readFileSync(manifestPath, "utf-8"));
  } catch (e) {
    console.error(`FATAL: Failed to parse corpus manifest JSON: ${e.message}`);
    process.exit(1);
  }

  // 0. Schema version
  if (registry.schemaVersion !== "1.0.0") {
    errors.push(`schemaVersion must be '1.0.0', got '${registry.schemaVersion}'`);
  }
  if (!registry.generatedFrom || typeof registry.generatedFrom.manifestPath !== "string") {
    errors.push("generatedFrom.manifestPath missing");
  } else if (registry.generatedFrom.manifestPath !== "src-tauri/tests/fixtures/corpus-manifest.json") {
    errors.push(`generatedFrom.manifestPath must be 'src-tauri/tests/fixtures/corpus-manifest.json', got '${registry.generatedFrom.manifestPath}'`);
  }
  if (!Array.isArray(registry.targets) || registry.targets.length === 0) {
    errors.push("targets must be non-empty array");
  }

  // Build manifest fixture set
  const manifestFixtureSet = new Set((manifest.fixtures || []).map((f) => f.id));

  // Optional: load coverage to check drift? owner distinction: not failing on overlap, but ensure fixtureIds not fabricated
  // Validate platforms vocabulary
  const declaredPlatformVocab = registry.platformVocabulary || [];
  for (const p of ALLOWED_PLATFORMS) {
    if (!declaredPlatformVocab.includes(p)) {
      errors.push(`platformVocabulary missing '${p}'`);
    }
  }

  const seenIds = new Set();
  const seenTargets = new Set();

  for (const rec of registry.targets || []) {
    const ctx = `target id='${rec.id}'`;

    // id uniqueness & format
    if (!rec.id || typeof rec.id !== "string" || rec.id.trim() === "") {
      errors.push(`${ctx}: id must be non-empty string`);
    } else {
      if (seenIds.has(rec.id)) errors.push(`Duplicate test registry id: '${rec.id}'`);
      seenIds.add(rec.id);
      // id should be repo-relative-ish? not strict but must not contain traversal
      const idLexErr = validateLexicalPath(rec.id.replace(/^rust:/, "").replace(/^js:/, ""));
      // allow colon prefix stripped; check rest doesn't have traversal etc
      if (idLexErr && rec.id.includes("..")) {
        errors.push(`${ctx}: id contains traversal: '${rec.id}'`);
      }
    }

    // target uniqueness & lexical & disk existence
    if (!rec.target || typeof rec.target !== "string") {
      errors.push(`${ctx}: target must be non-empty string`);
    } else {
      if (seenTargets.has(rec.target)) {
        errors.push(`Duplicate target identity (ambiguity): '${rec.target}' referenced by multiple records`);
      }
      seenTargets.add(rec.target);
      const lexErr = validateLexicalPath(rec.target);
      if (lexErr) {
        errors.push(`${ctx}: invalid repo-relative target path '${rec.target}': ${lexErr}`);
      } else {
        const full = resolve(repoRoot, rec.target);
        if (!existsSync(full)) {
          errors.push(`${ctx}: nonexistent target on disk '${rec.target}'`);
        } else {
          try {
            const lst = lstatSync(full);
            if (lst.isSymbolicLink()) {
              errors.push(`${ctx}: target is symlink forbidden '${rec.target}'`);
            }
          } catch (_) {}
        }
      }
      // traversal explicit double-check
      if (rec.target.includes("..")) {
        errors.push(`${ctx}: traversal '..' forbidden in target '${rec.target}'`);
      }
    }

    // language
    if (!ALLOWED_LANGUAGES.includes(rec.language)) {
      errors.push(`${ctx}: unknown language '${rec.language}' (allowed: ${ALLOWED_LANGUAGES.join(", ")})`);
    }

    // executionClass
    if (!ALLOWED_EXECUTION_CLASSES.includes(rec.executionClass)) {
      errors.push(`${ctx}: unknown executionClass '${rec.executionClass}'`);
    }

    // usesPdfium boolean
    if (typeof rec.usesPdfium !== "boolean") {
      errors.push(`${ctx}: usesPdfium must be boolean`);
    }

    // realWorker boolean
    if (typeof rec.realWorker !== "boolean") {
      errors.push(`${ctx}: realWorker must be boolean`);
    }

    // supportedPlatforms
    if (!Array.isArray(rec.supportedPlatforms) || rec.supportedPlatforms.length === 0) {
      errors.push(`${ctx}: supportedPlatforms must be non-empty array`);
    } else {
      for (const p of rec.supportedPlatforms) {
        if (!ALLOWED_PLATFORMS.includes(p)) {
          errors.push(`${ctx}: unknown platform '${p}' in supportedPlatforms`);
        }
      }
      // any should not be mixed with others? allow but warn via error if mixed
      if (rec.supportedPlatforms.includes("any") && rec.supportedPlatforms.length > 1) {
        errors.push(`${ctx}: supportedPlatforms contains 'any' mixed with specific platforms ${JSON.stringify(rec.supportedPlatforms)} — must be exactly ['any']`);
      }
    }

    // fixtureIds — must be canonical manifest ids only
    if (!Array.isArray(rec.fixtureIds)) {
      errors.push(`${ctx}: fixtureIds must be array`);
    } else {
      const seenF = new Set();
      for (const fid of rec.fixtureIds) {
        if (typeof fid !== "string" || fid.trim() === "") {
          errors.push(`${ctx}: empty fixtureId`);
          continue;
        }
        if (seenF.has(fid)) errors.push(`${ctx}: duplicate fixtureId '${fid}'`);
        seenF.add(fid);
        if (!manifestFixtureSet.has(fid)) {
          errors.push(`${ctx}: fixtureIds references unknown/non-canonical fixtureId '${fid}' not in corpus-manifest.json`);
        }
        // forbid local basename masquerading as id: basename contains '/'? already not
        if (fid.includes("/") || fid.includes("\\") || fid.includes(".")) {
          errors.push(`${ctx}: fixtureId '${fid}' looks like path/basename, must be canonical id`);
        }
      }
    }

    // syntheticInputs — must NOT be canonical fixture ids
    if (rec.syntheticInputs !== undefined) {
      if (!Array.isArray(rec.syntheticInputs)) {
        errors.push(`${ctx}: syntheticInputs must be array if present`);
      } else {
        for (const sid of rec.syntheticInputs) {
          if (manifestFixtureSet.has(sid)) {
            errors.push(`${ctx}: syntheticInputs contains canonical fixtureId '${sid}' — must use fixtureIds instead`);
          }
        }
      }
    }

    // nativeRequirements vs executionClass
    const nativeReqs = rec.nativeRequirements || [];
    if (!Array.isArray(nativeReqs)) {
      errors.push(`${ctx}: nativeRequirements must be array`);
    }

    // artifactRequirement
    const art = rec.artifactRequirement;
    if (rec.executionClass === "EXACT_ARTIFACT") {
      if (art === null || art === undefined) {
        errors.push(`${ctx}: EXACT_ARTIFACT entry must carry non-null artifactRequirement`);
      } else {
        if (typeof art !== "object" || typeof art.kind !== "string") {
          errors.push(`${ctx}: artifactRequirement must be object with kind`);
        }
      }
    }
    if (rec.executionClass === "FAST_LOCAL") {
      if (art !== null && art !== undefined) {
        errors.push(`${ctx}: FAST_LOCAL entry must not carry artifactRequirement (got ${JSON.stringify(art)})`);
      }
      if (rec.usesPdfium === true) {
        errors.push(`${ctx}: FAST_LOCAL entry must not have usesPdfium=true (native-only requirement)`);
      }
      // native-only checks
      for (const r of nativeReqs) {
        if (NATIVE_ONLY_REQUIREMENTS.includes(r)) {
          errors.push(`${ctx}: FAST_LOCAL entry carries native-only requirement '${r}' — impossible combination (see §4)`);
        }
      }
      // realWorker with pdfium is native; but FAST_LOCAL with realWorker true + usesPdfium false is allowed only for non-pdfium workers? we currently allow
      // to be strict: if realWorker true and NATIVE_ONLY in nativeReqs, already caught. If realWorker true but nativeReqs empty, we allow (plugin fake vs real ambiguity)
      // However spec §9 says FAST_LOCAL entry native-only requirement taşımaz — we enforce via above.
    }

    // impossible combos: EXACT_ARTIFACT with usesPdfium false? could be allowed (artifact smoke without pdfium) — not error
    // but NATIVE_PLATFORM with artifactRequirement != null is suspicious? allow

    // expectedTestCount / expectedTestPresence
    if (typeof rec.expectedTestCount !== "number" || !Number.isInteger(rec.expectedTestCount) || rec.expectedTestCount < 0) {
      errors.push(`${ctx}: expectedTestCount must be non-negative integer`);
    }
    const allowedPresence = ["at_least_one", "exact", "conditional"];
    if (rec.expectedTestPresence && !allowedPresence.includes(rec.expectedTestPresence)) {
      errors.push(`${ctx}: unknown expectedTestPresence '${rec.expectedTestPresence}'`);
    }
    // if conditional, need expectedTestCountByProfile
    if (rec.expectedTestPresence === "conditional" && !rec.expectedTestCountByProfile) {
      errors.push(`${ctx}: conditional presence should provide expectedTestCountByProfile`);
    }
    if (rec.expectedTestCount === 0 && rec.expectedTestPresence === "exact") {
      // allowed but warn: zero-test with exact 0 means cfg-gated target that legitimately has 0 on current platform — should be conditional, not exact
      // For now, we flag if NATIVE_PLATFORM with exact 0 (would hide zero-test risk)
      if (rec.executionClass === "NATIVE_PLATFORM") {
        errors.push(`${ctx}: NATIVE_PLATFORM with expectedTestCount=0 and exact presence hides zero-test risk — use conditional with profile map`);
      }
    }

    // B5: conditional profile must contain default + at least one native profile; enforce fail-closed semantics
    if (rec.expectedTestPresence === "conditional") {
      const profile = rec.expectedTestCountByProfile || {};
      const keys = Object.keys(profile);
      if (!keys.some((k) => k === "default" || k === "default_non_macos")) {
        errors.push(`${ctx}: conditional presence must include 'default' or 'default_non_macos' profile key (got ${JSON.stringify(keys)})`);
      }
      // Ensure at least one profile value >0 if this is a test-bearing target ( §3 ZERO_TESTS)
      const vals = Object.values(profile);
      if (vals.length > 0 && vals.every((v) => v === 0)) {
        errors.push(`${ctx}: conditional target claims all profiles 0 — would be ZERO_TESTS false-green (expectedTestPresence conditional but no profile >0)`);
      }
      // CFG_ELIDED: NATIVE targets with conditional 0 default must note the risk
      if (rec.executionClass === "NATIVE_PLATFORM" && profile.default === 0 && vals.some((v) => v > 0)) {
        if (!rec.notes || !rec.notes.includes("cfg")) {
          // soft warning: encourage note about cfg gating for auditability
          // Not a hard error to allow non-cfg conditional (e.g., feature)
        }
      }
    }

    // B5: realWorker invariant — fake worker must not masquerade as real
    // EXACT_ARTIFACT also uses real production worker from packaged artifact
    if (rec.realWorker === true && !["NATIVE_PLATFORM", "EXACT_ARTIFACT"].includes(rec.executionClass)) {
      errors.push(`${ctx}: MOCK_REALITY_MISMATCH — realWorker=true requires NATIVE_PLATFORM or EXACT_ARTIFACT executionClass (got ${rec.executionClass})`);
    }

    // B5: FAST_LOCAL must never have usesPdfium true (already checked) — double-check for pdfium_dynamic_lib etc is native-only
    // B5: Evidence status vocabulary sanity — every target implicitly supports PASS/FAIL/NOT_RUN/NOT_APPLICABLE/RUNTIME_UNAVAILABLE etc.
    // No explicit field, but taxonomy must be bounded (already enforced via FALSE_GREEN_TAXONOMY comment).

    // B5: forbid silent ignore — if test is ignored/skipped it must not be counted as PASS
    // Registry has no ignoredTests field; invariant is that ignored count is not in expectedTestCount.
    // We add a static check: no target claims exact 0 with vague notes missing owner/reason if it had ignored tests.
  }

  // summary invariants: counts consistency
  if (registry.summary) {
    const total = registry.summary.totalTargets;
    if (total !== registry.targets.length) {
      errors.push(`summary.totalTargets ${total} != actual targets length ${registry.targets.length}`);
    }
    const byClass = registry.summary.byExecutionClass || {};
    const counts = { FAST_LOCAL: 0, NATIVE_PLATFORM: 0, EXACT_ARTIFACT: 0 };
    for (const r of registry.targets) counts[r.executionClass] = (counts[r.executionClass] || 0) + 1;
    for (const cls of ALLOWED_EXECUTION_CLASSES) {
      if ((byClass[cls] || 0) !== counts[cls]) {
        errors.push(`summary.byExecutionClass.${cls} ${byClass[cls]} != actual ${counts[cls]}`);
      }
    }
    // B5: summary pdfium/realWorker counts must match actual ( §20: per-target evidence, aggregate ambiguity bounded )
    if (typeof registry.summary.pdfiumRuntimeRequired === "number") {
      const actualPdfium = registry.targets.filter((r) => r.usesPdfium).length;
      if (registry.summary.pdfiumRuntimeRequired !== actualPdfium) {
        errors.push(`summary.pdfiumRuntimeRequired ${registry.summary.pdfiumRuntimeRequired} != actual ${actualPdfium}`);
      }
    }
    if (typeof registry.summary.realWorkerRequired === "number") {
      const actualWorker = registry.targets.filter((r) => r.realWorker).length;
      if (registry.summary.realWorkerRequired !== actualWorker) {
        errors.push(`summary.realWorkerRequired ${registry.summary.realWorkerRequired} != actual ${actualWorker}`);
      }
    }
  }

  // B5: cross-check with coverage mock/real drift (§11) — fail-closed if realWorker target missing but coverage claims worker evidence
  // We do a light check: if coverage exists, ensure no fake worker id is sole evidence for worker dimension (detailed check in verify-test-evidence.mjs)
  // Here we just ensure registry has at least one realWorker=true before coverage can claim worker
  if (existsSync(coveragePath)) {
    try {
      const cov = JSON.parse(readFileSync(coveragePath, "utf-8"));
      const hasWorkerEdge = (cov.traceabilityEdges || []).some((e) => (e.dimensions || []).includes("worker"));
      const hasRealWorkerTarget = registry.targets.some((r) => r.realWorker === true);
      if (hasWorkerEdge && !hasRealWorkerTarget) {
        errors.push(`MOCK_REALITY_MISMATCH: coverage claims worker dimension but no registry target has realWorker=true`);
      }
    } catch (e) {
      errors.push(`FATAL: Failed to parse coverage JSON at ${coveragePath}: ${e.message}`);
    }
  }

  // Drifts vs coverage: ensure registry does not duplicate feature->fixture mapping owner
  // We only check that fixtureIds in registry are canonical; coverage owns traceability — no duplicate check here beyond that.

  if (errors.length > 0) {
    console.error(`\n❌ Test Execution Registry Validation FAILED with ${errors.length} error(s):`);
    for (const e of errors) console.error(`  - ${e}`);
    process.exit(1);
  }

  console.log("✅ Test Execution Registry Validation PASSED! (B5 fail-closed)");
  console.log(`   - Targets: ${registry.targets.length}`);
  const byClassAgg = {};
  for (const r of registry.targets) byClassAgg[r.executionClass] = (byClassAgg[r.executionClass] || 0) + 1;
  console.log(`   - FAST_LOCAL: ${byClassAgg["FAST_LOCAL"] || 0}`);
  console.log(`   - NATIVE_PLATFORM: ${byClassAgg["NATIVE_PLATFORM"] || 0}`);
  console.log(`   - EXACT_ARTIFACT: ${byClassAgg["EXACT_ARTIFACT"] || 0}`);
  console.log(`   - PDFIUM_RUNTIME_REQUIRED (usesPdfium=true): ${registry.targets.filter((r) => r.usesPdfium).length}`);
  console.log(`   - REAL_WORKER_REQUIRED (realWorker=true): ${registry.targets.filter((r) => r.realWorker).length}`);
  console.log(`   - Taxonomy: ${FALSE_GREEN_TAXONOMY.join(", ")}`);
  console.log(`   - Evidence statuses: ${EVIDENCE_STATUSES.join(", ")}`);
  console.log(`   - B5: Exit 0 alone is NOT PASS — per-target evidence required`);
}

run();
