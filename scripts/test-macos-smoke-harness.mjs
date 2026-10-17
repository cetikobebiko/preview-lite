#!/usr/bin/env node
/**
 * Fast-Local Test Suite for macOS Smoke Harness (11I-E2)
 *
 * Runs offline without release build artifacts or external tools.
 * Verifies:
 * - Missing artifact handling (exit code 2 ARTIFACT_UNAVAILABLE, schema validity)
 * - CLI argument parsing and help output
 * - Mach-O 64-bit arm64 architecture validator helper
 * - Info.plist parsing helper
 * - Mock bundle structure validation logic
 * - Smoke session interpretation & assertion logic
 * - Evidence schema 1.0.0 compliance
 */

import {
  validateMachOArchitecture,
  parseInfoPlist,
  validateBundleStructure,
  interpretSmoke,
  EXPECTED_RUNTIME_REL,
  EXPECTED_EXECUTABLE,
  EXPECTED_APP_NAME
} from "./artifact-smoke-macos.mjs";

import {
  createTempWorkspace,
  cleanupTempWorkspace,
  createEmptyEvidence,
  validateEvidenceSchema,
  computeOverallStatus,
  EXIT_CODES,
  CANONICAL_SMOKE_FIXTURE
} from "./artifact-smoke-common.mjs";

import { spawnSync } from "node:child_process";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { writeFileSync, mkdirSync, chmodSync } from "node:fs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = resolve(__filename, "..");
const REPO_ROOT = resolve(__dirname, "..");

let passed = 0;
let failed = 0;

function assert(condition, message) {
  if (condition) {
    passed++;
    console.log(`  ✓ ${message}`);
  } else {
    failed++;
    console.error(`  ✗ FAIL: ${message}`);
  }
}

console.log("=== Running macOS Artifact Smoke Harness Fast-Local Tests ===\n");

// ─── TEST 1: Missing Artifact CLI Behavior (Fail-Closed) ───
console.log("[Test 1] Missing artifact fail-closed behavior...");
{
  const scriptPath = resolve(__dirname, "artifact-smoke-macos.mjs");
  const res = spawnSync(process.execPath, [scriptPath, "--artifact", "/nonexistent/path/PreviewLite.dmg"], {
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 10000
  });

  assert(res.status === EXIT_CODES.ARTIFACT_UNAVAILABLE, `Exits with ARTIFACT_UNAVAILABLE (code 2), got ${res.status}`);

  let evidence = null;
  try {
    evidence = JSON.parse(res.stdout.toString().trim());
  } catch (e) {
    // maybe stderr has error
  }

  assert(evidence !== null, "Produces valid JSON evidence on missing artifact");
  if (evidence) {
    assert(evidence.installResult.status === "ARTIFACT_UNAVAILABLE", "installResult.status is ARTIFACT_UNAVAILABLE");
    assert(evidence.launchResult.status === "ARTIFACT_UNAVAILABLE", "launchResult.status is ARTIFACT_UNAVAILABLE");
    assert(evidence.overallStatus === "FAIL", "overallStatus is FAIL");
    assert(validateEvidenceSchema(evidence) === true, "Evidence satisfies schema 1.0.0");
  }
}

// ─── TEST 2: CLI --help ───
console.log("\n[Test 2] CLI --help output...");
{
  const scriptPath = resolve(__dirname, "artifact-smoke-macos.mjs");
  const res = spawnSync(process.execPath, [scriptPath, "--help"], {
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 5000
  });

  assert(res.status === 0, "CLI --help exits with code 0");
  assert(res.stdout.toString().includes("MACOS_DMG"), "Help mentions MACOS_DMG");
  assert(res.stdout.toString().includes("MACOS_APP_BUNDLE"), "Help mentions MACOS_APP_BUNDLE");
}

