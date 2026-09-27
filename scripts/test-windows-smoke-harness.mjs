#!/usr/bin/env node
/**
 * Fast-Local Test Suite for Windows Smoke Harness (11I-E4)
 *
 * Runs offline without release build artifacts or Windows-native tools.
 * Verifies:
 * - Missing artifact handling (exit code 2 ARTIFACT_UNAVAILABLE, schema validity)
 * - CLI argument parsing and help output
 * - Pure Node PE 64-bit x86_64 architecture validator helper
 * - Mock installed layout validation logic (MSI and NSIS)
 * - Smoke session interpretation & assertion logic
 * - Evidence schema 1.0.0 compliance
 * - False-green fail-closed semantics
 * - Adversarial contamination hardening verification
 */

import {
  validatePeArchitecture,
  validateInstalledLayout,
  interpretSmoke,
  EXPECTED_RUNTIME_FILENAME,
  EXPECTED_RESOURCE_SUBDIR,
  EXPECTED_EXECUTABLE,
  EXPECTED_APP_NAME,
  WINDOWS_ARTIFACT_TYPES
} from "./artifact-smoke-windows.mjs";

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
const __dirname = dirname(__filename);
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

console.log("=== Running Windows Artifact Smoke Harness Fast-Local Tests ===\n");

// ─── TEST 1: Missing Artifact CLI Behavior (Fail-Closed) ───
console.log("[Test 1] Missing artifact fail-closed behavior...");
{
  const scriptPath = resolve(__dirname, "artifact-smoke-windows.mjs");

  // 1a. Missing MSI artifact
  const resMsi = spawnSync(process.execPath, [scriptPath, "--artifact", "/nonexistent/path/PreviewLite.msi"], {
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 10000
  });

  assert(resMsi.status === EXIT_CODES.ARTIFACT_UNAVAILABLE, `MSI exits with ARTIFACT_UNAVAILABLE (code 2), got ${resMsi.status}`);

  let evidenceMsi = null;
  try {
    evidenceMsi = JSON.parse(resMsi.stdout.toString().trim());
  } catch (e) {}

  assert(evidenceMsi !== null, "Produces valid JSON evidence on missing MSI artifact");
  if (evidenceMsi) {
    assert(evidenceMsi.artifactType === "WINDOWS_MSI", `artifactType is WINDOWS_MSI (got ${evidenceMsi.artifactType})`);
    assert(evidenceMsi.installResult.status === "ARTIFACT_UNAVAILABLE", "installResult.status is ARTIFACT_UNAVAILABLE");
    assert(evidenceMsi.launchResult.status === "ARTIFACT_UNAVAILABLE", "launchResult.status is ARTIFACT_UNAVAILABLE");
    assert(evidenceMsi.overallStatus === "FAIL", "overallStatus is FAIL");
    assert(validateEvidenceSchema(evidenceMsi) === true, "Evidence satisfies schema 1.0.0");
  }

  // 1b. Missing NSIS artifact
  const resNsis = spawnSync(process.execPath, [scriptPath, "--artifact", "/nonexistent/path/PreviewLite-setup.exe"], {
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 10000
  });

  assert(resNsis.status === EXIT_CODES.ARTIFACT_UNAVAILABLE, `NSIS exits with ARTIFACT_UNAVAILABLE (code 2), got ${resNsis.status}`);

  let evidenceNsis = null;
  try {
    evidenceNsis = JSON.parse(resNsis.stdout.toString().trim());
  } catch (e) {}

  assert(evidenceNsis !== null, "Produces valid JSON evidence on missing NSIS artifact");
  if (evidenceNsis) {
    assert(evidenceNsis.artifactType === "WINDOWS_NSIS", `artifactType is WINDOWS_NSIS (got ${evidenceNsis.artifactType})`);
    assert(evidenceNsis.installResult.status === "ARTIFACT_UNAVAILABLE", "NSIS installResult.status is ARTIFACT_UNAVAILABLE");
    assert(evidenceNsis.overallStatus === "FAIL", "NSIS overallStatus is FAIL");
    assert(validateEvidenceSchema(evidenceNsis) === true, "NSIS evidence satisfies schema 1.0.0");
  }
}

