#!/usr/bin/env node
// 11I-E2 — macOS Packaging Runtime Wiring, Resource Layout, and Bundle Contract Validator
// Validates offline package/loader/manifest parity, targets (dmg, app), macOS path safety,
// architecture bounds (arm64 only, Mach-O 64-bit), and fail-closed presence semantics.
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

console.log("=== 11I-E2 macOS Packaging & Resource Layout Contract Validator ===\n");

// ─── 1. Package Target Contract (§1, §2) ───
console.log("[1/9] Checking macOS package targets in Tauri configuration...");
const tauriConf = parseJsonFile("src-tauri/tauri.conf.json");
const tauriMacosConf = parseJsonFile("src-tauri/tauri.macos.conf.json");

if (tauriConf && tauriConf.bundle) {
  const targets = tauriConf.bundle.targets || [];
  if (!targets.includes("dmg")) {
    error("tauri.conf.json bundle.targets must include 'dmg'");
  }
  if (!targets.includes("app")) {
    error("tauri.conf.json bundle.targets must include 'app'");
  }
} else {
  error("tauri.conf.json missing bundle configuration");
}

if (tauriMacosConf && tauriMacosConf.bundle) {
  const macosTargets = tauriMacosConf.bundle.targets;
  if (macosTargets) {
    if (!macosTargets.includes("dmg")) {
      error("tauri.macos.conf.json bundle.targets must specify 'dmg'");
    }
    if (!macosTargets.includes("app")) {
      error("tauri.macos.conf.json bundle.targets must specify 'app'");
    }
    const unsupported = macosTargets.filter(t => t !== "dmg" && t !== "app");
    if (unsupported.length > 0) {
      error(`tauri.macos.conf.json targets contains unsupported targets: ${unsupported.join(", ")}`);
    }
  } else {
    error("tauri.macos.conf.json bundle.targets must be explicitly defined as ['dmg', 'app']");
  }
} else {
  error("tauri.macos.conf.json missing or invalid");
}

// ─── 2. Resource Layout Contract (§3, §4) ───
console.log("[2/9] Checking resource bundling configuration in tauri.macos.conf.json...");
let macosResources = [];
if (tauriMacosConf && tauriMacosConf.bundle) {
  macosResources = tauriMacosConf.bundle.resources || [];
  if (!Array.isArray(macosResources)) {
    error("tauri.macos.conf.json bundle.resources must be an array of paths");
  } else {
    if (!macosResources.includes("lib/libpdfium.dylib")) {
      error("tauri.macos.conf.json bundle.resources must include 'lib/libpdfium.dylib'");
    }
    if (macosResources.includes("libpdfium.dylib")) {
      error("tauri.macos.conf.json bundle.resources contains root 'libpdfium.dylib' (must be placed under 'lib/libpdfium.dylib')");
    }
    // Ensure no cross-platform contamination in macos resources
    for (const res of macosResources) {
      if (res.includes("so")) {
        error(`tauri.macos.conf.json contains Linux shared library resource: ${res}`);
      }
      if (res.includes("dll")) {
        error(`tauri.macos.conf.json contains Windows dll resource: ${res}`);
      }
      if (res.includes("..") || res.startsWith("/") || res.includes(":\\") || res.includes("\\")) {
        error(`tauri.macos.conf.json resource path contains traversal, backslash, or absolute path: ${res}`);
      }
    }
  }
}

// ─── 3. Canonical Manifest ↔ Package Parity ───
console.log("[3/9] Checking canonical manifest (platform-support.json) ↔ package parity...");
const manifest = parseJsonFile("docs/engineering/platform-support.json");
let macosPlatform = null;