// ─── TEST 3: Mach-O Architecture Validator ───
console.log("\n[Test 3] Mach-O architecture validation...");
{
  const realDylib = resolve(REPO_ROOT, "src-tauri/lib/libpdfium.dylib");
  const realRes = validateMachOArchitecture(realDylib);
  assert(realRes.ok === true, "Staged libpdfium.dylib is recognized as valid Mach-O");
  assert(realRes.architecture === "arm64", `Staged libpdfium.dylib architecture is arm64 (got ${realRes.architecture})`);

  // Test with invalid / synthetic files
  const tmpDir = createTempWorkspace("test-macho-");
  try {
    // 1. Non-existent file
    const nonExistent = validateMachOArchitecture(join(tmpDir, "missing.dylib"));
    assert(nonExistent.ok === false, "Non-existent file returns ok: false");

    // 2. Truncated file (< 8 bytes)
    const shortFile = join(tmpDir, "short.dylib");
    writeFileSync(shortFile, Buffer.from([0x01, 0x02]));
    const shortRes = validateMachOArchitecture(shortFile);
    assert(shortRes.ok === false && shortRes.error.includes("too small"), "Short file rejected");

    // 3. Fake x86_64 Mach-O
    const x86File = join(tmpDir, "x86.dylib");
    const x86Buf = Buffer.alloc(16);
    x86Buf.writeUInt32LE(0xfeedfacf, 0); // MH_MAGIC_64
    x86Buf.writeInt32LE(0x01000007, 4);  // CPU_TYPE_X86_64
    writeFileSync(x86File, x86Buf);
    const x86Res = validateMachOArchitecture(x86File);
    assert(x86Res.ok === false && x86Res.architecture === "x86_64", "x86_64 Mach-O rejected with mismatch");

    // 4. Fake non-Mach-O file
    const fakeFile = join(tmpDir, "text.dylib");
    writeFileSync(fakeFile, Buffer.from("NOT_A_MACHO_BINARY"));
    const fakeRes = validateMachOArchitecture(fakeFile);
    assert(fakeRes.ok === false && fakeRes.error.includes("Not a Mach-O"), "Arbitrary non-Mach-O file rejected");
  } finally {
    cleanupTempWorkspace(tmpDir);
  }
}