// ─── TEST 2: CLI --help and Argument Validation ───
console.log("\n[Test 2] CLI --help output and argument validation...");
{
  const scriptPath = resolve(__dirname, "artifact-smoke-windows.mjs");
  const res = spawnSync(process.execPath, [scriptPath, "--help"], {
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 5000
  });

  assert(res.status === 0, "CLI --help exits with code 0");
  assert(res.stdout.toString().includes("WINDOWS_MSI"), "Help mentions WINDOWS_MSI");
  assert(res.stdout.toString().includes("WINDOWS_NSIS"), "Help mentions WINDOWS_NSIS");

  // Invalid artifact type rejected
  const tmpDir = createTempWorkspace("test-args-");
  try {
    const dummyFile = join(tmpDir, "dummy.xyz");
    writeFileSync(dummyFile, "dummy");
    const invalidRes = spawnSync(process.execPath, [scriptPath, "--artifact", dummyFile], {
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 5000
    });
    assert(invalidRes.status === EXIT_CODES.INVALID_ARGS, `Invalid artifact extension exits with INVALID_ARGS (code 3), got ${invalidRes.status}`);
  } finally {
    cleanupTempWorkspace(tmpDir);
  }
}

// ─── TEST 3: Pure Node PE Architecture Validator ───
console.log("\n[Test 3] PE architecture validation (pure Node Buffer)...");
{
  const tmpDir = createTempWorkspace("test-pe-");
  try {
    // 1. Non-existent file
    const nonExistent = validatePeArchitecture(join(tmpDir, "missing.dll"));
    assert(nonExistent.ok === false, "Non-existent file returns ok: false");

    // 2. Truncated file (< 64 bytes)
    const shortFile = join(tmpDir, "short.dll");
    writeFileSync(shortFile, Buffer.from([0x4d, 0x5a, 0x90, 0x00]));
    const shortRes = validatePeArchitecture(shortFile);
    assert(shortRes.ok === false && shortRes.error.includes("too small"), "Short file rejected (< 64 bytes)");

    // 3. Synthetic valid 64-bit little-endian x86_64 PE
    // DOS header: 'MZ' at 0..1, e_lfanew at 0x3C = 0x80
    // At 0x80: 'PE\0\0' (0x50, 0x45, 0x00, 0x00)
    // At 0x84: Machine = 0x8664 (IMAGE_FILE_MACHINE_AMD64)
    // At 0x84 + 18 = 0x96: Characteristics = 0x0002 (EXECUTABLE_IMAGE, not 32-bit)
    const pe64File = join(tmpDir, "valid_x86_64.dll");
    const pe64Buf = Buffer.alloc(256);
    pe64Buf[0] = 0x4d; // 'M'
    pe64Buf[1] = 0x5a; // 'Z'
    const peOffset = 0x80;
    pe64Buf.writeUInt32LE(peOffset, 0x3c);
    pe64Buf[peOffset] = 0x50;     // 'P'
    pe64Buf[peOffset + 1] = 0x45; // 'E'
    pe64Buf[peOffset + 2] = 0x00;
    pe64Buf[peOffset + 3] = 0x00;
    pe64Buf.writeUInt16LE(0x8664, peOffset + 4);  // IMAGE_FILE_MACHINE_AMD64
    pe64Buf.writeUInt16LE(0x0002, peOffset + 22); // Characteristics (non-32-bit)
    writeFileSync(pe64File, pe64Buf);

    const pe64Res = validatePeArchitecture(pe64File);
    assert(pe64Res.ok === true, "Synthetic x86_64 PE passes validation");
    assert(pe64Res.architecture === "x86_64", "Recognized architecture is x86_64");
    assert(pe64Res.machineCode === "0x8664", "Machine code is 0x8664");

    // 4. Synthetic 32-bit x86 PE (Machine = 0x014c / IMAGE_FILE_MACHINE_I386)
    const pe32File = join(tmpDir, "pe32.dll");
    const pe32Buf = Buffer.alloc(256);
    pe32Buf[0] = 0x4d;
    pe32Buf[1] = 0x5a;
    pe32Buf.writeUInt32LE(peOffset, 0x3c);
    pe32Buf[peOffset] = 0x50;
    pe32Buf[peOffset + 1] = 0x45;
    pe32Buf[peOffset + 2] = 0x00;
    pe32Buf[peOffset + 3] = 0x00;
    pe32Buf.writeUInt16LE(0x014c, peOffset + 4); // IMAGE_FILE_MACHINE_I386
    writeFileSync(pe32File, pe32Buf);

    const pe32Res = validatePeArchitecture(pe32File);
    assert(pe32Res.ok === false && pe32Res.error.includes("mismatch"), "32-bit PE rejected fail-closed");

    // 5. Synthetic ARM64 PE (Machine = 0xaa64 / IMAGE_FILE_MACHINE_ARM64)
    const peArmFile = join(tmpDir, "pe_arm64.dll");
    const peArmBuf = Buffer.alloc(256);
    peArmBuf[0] = 0x4d;
    peArmBuf[1] = 0x5a;
    peArmBuf.writeUInt32LE(peOffset, 0x3c);
    peArmBuf[peOffset] = 0x50;
    peArmBuf[peOffset + 1] = 0x45;
    peArmBuf[peOffset + 2] = 0x00;
    peArmBuf[peOffset + 3] = 0x00;
    peArmBuf.writeUInt16LE(0xaa64, peOffset + 4); // IMAGE_FILE_MACHINE_ARM64
    writeFileSync(peArmFile, peArmBuf);

    const peArmRes = validatePeArchitecture(peArmFile);
    assert(peArmRes.ok === false && peArmRes.architecture === "arm64", "ARM64 PE rejected with mismatch");

    // 6. Synthetic 64-bit AMD64 PE with 32-bit machine flag erroneously set (bit 0x0020)
    const pe32FlagFile = join(tmpDir, "pe32_flag.dll");
    const pe32FlagBuf = Buffer.alloc(256);
    pe32FlagBuf[0] = 0x4d;
    pe32FlagBuf[1] = 0x5a;
    pe32FlagBuf.writeUInt32LE(peOffset, 0x3c);
    pe32FlagBuf[peOffset] = 0x50;
    pe32FlagBuf[peOffset + 1] = 0x45;
    pe32FlagBuf[peOffset + 2] = 0x00;
    pe32FlagBuf[peOffset + 3] = 0x00;
    pe32FlagBuf.writeUInt16LE(0x8664, peOffset + 4);
    pe32FlagBuf.writeUInt16LE(0x0020, peOffset + 22); // IMAGE_FILE_32BIT_MACHINE set!
    writeFileSync(pe32FlagFile, pe32FlagBuf);

    const pe32FlagRes = validatePeArchitecture(pe32FlagFile);
    assert(pe32FlagRes.ok === false && pe32FlagRes.error.includes("32BIT_MACHINE"), "PE with 32-bit flag set rejected fail-closed");

    // 7. Arbitrary non-PE file (>= 64 bytes)
    const fakeFile = join(tmpDir, "text.dll");
    writeFileSync(fakeFile, Buffer.from("NOT_A_VALID_PE_COFF_EXECUTABLE_FILE_BUFFER_LONGER_THAN_64_BYTES_FOR_TESTING"));
    const fakeRes = validatePeArchitecture(fakeFile);
    assert(fakeRes.ok === false && fakeRes.error.includes("MZ"), "Non-PE file without MZ rejected");
  } finally {
    cleanupTempWorkspace(tmpDir);
  }
}

