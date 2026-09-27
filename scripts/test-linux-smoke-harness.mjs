#!/usr/bin/env node
/**
 * Fast-Local Test Suite for Linux Smoke Harness (11I-E3)
 *
 * Runs offline without release build artifacts or external deb tools.
 * Verifies:
 * - Missing artifact handling (exit code 2 ARTIFACT_UNAVAILABLE, schema validity)
 * - CLI argument parsing and help output
 * - Pure Node ELF 64-bit x86_64 architecture validator helper
 * - Debian control text parser helper
 * - Mock installed layout validation logic
 * - Smoke session interpretation & assertion logic
 * - Evidence schema 1.0.0 compliance
 * - False-green fail-closed semantics
 */

import {
  validateElfArchitecture,
  parseDebControlText,
  validateInstalledLayout,
  interpretSmoke,
  EXPECTED_RUNTIME_FILENAME,
  EXPECTED_RESOURCE_SUBDIR,
  EXPECTED_EXECUTABLE,
  EXPECTED_APP_NAME
} from "./artifact-smoke-linux.mjs";

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

console.log("=== Running Linux Artifact Smoke Harness Fast-Local Tests ===\n");

// ─── TEST 1: Missing Artifact CLI Behavior (Fail-Closed) ───
console.log("[Test 1] Missing artifact fail-closed behavior...");
{
  const scriptPath = resolve(__dirname, "artifact-smoke-linux.mjs");
  const res = spawnSync(process.execPath, [scriptPath, "--artifact", "/nonexistent/path/preview-lite.deb"], {
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
  const scriptPath = resolve(__dirname, "artifact-smoke-linux.mjs");
  const res = spawnSync(process.execPath, [scriptPath, "--help"], {
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 5000
  });

  assert(res.status === 0, "CLI --help exits with code 0");
  assert(res.stdout.toString().includes("LINUX_DEB"), "Help mentions LINUX_DEB");
}

// ─── TEST 3: Pure Node ELF Architecture Validator ───
console.log("\n[Test 3] ELF architecture validation...");
{
  const tmpDir = createTempWorkspace("test-elf-");
  try {
    // 1. Non-existent file
    const nonExistent = validateElfArchitecture(join(tmpDir, "missing.so"));
    assert(nonExistent.ok === false, "Non-existent file returns ok: false");

    // 2. Truncated file (< 20 bytes)
    const shortFile = join(tmpDir, "short.so");
    writeFileSync(shortFile, Buffer.from([0x7f, 0x45, 0x4c, 0x46, 0x02]));
    const shortRes = validateElfArchitecture(shortFile);
    assert(shortRes.ok === false && shortRes.error.includes("too small"), "Short file rejected (< 20 bytes)");

    // 3. Synthetic valid 64-bit little-endian x86_64 ELF
    const x86_64File = join(tmpDir, "valid_x86_64.so");
    const x86_64Buf = Buffer.alloc(64);
    x86_64Buf[0] = 0x7f;
    x86_64Buf[1] = 0x45; // 'E'
    x86_64Buf[2] = 0x4c; // 'L'
    x86_64Buf[3] = 0x46; // 'F'
    x86_64Buf[4] = 2;    // ei_class = 2 (64-bit)
    x86_64Buf[5] = 1;    // ei_data = 1 (little-endian)
    x86_64Buf[6] = 1;    // ei_version = 1
    x86_64Buf.writeUInt16LE(0x003e, 18); // e_machine = EM_X86_64 (62)
    writeFileSync(x86_64File, x86_64Buf);

    const x86_64Res = validateElfArchitecture(x86_64File);
    assert(x86_64Res.ok === true, "Synthetic x86_64 ELF passes validation");
    assert(x86_64Res.architecture === "x86_64", "Recognized architecture is x86_64");

    // 4. Synthetic 32-bit ELF (ei_class = 1)
    const elf32File = join(tmpDir, "elf32.so");
    const elf32Buf = Buffer.alloc(64);
    elf32Buf[0] = 0x7f;
    elf32Buf[1] = 0x45;
    elf32Buf[2] = 0x4c;
    elf32Buf[3] = 0x46;
    elf32Buf[4] = 1; // 32-bit
    elf32Buf[5] = 1; // little-endian
    elf32Buf.writeUInt16LE(0x0003, 18); // EM_386
    writeFileSync(elf32File, elf32Buf);

    const elf32Res = validateElfArchitecture(elf32File);
    assert(elf32Res.ok === false && elf32Res.error.includes("64-bit"), "32-bit ELF rejected fail-closed");

    // 5. Synthetic ARM64 (AArch64) ELF (e_machine = 0x00b7)
    const arm64File = join(tmpDir, "arm64.so");
    const arm64Buf = Buffer.alloc(64);
    arm64Buf[0] = 0x7f;
    arm64Buf[1] = 0x45;
    arm64Buf[2] = 0x4c;
    arm64Buf[3] = 0x46;
    arm64Buf[4] = 2; // 64-bit
    arm64Buf[5] = 1; // little-endian
    arm64Buf.writeUInt16LE(0x00b7, 18); // EM_AARCH64
    writeFileSync(arm64File, arm64Buf);

    const arm64Res = validateElfArchitecture(arm64File);
    assert(arm64Res.ok === false && arm64Res.architecture === "arm64", "ARM64 ELF rejected with architecture mismatch");

    // 6. Arbitrary non-ELF file
    const fakeFile = join(tmpDir, "text.so");
    writeFileSync(fakeFile, Buffer.from("NOT_AN_ELF_SHARED_OBJECT_FILE_DATA"));
    const fakeRes = validateElfArchitecture(fakeFile);
    assert(fakeRes.ok === false && fakeRes.error.includes("Not a valid ELF binary"), "Non-ELF file rejected");
  } finally {
    cleanupTempWorkspace(tmpDir);
  }
}

// ─── TEST 4: Debian Control Text Parser ───
console.log("\n[Test 4] Debian control text parsing...");
{
  const sampleControl = `
Package: preview-lite
Version: 0.1.0
Architecture: amd64
Maintainer: PreviewLite Team <dev@previewlite.local>
Installed-Size: 45000
Depends: libc6 (>= 2.31), libgtk-3-0
Section: utils
Priority: optional
Description: Fast preview application
 PreviewLite provides high performance PDF viewing.
`;

  const parsed = parseDebControlText(sampleControl);
  assert(parsed.packageName === "preview-lite", `Package parsed: ${parsed.packageName}`);
  assert(parsed.version === "0.1.0", `Version parsed: ${parsed.version}`);
  assert(parsed.architecture === "amd64", `Architecture parsed: ${parsed.architecture}`);
  assert(parsed.maintainer === "PreviewLite Team <dev@previewlite.local>", `Maintainer parsed: ${parsed.maintainer}`);
  assert(parsed.description.includes("Fast preview"), "Description parsed");
}

// ─── TEST 5: Mock Installed Layout Validator ───
console.log("\n[Test 5] Installed layout validation...");
{
  const tmpDir = createTempWorkspace("test-layout-");
  try {
    const installPrefix = join(tmpDir, "install");
    mkdirSync(installPrefix, { recursive: true });

    // 1. Incomplete layout (missing binary & runtime)
    const incompleteVal = validateInstalledLayout(installPrefix, { packageName: "preview-lite" });
    assert(incompleteVal.valid === false, "Incomplete layout fails validation");
    assert(incompleteVal.errors.some(e => e.includes("executable")), "Reports missing executable");
    assert(incompleteVal.errors.some(e => e.includes("runtime")), "Reports missing runtime");

    // 2. Add executable
    const binDir = join(installPrefix, "usr/bin");
    mkdirSync(binDir, { recursive: true });
    const exePath = join(binDir, EXPECTED_EXECUTABLE);
    writeFileSync(exePath, "#!/bin/sh\nexit 0\n");
    chmodSync(exePath, 0o755);

    // 3. Add runtime at usr/lib/preview-lite/lib/libpdfium.so
    const runtimeDir = join(installPrefix, "usr/lib/preview-lite/lib");
    mkdirSync(runtimeDir, { recursive: true });
    const runtimePath = join(runtimeDir, EXPECTED_RUNTIME_FILENAME);
    writeFileSync(runtimePath, "MOCK_PDFIUM_SO");

    // 4. Add desktop file
    const appsDir = join(installPrefix, "usr/share/applications");
    mkdirSync(appsDir, { recursive: true });
    writeFileSync(join(appsDir, "PreviewLite.desktop"), "[Desktop Entry]\nType=Application\n");

    const completeVal = validateInstalledLayout(installPrefix, { packageName: "preview-lite" });
    assert(completeVal.valid === true, "Complete layout passes validation");
    assert(completeVal.mainExecutable.status === "PASS", "mainExecutable is PASS");
    assert(completeVal.mainExecutable.path === exePath, `Main executable path matches: ${completeVal.mainExecutable.path}`);
    assert(completeVal.worker.status === "PASS", "worker is PASS");
    assert(completeVal.runtime.status === "PASS", "runtime is PASS");
    assert(completeVal.runtime.path === runtimePath, `Runtime path matches: ${completeVal.runtime.path}`);
    assert(completeVal.resourcesPath === join(installPrefix, "usr/lib/preview-lite"), "resourcesPath correctly identified");
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

  // 4. Failed render
  const badRenderSession = {
    code: 0,
    open: { status: "ok", page_count: 2 },
    render: { status: "error", error: "render failed" },
    text1: { status: "ok", text: "RELEASE_SMOKE_ALPHA_TOKEN" },
    text2: { status: "ok", text: "RELEASE_SMOKE_BETA_TOKEN CANONICAL VERIFICATION TARGET" }
  };
  const interpretedBadRender = interpretSmoke(badRenderSession);
  assert(interpretedBadRender.smokeOk === false, "Failed render fails smokeOk");
  assert(interpretedBadRender.renderTest.status === "FAIL", "renderTest.status is FAIL");
}

// ─── TEST 7: Evidence Schema 1.0.0 Compliance ───
console.log("\n[Test 7] Evidence schema 1.0.0 compliance & fail-closed semantics...");
{
  const evidence = createEmptyEvidence("linux", "x86_64", "LINUX_DEB", "/path/to/preview-lite.deb", "test-run");
  evidence.installResult = {
    status: "PASS",
    method: "deb_install",
    installPath: "/tmp/test/install",
    durationMs: 120,
    error: null
  };
  evidence.launchResult = {
    status: "PASS",
    executablePath: "/tmp/test/install/usr/bin/preview-alternative",
    pid: 1234,
    startupDurationMs: 50,
    error: null
  };
  evidence.runtimeResolutionResult = {
    status: "PASS",
    resolvedPath: "/tmp/test/install/usr/lib/preview-lite/lib/libpdfium.so",
    source: "packaged",
    sha256: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    architecture: "x86_64",
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

  // False-green checks: Any required check failing causes overall FAIL
  const failRuntimeEvidence = JSON.parse(JSON.stringify(evidence));
  failRuntimeEvidence.runtimeResolutionResult.status = "FAIL";
  assert(computeOverallStatus(failRuntimeEvidence) === "FAIL", "Fails closed if runtime check FAIL");

  const failInstallEvidence = JSON.parse(JSON.stringify(evidence));
  failInstallEvidence.installResult.status = "FAIL";
  assert(computeOverallStatus(failInstallEvidence) === "FAIL", "Fails closed if install check FAIL");

  const failSmokeEvidence = JSON.parse(JSON.stringify(evidence));
  failSmokeEvidence.smokeDocumentResult.status = "FAIL";
  assert(computeOverallStatus(failSmokeEvidence) === "FAIL", "Fails closed if smoke document FAIL");

  const failShutdownEvidence = JSON.parse(JSON.stringify(evidence));
  failShutdownEvidence.shutdownResult.status = "FAIL";
  assert(computeOverallStatus(failShutdownEvidence) === "FAIL", "Fails closed if shutdown FAIL");
}

console.log("\n---------------------------------------------------");
console.log(`Results: ${passed} passed, ${failed} failed`);
if (failed > 0) {
  process.exit(1);
} else {
  console.log("All fast-local Linux smoke harness tests passed.");
  process.exit(0);
}
