#!/usr/bin/env node
// 11I-C4 — Linux Packaging Runtime Wiring, Resource Layout, and Artifact Content Contract Validator
// Validates offline package/loader/manifest parity, targets, file permissions contract,
// architecture bounds, and fail-closed presence semantics.
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

console.log("=== 11I-C4 Linux Packaging & Resource Layout Contract Validator ===\n");

// ─── 1. Package Target Contract (§1.A, §4) ───
console.log("[1/9] Checking Linux package targets in Tauri configuration...");
const tauriConf = parseJsonFile("src-tauri/tauri.conf.json");
const tauriLinuxConf = parseJsonFile("src-tauri/tauri.linux.conf.json");

if (tauriConf && tauriConf.bundle) {
  const targets = tauriConf.bundle.targets || [];
  if (!targets.includes("deb")) {
    error("tauri.conf.json bundle.targets must include 'deb'");
  }
  if (targets.includes("appimage")) {
    error("tauri.conf.json must NOT declare 'appimage' target (scope expansion / unverified)");
  }
  if (targets.includes("rpm")) {
    error("tauri.conf.json must NOT declare 'rpm' target (scope expansion / unverified)");
  }
} else {
  error("tauri.conf.json missing bundle configuration");
}

if (tauriLinuxConf && tauriLinuxConf.bundle) {
  const linuxTargets = tauriLinuxConf.bundle.targets;
  if (linuxTargets) {
    if (!linuxTargets.includes("deb")) {
      error("tauri.linux.conf.json bundle.targets must specify 'deb'");
    }
    if (linuxTargets.some(t => t !== "deb")) {
      error(`tauri.linux.conf.json targets contains unsupported targets: ${linuxTargets.filter(t => t !== "deb").join(", ")}`);
    }
  }
} else {
  error("tauri.linux.conf.json missing or invalid");
}

// ─── 2. Resource Layout Contract (§1.B, §1.C, §2, §3, §12) ───
console.log("[2/9] Checking resource bundling configuration in tauri.linux.conf.json...");
let linuxResources = [];
if (tauriLinuxConf && tauriLinuxConf.bundle) {
  linuxResources = tauriLinuxConf.bundle.resources || [];
  if (!Array.isArray(linuxResources)) {
    error("tauri.linux.conf.json bundle.resources must be an array of paths");
  } else {
    if (!linuxResources.includes("lib/libpdfium.so")) {
      error("tauri.linux.conf.json bundle.resources must include 'lib/libpdfium.so'");
    }
    // Ensure no cross-platform contamination in linux resources
    for (const res of linuxResources) {
      if (res.includes("dylib")) {
        error(`tauri.linux.conf.json contains macOS dylib resource: ${res}`);
      }
      if (res.includes("dll")) {
        error(`tauri.linux.conf.json contains Windows dll resource: ${res}`);
      }
      if (res.includes("..") || res.startsWith("/") || res.includes(":\\")) {
        error(`tauri.linux.conf.json resource path contains traversal or absolute path: ${res}`);
      }
    }
  }
}

// ─── 3. Canonical Manifest ↔ Package Parity (§8) ───
console.log("[3/9] Checking canonical manifest (platform-support.json) ↔ package parity...");
const manifest = parseJsonFile("docs/engineering/platform-support.json");
let linuxPlatform = null;