// ─── TEST 4: Mock Installed Layout Validator ───
console.log("\n[Test 4] Mock installed layout validation...");
{
  const tmpDir = createTempWorkspace("test-win-layout-");
  try {
    const installPrefix = join(tmpDir, "install");
    mkdirSync(installPrefix, { recursive: true });

    // 1. Incomplete layout (missing binary & runtime)
    const incompleteVal = validateInstalledLayout(installPrefix);
    assert(incompleteVal.valid === false, "Incomplete layout fails validation");
    assert(incompleteVal.errors.some(e => e.includes(EXPECTED_EXECUTABLE)), "Reports missing executable");
    assert(incompleteVal.errors.some(e => e.includes(EXPECTED_RUNTIME_FILENAME)), "Reports missing runtime");

    // 2. Add executable at root
    const exePath = join(installPrefix, EXPECTED_EXECUTABLE);
    writeFileSync(exePath, "MOCK_EXE");

    // 3. Add runtime at lib/pdfium.dll
    const libDir = join(installPrefix, EXPECTED_RESOURCE_SUBDIR);
    mkdirSync(libDir, { recursive: true });
    const runtimePath = join(libDir, EXPECTED_RUNTIME_FILENAME);
    writeFileSync(runtimePath, "MOCK_PDFIUM_DLL");

    const completeVal = validateInstalledLayout(installPrefix);
    assert(completeVal.valid === true, "Complete flat layout passes validation");
    assert(completeVal.mainExecutable.status === "PASS", "mainExecutable status is PASS");
    assert(completeVal.mainExecutable.path === exePath, `Main executable path matches: ${completeVal.mainExecutable.path}`);
    assert(completeVal.worker.status === "PASS", "worker status is PASS");
    assert(completeVal.worker.entry === "--run-worker", "worker entry is --run-worker");
    assert(completeVal.runtime.status === "PASS", "runtime status is PASS");
    assert(completeVal.runtime.path === runtimePath, `Runtime path matches: ${completeVal.runtime.path}`);

    // 4. Nested layout (e.g. installDir/PreviewLite/preview-alternative.exe + PreviewLite/lib/pdfium.dll)
    const nestedPrefix = join(tmpDir, "nested_install");
    const appDir = join(nestedPrefix, EXPECTED_APP_NAME);
    const nestedLib = join(appDir, EXPECTED_RESOURCE_SUBDIR);
    mkdirSync(nestedLib, { recursive: true });
    const nestedExe = join(appDir, EXPECTED_EXECUTABLE);
    writeFileSync(nestedExe, "MOCK_EXE");
    writeFileSync(join(nestedLib, EXPECTED_RUNTIME_FILENAME), "MOCK_PDFIUM_DLL");

    const nestedVal = validateInstalledLayout(nestedPrefix);
    assert(nestedVal.valid === true, "Complete nested layout passes validation");
    assert(nestedVal.executablePath === nestedExe, `Nested executable path matches: ${nestedVal.executablePath}`);
  } finally {
    cleanupTempWorkspace(tmpDir);
  }
}