if (manifest && Array.isArray(manifest.platforms)) {
  macosPlatform = manifest.platforms.find(
    p => p.operatingSystem === "macos" || p.targetTriple === "aarch64-apple-darwin"
  );
  if (!macosPlatform) {
    error("platform-support.json does not contain a platform record for macOS arm64");
  } else {
    const r = macosPlatform.runtime;
    if (!r) {
      error("macOS platform in platform-support.json is missing runtime record");
    } else {
      if (r.runtimeFileName !== "libpdfium.dylib") {
        error(`Manifest runtimeFileName '${r.runtimeFileName}' does not match expected 'libpdfium.dylib'`);
      }
      if (r.expectedStagedPath !== "src-tauri/lib/libpdfium.dylib") {
        error(`Manifest expectedStagedPath '${r.expectedStagedPath}' does not match 'src-tauri/lib/libpdfium.dylib'`);
      }
      if (r.expectedPackagedLocation !== "resourceDir/lib/libpdfium.dylib") {
        error(`Manifest expectedPackagedLocation '${r.expectedPackagedLocation}' does not match 'resourceDir/lib/libpdfium.dylib'`);
      }
      if (r.distributionClass !== "bundled_with_artifact") {
        error(`Manifest distributionClass is '${r.distributionClass}', expected 'bundled_with_artifact'`);
      }
      if (r.architecture !== "arm64") {
        error(`Manifest architecture is '${r.architecture}', expected 'arm64'`);
      }

      // Parity check: staged path in repo vs package resource entry
      const expectedInConfig = r.expectedStagedPath.replace(/^src-tauri\//, "");
      if (!macosResources.includes(expectedInConfig)) {
        error(`Package config tauri.macos.conf.json missing resource '${expectedInConfig}' derived from manifest expectedStagedPath '${r.expectedStagedPath}'`);
      }

      // Parity check: packaged relative path
      const expectedPackaged = `resourceDir/${expectedInConfig}`;
      if (r.expectedPackagedLocation !== expectedPackaged) {
        error(`Manifest expectedPackagedLocation '${r.expectedPackagedLocation}' does not align with packaged resource '${expectedPackaged}'`);
      }
    }
  }
}

// ─── 4. Loader ↔ Package Path Parity ───
console.log("[4/9] Checking loader (resolve_macos_runtime_path) ↔ package layout parity...");
const modRs = readSourceFile("src-tauri/src/pdf_engine/mod.rs");

if (!modRs.includes("resolve_macos_runtime_path")) {
  error("pdf_engine/mod.rs does not define resolve_macos_runtime_path()");
} else {
  const hasResourceDirCheck = modRs.includes('std::env::var("TAURI_RESOURCE_DIR")');
  const hasLibSubdir = modRs.includes('.join("lib")');
  const hasRuntimeFilename = modRs.includes('.join(RUNTIME_FILENAME)');

  if (!hasResourceDirCheck) {
    error("resolve_macos_runtime_path() does not check TAURI_RESOURCE_DIR");
  }
  if (!hasLibSubdir || !hasRuntimeFilename) {
    error("resolve_macos_runtime_path() does not construct path with .join(\"lib\").join(RUNTIME_FILENAME)");
  }
}

// ─── 5. Worker Runtime Resolution Parity ───
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

// ─── 6. Architecture Isolation (Mach-O 64-bit arm64 only) ───
console.log("[6/9] Checking architecture isolation (Mach-O 64-bit arm64)...");
const stagedBinaryPath = resolve(repoRoot, "src-tauri/lib/libpdfium.dylib");
const stagedBinaryExists = existsSync(stagedBinaryPath);
console.log(`  -> Staged binary exists locally: ${stagedBinaryExists ? "YES" : "NO"}`);

if (stagedBinaryExists) {
  const buf = readFileSync(stagedBinaryPath);
  if (buf.length < 8) {
    error("Staged libpdfium.dylib is too short to be a valid Mach-O binary");
  } else {
    const magic = buf.readUInt32LE(0);
    const cpuType = buf.readInt32LE(4);
    const MH_MAGIC_64 = 0xfeedfacf;
    const CPU_TYPE_ARM64 = 0x0100000c;

    if (magic !== MH_MAGIC_64) {
      error(`Staged libpdfium.dylib magic mismatch: expected 0xfeedfacf, got 0x${magic.toString(16)}`);
    }
    if (cpuType !== CPU_TYPE_ARM64) {
      error(`Staged libpdfium.dylib CPU type mismatch: expected 0x0100000c (ARM64), got 0x${cpuType.toString(16)}`);
    }

    if (buf.length !== 7151568) {
      error(`Staged libpdfium.dylib byte length mismatch: expected 7151568, got ${buf.length}`);
    }
  }
} else {
  error("src-tauri/lib/libpdfium.dylib missing from repository");
}

// ─── 7. Path Safety Contract ───
console.log("[7/9] Checking path safety (no traversal, no absolute paths in package configs)...");
for (const res of macosResources) {
  if (res.includes("..") || res.startsWith("/") || res.includes("\\")) {
    error(`Resource path '${res}' violates safety rules (must be relative, forward slashes, no traversal)`);
  }
}

// ─── 8. Smoke Hook & Entry Point Parity ───
console.log("[8/9] Checking smoke hook entry points in main.rs and lib.rs...");
const mainRs = readSourceFile("src-tauri/src/main.rs");
const libRs = readSourceFile("src-tauri/src/lib.rs");

if (!mainRs.includes('--smoke') || !mainRs.includes('run_smoke()')) {
  error("main.rs must handle '--smoke' CLI argument and invoke run_smoke()");
}
if (!mainRs.includes('--run-worker') || !mainRs.includes('run_worker()')) {
  error("main.rs must handle '--run-worker' CLI argument and invoke run_worker()");
}
if (!libRs.includes('pub fn run_smoke()')) {
  error("lib.rs must export pub fn run_smoke()");
}

// ─── 9. Strict Offline Boundary ───
console.log("[9/9] Checking strict offline boundary...");
const forbiddenTools = ["curl", "wget", "git clone", "npm install", "cargo update"];
for (const tool of forbiddenTools) {
  if (tauriMacosConf && JSON.stringify(tauriMacosConf).includes(tool)) {
    error(`tauri.macos.conf.json contains forbidden network command string: '${tool}'`);
  }
}

console.log("\n---------------------------------------------------");
if (WARNINGS.length > 0) {
  console.log(`Warnings (${WARNINGS.length}):`);
  for (const w of WARNINGS) {
    console.log(`  [WARN] ${w}`);
  }
}

if (ERRORS.length > 0) {
  console.error(`Validation FAILED with ${ERRORS.length} error(s):`);
  for (const e of ERRORS) {
    console.error(`  [FAIL] ${e}`);
  }
  process.exit(1);
} else {
  console.log("Validation PASSED (macOS packaging contract verified offline).");
  process.exit(0);
}