// ─── TEST 4: Info.plist Parser ───
console.log("\n[Test 4] Info.plist parsing...");
{
  const samplePlist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>preview-alternative</string>
  <key>CFBundleIdentifier</key>
  <string>com.preview.lite</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleName</key>
  <string>PreviewLite</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
</dict>
</plist>`;

  const parsed = parseInfoPlist(samplePlist);
  assert(parsed.executable === "preview-alternative", `CFBundleExecutable parsed: ${parsed.executable}`);
  assert(parsed.identifier === "com.preview.lite", `CFBundleIdentifier parsed: ${parsed.identifier}`);
  assert(parsed.packageType === "APPL", `CFBundlePackageType parsed: ${parsed.packageType}`);
  assert(parsed.name === "PreviewLite", `CFBundleName parsed: ${parsed.name}`);
  assert(parsed.version === "0.1.0", `CFBundleShortVersionString parsed: ${parsed.version}`);
}

// ─── TEST 5: Mock Bundle Structure Validator ───
console.log("\n[Test 5] Bundle structural validation...");
{
  const tmpDir = createTempWorkspace("test-bundle-");
  try {
    const mockApp = join(tmpDir, "PreviewLite.app");
    const contents = join(mockApp, "Contents");
    const macos = join(contents, "MacOS");
    const resources = join(contents, "Resources");
    const resLib = join(resources, "lib");

    mkdirSync(macos, { recursive: true });
    mkdirSync(resLib, { recursive: true });

    // Incomplete bundle (missing Info.plist & runtime)
    const incompleteVal = validateBundleStructure(mockApp);
    assert(incompleteVal.valid === false, "Incomplete bundle fails validation");
    assert(incompleteVal.errors.some(e => e.includes("Info.plist")), "Reports missing Info.plist");
    assert(incompleteVal.errors.some(e => e.includes("runtime missing")), "Reports missing runtime");

    // Add Info.plist
    writeFileSync(join(contents, "Info.plist"), `<plist><dict><key>CFBundleExecutable</key><string>preview-alternative</string></dict></plist>`);

    // Add executable
    const exePath = join(macos, "preview-alternative");
    writeFileSync(exePath, "#!/bin/sh\nexit 0\n");
    chmodSync(exePath, 0o755);

    // Add mock runtime
    writeFileSync(join(resLib, "libpdfium.dylib"), "MOCK_DYLIB");

    const completeVal = validateBundleStructure(mockApp);
    assert(completeVal.valid === true, "Complete bundle passes structural validation");
    assert(completeVal.executablePath === exePath, "Executable path correctly resolved");
    assert(completeVal.runtimePath === join(mockApp, EXPECTED_RUNTIME_REL), "Runtime path correctly resolved");
  } finally {
    cleanupTempWorkspace(tmpDir);
  }
}

// ─── TEST 6: Smoke Session Interpretation Logic ───
console.log("\n[Test 6] Smoke session interpretation logic...");
{
  // 1. Successful session
  const successSession = {
    code: 0,
    open: { status: "ok", page_count: 2 },
    render: { status: "ok", width: 148, height: 210, data_len: 124320 },
    text1: { status: "ok", text: "RELEASE_SMOKE_ALPHA_TOKEN PAGE_MARKER_PAGE_1" },
    text2: { status: "ok", text: "RELEASE_SMOKE_BETA_TOKEN CANONICAL VERIFICATION TARGET PAGE_MARKER_PAGE_2" }
  };

  const interpretedSuccess = interpretSmoke(successSession);
  assert(interpretedSuccess.smokeOk === true, "Valid smoke session interprets as smokeOk: true");
  assert(interpretedSuccess.pageCount === 2, "Page count is 2");
  assert(interpretedSuccess.tokens.length >= 3, `Found ${interpretedSuccess.tokens.length} canonical tokens (>=3)`);
  assert(interpretedSuccess.renderTest.status === "PASS", "Render test is PASS");

  // 2. Failed session (page count mismatch)
  const badPageSession = {
    code: 0,
    open: { status: "ok", page_count: 1 },
    render: { status: "ok", width: 148, height: 210 },
    text1: { status: "ok", text: "RELEASE_SMOKE_ALPHA_TOKEN" },
    text2: { status: "ok", text: "RELEASE_SMOKE_BETA_TOKEN" }
  };
  const interpretedBadPage = interpretSmoke(badPageSession);
  assert(interpretedBadPage.smokeOk === false, "Page count != 2 fails smokeOk");

  // 3. Failed session (missing canonical tokens)
  const missingTokenSession = {
    code: 0,
    open: { status: "ok", page_count: 2 },
    render: { status: "ok", width: 148, height: 210 },
    text1: { status: "ok", text: "some generic text" },
    text2: { status: "ok", text: "other generic text" }
  };
  const interpretedMissingTokens = interpretSmoke(missingTokenSession);
  assert(interpretedMissingTokens.smokeOk === false, "Missing tokens fails smokeOk");
}

// ─── TEST 7: Evidence Schema 1.0.0 Compliance ───
console.log("\n[Test 7] Evidence schema 1.0.0 compliance...");
{
  const evidence = createEmptyEvidence("macos", "arm64", "MACOS_DMG", "/path/to/PreviewLite.dmg", "test-run");
  evidence.installResult = {
    status: "PASS",
    method: "dmg_mount_copy",
    installPath: "/tmp/test/PreviewLite.app",
    durationMs: 120,
    error: null
  };
  evidence.launchResult = {
    status: "PASS",
    executablePath: "/tmp/test/PreviewLite.app/Contents/MacOS/preview-alternative",
    pid: 1234,
    startupDurationMs: 50,
    error: null
  };
  evidence.runtimeResolutionResult = {
    status: "PASS",
    resolvedPath: "/tmp/test/PreviewLite.app/Contents/Resources/lib/libpdfium.dylib",
    source: "packaged",
    sha256: "cb167bb40974e409a4de3e37fc97fe29802a33aa0bfb75ce0a4e6c991c7857ab",
    error: null
  };
  evidence.smokeDocumentResult = {
    status: "PASS",
    fixtureId: "core-release-smoke",
    fixturePath: "/tmp/release-smoke.pdf",
    pageCount: 2,
    textTokensFound: ["RELEASE_SMOKE_ALPHA_TOKEN", "RELEASE_SMOKE_BETA_TOKEN", "CANONICAL VERIFICATION TARGET"],
    renderTest: { status: "PASS", operation: "thumbnail", dimensions: "148x210", durationMs: 25 },
    workerUsed: true,
    workerSpawnMethod: "production_ipc",
    error: null
  };
  evidence.shutdownResult = {
    status: "PASS",
    cleanExit: true,
    workerCleanup: true,
    durationMs: 10,
    error: null
  };

  const overall = computeOverallStatus(evidence);
  evidence.overallStatus = overall;
  assert(overall === "PASS", "computeOverallStatus returns PASS when all required PASS");
  assert(validateEvidenceSchema(evidence) === true, "Populated evidence passes validateEvidenceSchema");
}

console.log("\n---------------------------------------------------");
console.log(`Results: ${passed} passed, ${failed} failed`);
if (failed > 0) {
  process.exit(1);
} else {
  console.log("All fast-local macOS smoke harness tests passed.");
  process.exit(0);
}