// ─── TEST 5: Smoke Session Interpretation Logic ───
console.log("\n[Test 5] Smoke session interpretation logic...");
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

// ─── TEST 6: Evidence Schema 1.0.0 Compliance & Fail-Closed Semantics ───
console.log("\n[Test 6] Evidence schema 1.0.0 compliance & fail-closed semantics...");
{
  const evidence = createEmptyEvidence("windows", "x86_64", "WINDOWS_MSI", "/path/to/PreviewLite.msi", "test-run");
  evidence.installResult = {
    status: "PASS",
    method: "msi_install",
    installPath: "/tmp/test/install",
    durationMs: 150,
    error: null
  };
  evidence.launchResult = {
    status: "PASS",
    executablePath: "/tmp/test/install/preview-alternative.exe",
    pid: 4321,
    startupDurationMs: 60,
    error: null
  };
  evidence.runtimeResolutionResult = {
    status: "PASS",
    resolvedPath: "/tmp/test/install/lib/pdfium.dll",
    source: "packaged",
    sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    architecture: "x86_64",
    error: null
  };
  evidence.smokeDocumentResult = {
    status: "PASS",
    fixtureId: "core-release-smoke",
    fixturePath: "/tmp/release-smoke.pdf",
    pageCount: 2,
    textTokensFound: ["RELEASE_SMOKE_ALPHA_TOKEN", "RELEASE_SMOKE_BETA_TOKEN", "CANONICAL VERIFICATION TARGET"],
    renderTest: { status: "PASS", operation: "thumbnail", dimensions: "148x210", durationMs: 30 },
    workerUsed: true,
    workerSpawnMethod: "production_ipc",
    error: null
  };
  evidence.hardeningResult = {
    status: "PASS",
    decoyDllCwd: "PASS",
    pathContamination: "PASS",
    error: null
  };
  evidence.shutdownResult = {
    status: "PASS",
    cleanExit: true,
    workerCleanup: true,
    durationMs: 15,
    error: null
  };

  const overall = computeOverallStatus(evidence);
  evidence.overallStatus = overall;
  assert(overall === "PASS", "computeOverallStatus returns PASS when all required checks PASS");
  assert(validateEvidenceSchema(evidence) === true, "Populated Windows evidence passes validateEvidenceSchema");

  // False-green tests: any individual required check failure forces overall FAIL
  const failRuntime = JSON.parse(JSON.stringify(evidence));
  failRuntime.runtimeResolutionResult.status = "FAIL";
  assert(computeOverallStatus(failRuntime) === "FAIL", "Fails closed if runtime check is FAIL");

  const failInstall = JSON.parse(JSON.stringify(evidence));
  failInstall.installResult.status = "FAIL";
  assert(computeOverallStatus(failInstall) === "FAIL", "Fails closed if install check is FAIL");

  const failLaunch = JSON.parse(JSON.stringify(evidence));
  failLaunch.launchResult.status = "FAIL";
  assert(computeOverallStatus(failLaunch) === "FAIL", "Fails closed if launch check is FAIL");

  const failSmoke = JSON.parse(JSON.stringify(evidence));
  failSmoke.smokeDocumentResult.status = "FAIL";
  assert(computeOverallStatus(failSmoke) === "FAIL", "Fails closed if smoke document is FAIL");

  const failHardening = JSON.parse(JSON.stringify(evidence));
  failHardening.hardeningResult.status = "FAIL";
  assert(computeOverallStatus(failHardening) === "FAIL", "Fails closed if hardening check is FAIL");

  const failShutdown = JSON.parse(JSON.stringify(evidence));
  failShutdown.shutdownResult.status = "FAIL";
  assert(computeOverallStatus(failShutdown) === "FAIL", "Fails closed if shutdown check is FAIL");

  // Packaged runtime source enforcement: if status is PASS, source MUST be 'packaged'
  const nonPackagedEvidence = JSON.parse(JSON.stringify(evidence));
  nonPackagedEvidence.runtimeResolutionResult.source = "development_override";
  let nonPackagedRejected = false;
  try {
    validateEvidenceSchema(nonPackagedEvidence);
  } catch (e) {
    nonPackagedRejected = true;
  }
  assert(nonPackagedRejected === true, "Fails validation if runtime source is not 'packaged' on PASS");
}

console.log("\n---------------------------------------------------");
console.log(`Results: ${passed} passed, ${failed} failed`);
if (failed > 0) {
  process.exit(1);
} else {
  console.log("All fast-local Windows smoke harness tests passed.");
  process.exit(0);
}
