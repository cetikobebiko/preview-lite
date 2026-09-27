#!/usr/bin/env node
// 11I-D4 — Windows Packaging Runtime Wiring, Resource Layout, and DLL Packaging Contract Validator
// Validates offline package/loader/manifest parity, targets (msi, nsis), Windows path safety,
// architecture bounds (x86_64 only, PE COFF), and fail-closed presence semantics.
// Zero network, dependency-free (node stdlib only).

import { readFileSync, existsSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const repoRoot = resolve(__dirname, "..");

const ERRORS = [];
const WARNINGS = [];

function error(msg) {
  ERRORS.push(msg);
}

function warn(msg) {
  WARNINGS.push(msg);
}

function parseJsonFile(relPath) {
  const fullPath = resolve(repoRoot, relPath);
  if (!existsSync(fullPath)) {
    error(`Required file missing: ${relPath}`);
    return null;
  }
  try {
    return JSON.parse(readFileSync(fullPath, "utf-8"));
  } catch (e) {
    error(`Failed to parse JSON file ${relPath}: ${e.message}`);
    return null;
  }
}

function readSourceFile(relPath) {
  const fullPath = resolve(repoRoot, relPath);
  if (!existsSync(fullPath)) {
    error(`Required source file missing: ${relPath}`);
    return "";
  }
  return readFileSync(fullPath, "utf-8");
}

console.log("=== 11I-D4 Windows Packaging & Resource Layout Contract Validator ===\n");

// ─── 1. Package Target Contract (§1, §2) ───
console.log("[1/9] Checking Windows package targets in Tauri configuration...");
const tauriConf = parseJsonFile("src-tauri/tauri.conf.json");
const tauriWindowsConf = parseJsonFile("src-tauri/tauri.windows.conf.json");

if (tauriConf && tauriConf.bundle) {
  const targets = tauriConf.bundle.targets || [];
  if (!targets.includes("msi")) {
    error("tauri.conf.json bundle.targets must include 'msi'");
  }
  if (!targets.includes("nsis")) {
    error("tauri.conf.json bundle.targets must include 'nsis'");
  }
  if (targets.includes("appx")) {
    error("tauri.conf.json must NOT declare 'appx' target (scope expansion / unverified)");
  }
} else {
  error("tauri.conf.json missing bundle configuration");
}

if (tauriWindowsConf && tauriWindowsConf.bundle) {
  const windowsTargets = tauriWindowsConf.bundle.targets;
  if (windowsTargets) {
    if (!windowsTargets.includes("msi")) {
      error("tauri.windows.conf.json bundle.targets must specify 'msi'");
    }
    if (!windowsTargets.includes("nsis")) {
      error("tauri.windows.conf.json bundle.targets must specify 'nsis'");
    }
    const unsupported = windowsTargets.filter(t => t !== "msi" && t !== "nsis");
    if (unsupported.length > 0) {
      error(`tauri.windows.conf.json targets contains unsupported targets: ${unsupported.join(", ")}`);
    }
  } else {
    error("tauri.windows.conf.json bundle.targets must be explicitly defined as ['msi', 'nsis']");
  }
} else {
  error("tauri.windows.conf.json missing or invalid");
}

// ─── 2. Resource Layout Contract (§3, §4, §11, §14) ───
console.log("[2/9] Checking resource bundling configuration in tauri.windows.conf.json...");
let windowsResources = [];
if (tauriWindowsConf && tauriWindowsConf.bundle) {
  windowsResources = tauriWindowsConf.bundle.resources || [];
  if (!Array.isArray(windowsResources)) {
    error("tauri.windows.conf.json bundle.resources must be an array of paths");
  } else {
    if (!windowsResources.includes("lib/pdfium.dll")) {
      error("tauri.windows.conf.json bundle.resources must include 'lib/pdfium.dll'");
    }
    if (windowsResources.includes("pdfium.dll")) {
      error("tauri.windows.conf.json bundle.resources contains root 'pdfium.dll' (must be placed under 'lib/pdfium.dll')");
    }
    // Ensure no cross-platform contamination in windows resources
    for (const res of windowsResources) {
      if (res.includes("dylib")) {
        error(`tauri.windows.conf.json contains macOS dylib resource: ${res}`);
      }
      if (res.includes("so")) {
        error(`tauri.windows.conf.json contains Linux shared library resource: ${res}`);
      }
      if (res.includes("..") || res.startsWith("/") || res.includes(":\\") || res.includes("\\")) {
        error(`tauri.windows.conf.json resource path contains traversal, backslash, or absolute path: ${res}`);
      }
    }
  }
}

// ─── 3. Canonical Manifest ↔ Package Parity (§4, §14) ───
console.log("[3/9] Checking canonical manifest (platform-support.json) ↔ package parity...");
const manifest = parseJsonFile("docs/engineering/platform-support.json");
let windowsPlatform = null;

if (manifest && Array.isArray(manifest.platforms)) {
  windowsPlatform = manifest.platforms.find(
    p => p.operatingSystem === "windows" || p.targetTriple === "x86_64-pc-windows-msvc"
  );
  if (!windowsPlatform) {
    error("platform-support.json does not contain a platform record for Windows x86_64");
  } else {
    const r = windowsPlatform.runtime;
    if (!r) {
      error("Windows platform in platform-support.json is missing runtime record");
    } else {
      if (r.runtimeFileName !== "pdfium.dll") {
        error(`Manifest runtimeFileName '${r.runtimeFileName}' does not match expected 'pdfium.dll'`);
      }
      if (r.expectedStagedPath !== "src-tauri/lib/pdfium.dll") {
        error(`Manifest expectedStagedPath '${r.expectedStagedPath}' does not match 'src-tauri/lib/pdfium.dll'`);
      }
      if (r.expectedPackagedLocation !== "resourceDir/lib/pdfium.dll") {
        error(`Manifest expectedPackagedLocation '${r.expectedPackagedLocation}' does not match 'resourceDir/lib/pdfium.dll'`);
      }
      if (r.distributionClass !== "bundled_with_artifact") {
        error(`Manifest distributionClass is '${r.distributionClass}', expected 'bundled_with_artifact'`);
      }
      if (r.architecture !== "x86_64") {
        error(`Manifest architecture is '${r.architecture}', expected 'x86_64'`);
      }

      // Parity check: staged path in repo vs package resource entry
      // "src-tauri/lib/pdfium.dll" relative to src-tauri is "lib/pdfium.dll"
      const expectedInConfig = r.expectedStagedPath.replace(/^src-tauri\//, "");
      if (!windowsResources.includes(expectedInConfig)) {
        error(`Package config tauri.windows.conf.json missing resource '${expectedInConfig}' derived from manifest expectedStagedPath '${r.expectedStagedPath}'`);
      }

      // Parity check: packaged relative path
      // "resourceDir/lib/pdfium.dll" maps to resourceDir + expectedInConfig
      const expectedPackaged = `resourceDir/${expectedInConfig}`;
      if (r.expectedPackagedLocation !== expectedPackaged) {
        error(`Manifest expectedPackagedLocation '${r.expectedPackagedLocation}' does not align with packaged resource '${expectedPackaged}'`);
      }
    }
  }
}

// ─── 4. Loader ↔ Package Path Parity (§3, §11, §12) ───
console.log("[4/9] Checking loader (resolve_windows_runtime_path) ↔ package layout parity...");
const modRs = readSourceFile("src-tauri/src/pdf_engine/mod.rs");

if (!modRs.includes("resolve_windows_runtime_path")) {
  error("pdf_engine/mod.rs does not define resolve_windows_runtime_path()");
} else {
  // Verify loader searches TAURI_RESOURCE_DIR / "lib" / RUNTIME_FILENAME
  const hasResourceDirCheck = modRs.includes('std::env::var("TAURI_RESOURCE_DIR")');
  const hasLibSubdir = modRs.includes('.join("lib")');
  const hasRuntimeFilename = modRs.includes('.join(RUNTIME_FILENAME)');

  if (!hasResourceDirCheck) {
    error("resolve_windows_runtime_path() does not check TAURI_RESOURCE_DIR");
  }
  if (!hasLibSubdir || !hasRuntimeFilename) {
    error("resolve_windows_runtime_path() does not construct path with .join(\"lib\").join(RUNTIME_FILENAME)");
  }

  // Loader fail-closed invariant
  if (!modRs.includes("No CWD or PATH fallback permitted") && !modRs.includes("No CWD")) {
    warn("resolve_windows_runtime_path() error message should emphasize no CWD or PATH fallback");
  }
}

// ─── 5. Worker Runtime Resolution Parity (§13) ───
console.log("[5/9] Checking worker runtime resolution & path propagation parity...");
const subprocessRs = readSourceFile("src-tauri/src/pdf_engine/subprocess_manager.rs");
const workerRs = readSourceFile("src-tauri/src/worker/pdfium.rs");

if (!subprocessRs.includes('command.env("PDFIUM_RESOLVED_PATH"')) {
  error("subprocess_manager.rs does not inject PDFIUM_RESOLVED_PATH into worker child environment");
}
if (!workerRs.includes('std::env::var_os("PDFIUM_RESOLVED_PATH")')) {
  error("worker/pdfium.rs does not read PDFIUM_RESOLVED_PATH from parent environment");
}
if (!workerRs.includes("get_pdfium_with_resolved_path(resolved_runtime_path)")) {
  error("worker/pdfium.rs does not pass resolved path to get_pdfium_with_resolved_path()");
}

// ─── 6. Architecture Isolation & Non-Contamination Gate (§5, §9) ───
console.log("[6/9] Checking architecture isolation (x86_64 only, reject 32-bit & ARM64)...");
const buildRs = readSourceFile("src-tauri/build.rs");

if (!modRs.includes("IMAGE_FILE_MACHINE_AMD64") || !modRs.includes("0x8664")) {
  error("pdf_engine/mod.rs verify_pe_header_bytes must validate IMAGE_FILE_MACHINE_AMD64 (0x8664)");
}
if (!modRs.includes("IMAGE_FILE_32BIT_MACHINE") || !modRs.includes("0x0020")) {
  error("pdf_engine/mod.rs verify_pe_header_bytes must reject 32-bit PE flag (IMAGE_FILE_32BIT_MACHINE / 0x0020)");
}
if (!buildRs.includes("verify_windows_runtime_pre_package")) {
  error("build.rs must define and invoke verify_windows_runtime_pre_package()");
}
if (!buildRs.includes("IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;")) {
  error("build.rs verify_windows_dll_architecture must check IMAGE_FILE_MACHINE_AMD64 (0x8664)");
}
if (!buildRs.includes("IMAGE_FILE_32BIT_MACHINE: u16 = 0x0020;")) {
  error("build.rs verify_windows_dll_architecture must reject IMAGE_FILE_32BIT_MACHINE (0x0020)");
}

// ─── 7. Binary Presence & Pre-Package Gate Semantics (§6, §7, §8, §15, §16) ───
console.log("[7/9] Checking pre-package gate & fail-closed presence semantics...");
if (!buildRs.includes("RELEASE_ELIGIBILITY_GATE_FAILED")) {
  error("build.rs must enforce RELEASE_ELIGIBILITY_GATE_FAILED when supplyChainStatus != 'COMPLETE'");
}
if (!buildRs.includes("MISSING_RUNTIME_ARTIFACT")) {
  error("build.rs must enforce MISSING_RUNTIME_ARTIFACT when staged DLL does not exist");
}
if (!buildRs.includes("INTEGRITY_MISMATCH")) {
  error("build.rs must enforce INTEGRITY_MISMATCH on hash or byte-length mismatch");
}
if (!buildRs.includes("ARCHITECTURE_MISMATCH")) {
  error("build.rs must enforce ARCHITECTURE_MISMATCH on machine type or 32-bit flag mismatch");
}

// Check repository staging presence status (strict offline reporting)
const stagedBinaryPath = resolve(repoRoot, "src-tauri/lib/pdfium.dll");
const stagedBinaryExists = existsSync(stagedBinaryPath);
console.log(`  -> Staged binary exists locally: ${stagedBinaryExists ? "YES" : "NO (RUNTIME_BYTES_UNAVAILABLE_OFFLINE)"}`);

if (!stagedBinaryExists) {
  // Confirm that platform-support.json correctly flags present: false
  if (windowsPlatform && windowsPlatform.runtime && windowsPlatform.runtime.present !== false) {
    error("platform-support.json must have runtime.present: false when src-tauri/lib/pdfium.dll is missing");
  }
}

// ─── 8. Windows Path Safety & Cleanliness (§10, §23, §24) ───
console.log("[8/9] Checking Windows path security & cleanliness...");
const developerPathRe = /[A-Za-z]:\\Users\\[^\\\s]+\\/;
if (developerPathRe.test(readSourceFile("src-tauri/tauri.windows.conf.json"))) {
  error("tauri.windows.conf.json contains developer machine absolute path");
}
if (developerPathRe.test(readSourceFile("docs/engineering/platform-support.json"))) {
  error("platform-support.json contains developer machine absolute path");
}
// Ensure no maintainer script hooks in tauri.windows.conf.json
if (tauriWindowsConf && tauriWindowsConf.bundle && tauriWindowsConf.bundle.windows) {
  const winBundle = tauriWindowsConf.bundle.windows;
  if (winBundle.wix && winBundle.wix.hooks) {
    error("tauri.windows.conf.json contains custom WiX hooks (§10 forbids unverified execution scripts)");
  }
  if (winBundle.nsis && (winBundle.nsis.customTemplate || winBundle.nsis.installerHooks)) {
    error("tauri.windows.conf.json contains custom NSIS hooks/templates");
  }
}

// ─── 9. Exact Smoke Test Contract Parity (§19, 11V-E handoff) ───
console.log("[9/9] Checking 11V-E handoff & artifact smoke contract alignment...");
const handoffJson = parseJsonFile("docs/engineering/11v-e-handoff.json");
if (handoffJson && Array.isArray(handoffJson.matrix)) {
  for (const artifactClass of ["WINDOWS_MSI", "WINDOWS_NSIS"]) {
    const row = handoffJson.matrix.find(m => m.artifactClass === artifactClass);
    if (!row) {
      error(`11v-e-handoff.json matrix missing ${artifactClass} row`);
    } else {
      if (row.runnerOs !== "windows") error(`11v-e-handoff.json ${artifactClass} runnerOs must be 'windows'`);
      if (row.architecture !== "x86_64") error(`11v-e-handoff.json ${artifactClass} architecture must be 'x86_64'`);
      if (row.targetTriple !== "x86_64-pc-windows-msvc") error(`11v-e-handoff.json ${artifactClass} targetTriple mismatch`);
      if (row.runtimeExpectation.relativePath !== "resourceDir/lib/pdfium.dll") {
        error(`11v-e-handoff.json ${artifactClass} runtimeExpectation '${row.runtimeExpectation.relativePath}' != 'resourceDir/lib/pdfium.dll'`);
      }
    }
  }
}

const smokeScript = readSourceFile("scripts/artifact-smoke-windows.mjs");
if (!smokeScript.includes('join(resourceLib, runtimeMeta.runtimeFilename)') && !smokeScript.includes('resourceLib')) {
  error("scripts/artifact-smoke-windows.mjs must resolve runtime via resourceLib / runtimeFilename");
}
if (!smokeScript.includes('expectedPackagedLocation: "resourceDir/lib/pdfium.dll"')) {
  error("scripts/artifact-smoke-windows.mjs must specify expectedPackagedLocation 'resourceDir/lib/pdfium.dll'");
}

// ─── Summary & Exit ───
console.log("\n============================================================");
console.log("Validator Summary:");
console.log(`Errors:   ${ERRORS.length}`);
console.log(`Warnings: ${WARNINGS.length}`);

if (WARNINGS.length > 0) {
  console.log("\nWarnings:");
  for (const w of WARNINGS) console.log(`  [WARN] ${w}`);
}

if (ERRORS.length > 0) {
  console.log("\nFailures:");
  for (const e of ERRORS) console.log(`  [FAIL] ${e}`);
  console.log("\n❌ 11I-D4 Windows Packaging & Resource Layout Contract Validation FAILED");
  process.exit(1);
}

console.log("\n✅ 11I-D4 Windows Packaging & Resource Layout Contract Validation PASSED");
console.log("  - Supported package targets: [msi, nsis] strictly configured");
console.log("  - Resource layout: resourceDir/lib/pdfium.dll");
console.log("  - Package ↔ Loader path parity: PASS");
console.log("  - Manifest ↔ Package parity: PASS");
console.log("  - Worker resolution parity: PASS");
console.log("  - Architecture mapping: x86_64 strict (IMAGE_FILE_MACHINE_AMD64 0x8664 / 64-bit)");
console.log("  - Pre-package gate: FAIL-CLOSED (RELEASE_ELIGIBILITY_GATE_FAILED / MISSING_RUNTIME_ARTIFACT)");
console.log("  - Offline runtime status: RUNTIME_BYTES_UNAVAILABLE_OFFLINE");
console.log("  - Exact artifact verification: DEFERRED to 11V-E");
process.exit(0);