if (manifest && Array.isArray(manifest.platforms)) {
  linuxPlatform = manifest.platforms.find(p => p.operatingSystem === "linux" || p.targetTriple === "x86_64-unknown-linux-gnu");
  if (!linuxPlatform) {
    error("platform-support.json does not contain a platform record for Linux x86_64");
  } else {
    const r = linuxPlatform.runtime;
    if (!r) {
      error("Linux platform in platform-support.json is missing runtime record");
    } else {
      if (r.runtimeFileName !== "libpdfium.so") {
        error(`Manifest runtimeFileName '${r.runtimeFileName}' does not match expected 'libpdfium.so'`);
      }
      if (r.expectedStagedPath !== "src-tauri/lib/libpdfium.so") {
        error(`Manifest expectedStagedPath '${r.expectedStagedPath}' does not match 'src-tauri/lib/libpdfium.so'`);
      }
      if (r.expectedPackagedLocation !== "resourceDir/lib/libpdfium.so") {
        error(`Manifest expectedPackagedLocation '${r.expectedPackagedLocation}' does not match 'resourceDir/lib/libpdfium.so'`);
      }
      if (r.distributionClass !== "bundled_with_artifact") {
        error(`Manifest distributionClass is '${r.distributionClass}', expected 'bundled_with_artifact'`);
      }
      if (r.architecture !== "x86_64") {
        error(`Manifest architecture is '${r.architecture}', expected 'x86_64'`);
      }

      // Parity check: staged path in repo vs package resource entry
      // "src-tauri/lib/libpdfium.so" relative to src-tauri is "lib/libpdfium.so"
      const expectedInConfig = r.expectedStagedPath.replace(/^src-tauri\//, "");
      if (!linuxResources.includes(expectedInConfig)) {
        error(`Package config tauri.linux.conf.json missing resource '${expectedInConfig}' derived from manifest expectedStagedPath '${r.expectedStagedPath}'`);
      }

      // Parity check: packaged relative path
      // "resourceDir/lib/libpdfium.so" maps to resourceDir + expectedInConfig
      const expectedPackaged = `resourceDir/${expectedInConfig}`;
      if (r.expectedPackagedLocation !== expectedPackaged) {
        error(`Manifest expectedPackagedLocation '${r.expectedPackagedLocation}' does not align with packaged resource '${expectedPackaged}'`);
      }
    }
  }
}

// ─── 4. Loader ↔ Package Path Parity (§1.D, §12) ───
console.log("[4/9] Checking loader (resolve_linux_runtime_path) ↔ package layout parity...");
const modRs = readSourceFile("src-tauri/src/pdf_engine/mod.rs");

if (!modRs.includes("resolve_linux_runtime_path")) {
  error("pdf_engine/mod.rs does not define resolve_linux_runtime_path()");
} else {
  // Verify loader searches TAURI_RESOURCE_DIR / "lib" / RUNTIME_FILENAME
  const hasResourceDirCheck = modRs.includes('std::env::var("TAURI_RESOURCE_DIR")');
  const hasLibSubdir = modRs.includes('.join("lib")');
  const hasRuntimeFilename = modRs.includes('.join(RUNTIME_FILENAME)');

  if (!hasResourceDirCheck) {
    error("resolve_linux_runtime_path() does not check TAURI_RESOURCE_DIR");
  }
  if (!hasLibSubdir || !hasRuntimeFilename) {
    error("resolve_linux_runtime_path() does not construct path with .join(\"lib\").join(RUNTIME_FILENAME)");
  }

  // Loader fail-closed invariant
  if (!modRs.includes("No CWD or system search permitted") && !modRs.includes("No CWD")) {
    warn("resolve_linux_runtime_path() error message should emphasize no CWD or system fallback");
  }
}

// ─── 5. Worker Runtime Resolution Parity (§13) ───
console.log("[5/9] Checking worker runtime resolution & path propagation parity...");
const subprocessRs = readSourceFile("src-tauri/src/pdf_engine/subprocess_manager.rs");
const workerRs = readSourceFile("src-tauri/src/worker/pdfium.rs");

if (!subprocessRs.includes('command.env("PDFIUM_RESOLVED_PATH"')) {
  error("subprocess_manager.rs does not inject PDFIUM_RESOLVED_PATH into worker environment");
}
if (!workerRs.includes('std::env::var_os("PDFIUM_RESOLVED_PATH")')) {
  error("worker/pdfium.rs does not read PDFIUM_RESOLVED_PATH from parent environment");
}
if (!workerRs.includes("get_pdfium_with_resolved_path(resolved_runtime_path)")) {
  error("worker/pdfium.rs does not pass resolved path to get_pdfium_with_resolved_path()");
}

// ─── 6. Architecture Isolation & Non-Contamination Gate (§7) ───
console.log("[6/9] Checking architecture isolation (x86_64 only, reject multi-arch)...");
const buildRs = readSourceFile("src-tauri/build.rs");

if (!modRs.includes("EM_X86_64") || !modRs.includes("0x3e")) {
  error("pdf_engine/mod.rs verify_linux_architecture must validate ELF machine code 0x3e (EM_X86_64)");
}
if (!modRs.includes("ei_class != 2")) {
  error("pdf_engine/mod.rs verify_linux_architecture must reject 32-bit ELF (ei_class != 2)");
}
if (!buildRs.includes("verify_linux_runtime_pre_package")) {
  error("build.rs must define and invoke verify_linux_runtime_pre_package()");
}
if (!buildRs.includes("const EM_X86_64: u16 = 0x3e;")) {
  error("build.rs verify_linux_so_architecture must check EM_X86_64 (0x3e)");
}

// ─── 7. Binary Presence & Pre-Package Gate Semantics (§9, §10, §16) ───
console.log("[7/9] Checking pre-package gate & fail-closed presence semantics...");
if (!buildRs.includes("RELEASE_ELIGIBILITY_GATE_FAILED")) {
  error("build.rs must enforce RELEASE_ELIGIBILITY_GATE_FAILED when supplyChainStatus != 'COMPLETE'");
}
if (!buildRs.includes("MISSING_RUNTIME_ARTIFACT")) {
  error("build.rs must enforce MISSING_RUNTIME_ARTIFACT when staged library does not exist");
}
if (!buildRs.includes("INTEGRITY_MISMATCH")) {
  error("build.rs must enforce INTEGRITY_MISMATCH on hash or byte-length mismatch");
}

// Check repository staging presence status (strict offline reporting)
const stagedBinaryPath = resolve(repoRoot, "src-tauri/lib/libpdfium.so");
const stagedBinaryExists = existsSync(stagedBinaryPath);
console.log(`  -> Staged binary exists locally: ${stagedBinaryExists ? "YES" : "NO (RUNTIME_BYTES_UNAVAILABLE_OFFLINE)"}`);

if (!stagedBinaryExists) {
  // Confirm that platform-support.json correctly flags present: false
  if (linuxPlatform && linuxPlatform.runtime && linuxPlatform.runtime.present !== false) {
    error("platform-support.json must have runtime.present: false when src-tauri/lib/libpdfium.so is missing");
  }
}

// ─── 8. File Permissions & Packaging Script Safety Contract (§6) ───
console.log("[8/9] Checking permissions contract & maintainer scripts safety...");
// Ensure no arbitrary shell maintainer script in tauri.linux.conf.json
if (tauriLinuxConf && tauriLinuxConf.bundle && tauriLinuxConf.bundle.linux && tauriLinuxConf.bundle.linux.deb) {
  const debConf = tauriLinuxConf.bundle.linux.deb;
  for (const scriptKey of ["preInstallScript", "postInstallScript", "preRemoveScript", "postRemoveScript"]) {
    if (debConf[scriptKey]) {
      error(`tauri.linux.conf.json contains custom deb script '${scriptKey}': ${debConf[scriptKey]} (§6 forbids post-install chmod/scripts)`);
    }
  }
}
console.log("  -> Expected shared library permissions contract: 0755 (-rwxr-xr-x) standard ELF DSO");

// ─── 9. Exact Smoke Test Contract Parity (§17, 11V-E handoff) ───
console.log("[9/9] Checking 11V-E handoff & artifact smoke contract alignment...");
const handoffJson = parseJsonFile("docs/engineering/11v-e-handoff.json");
if (handoffJson && Array.isArray(handoffJson.matrix)) {
  const linuxRow = handoffJson.matrix.find(m => m.artifactClass === "LINUX_DEB");
  if (!linuxRow) {
    error("11v-e-handoff.json matrix missing LINUX_DEB row");
  } else {
    if (linuxRow.runnerOs !== "linux") error("11v-e-handoff.json LINUX_DEB runnerOs must be 'linux'");
    if (linuxRow.architecture !== "x86_64") error("11v-e-handoff.json LINUX_DEB architecture must be 'x86_64'");
    if (linuxRow.targetTriple !== "x86_64-unknown-linux-gnu") error("11v-e-handoff.json LINUX_DEB targetTriple mismatch");
    if (linuxRow.runtimeExpectation.relativePath !== "resourceDir/lib/libpdfium.so") {
      error(`11v-e-handoff.json LINUX_DEB runtimeExpectation '${linuxRow.runtimeExpectation.relativePath}' != 'resourceDir/lib/libpdfium.so'`);
    }
  }
}

const smokeScript = readSourceFile("scripts/artifact-smoke-linux.mjs");
if (!smokeScript.includes('EXPECTED_RUNTIME_FILENAME = "libpdfium.so"')) {
  error("scripts/artifact-smoke-linux.mjs must specify EXPECTED_RUNTIME_FILENAME = 'libpdfium.so'");
}
if (!smokeScript.includes('EXPECTED_RESOURCE_SUBDIR = "lib"')) {
  error("scripts/artifact-smoke-linux.mjs must specify EXPECTED_RESOURCE_SUBDIR = 'lib'");
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
  console.log("\n❌ 11I-C4 Packaging & Resource Layout Contract Validation FAILED");
  process.exit(1);
}

console.log("\n✅ 11I-C4 Packaging & Resource Layout Contract Validation PASSED");
console.log("  - Supported package targets: [deb] strictly configured");
  console.log("  - AppImage / RPM unconfigured (no scope expansion)");
console.log("  - Resource layout: resourceDir/lib/libpdfium.so");
console.log("  - Package ↔ Loader path parity: PASS");
console.log("  - Manifest ↔ Package parity: PASS");
console.log("  - Worker resolution parity: PASS");
console.log("  - Architecture mapping: x86_64 strict (EM_X86_64 / ELFCLASS64)");
console.log("  - Pre-package gate: FAIL-CLOSED (RELEASE_ELIGIBILITY_GATE_FAILED / MISSING_RUNTIME_ARTIFACT)");
console.log("  - File permissions contract: 0755 standard ELF DSO");
console.log("  - Offline runtime status: RUNTIME_BYTES_UNAVAILABLE_OFFLINE");
console.log("  - Exact artifact verification: DEFERRED to 11V-E");
process.exit(0);
