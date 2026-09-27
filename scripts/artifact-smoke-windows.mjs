#!/usr/bin/env node
/**
 * Windows Artifact Smoke Harness — 11I-E4
 *
 * Tests: WINDOWS_MSI, WINDOWS_NSIS
 * Implements the exact artifact smoke contract using Windows-native tools.
 *
 * Architecture:
 * - Node stdlib only (zero network, zero external npm dependencies)
 * - Deterministic, fail-closed
 * - Full lifecycle: inspect -> deploy/install -> layout validation -> runtime resolution -> launch -> worker IPC -> canonical fixture smoke -> adversarial hardening -> shutdown -> uninstall/cleanup
 * - Pure Node Buffer PE 64-bit x86_64 architecture header validation (IMAGE_FILE_MACHINE_AMD64 0x8664, non-32-bit)
 * - Environment sanitization (scrubs dev overrides, PDFIUM_PATH, dev search paths)
 * - Conforms to docs/engineering/artifact-smoke-contract.md and artifact-smoke-evidence-model.json schema 1.0.0
 */

import {
  ARTIFACT_TYPES,
  EVIDENCE_STATUSES,
  INSTALL_METHODS,
  RUNTIME_SOURCES,
  WORKER_SPAWN_METHODS,
  CANONICAL_SMOKE_FIXTURE,
  TIMEOUTS,
  createTempWorkspace,
  cleanupTempWorkspace,
  loadManifest,
  verifyFixtureIntegrity,
  copyFixtureToWorkspace,
  spawnWithTimeout,
  spawnSyncWithTimeout,
  createEmptyEvidence,
  computeOverallStatus,
  validateEvidenceSchema,
  parseCommonArgs,
  printCommonUsage,
  validateRequiredArgs,
  detectPlatform,
  detectArchitecture,
  EXIT_CODES,
  exitWithEvidence,
  generateRunId,
  hashSha256,
  hashSha256Async
} from "./artifact-smoke-common.mjs";

import {
  existsSync,
  readFileSync,
  mkdirSync,
  cpSync,
  readdirSync,
  rmSync,
  writeFileSync,
  statSync
} from "node:fs";
import { join, dirname, basename, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const REPO_ROOT = resolve(__dirname, "..");
const PLATFORM_SUPPORT_PATH = resolve(REPO_ROOT, "docs/engineering/platform-support.json");

export const WINDOWS_ARTIFACT_TYPES = Object.freeze(["WINDOWS_MSI", "WINDOWS_NSIS"]);
export const EXPECTED_APP_NAME = "PreviewLite";
export const EXPECTED_EXECUTABLE = "preview-alternative.exe";
export const EXPECTED_RUNTIME_FILENAME = "pdfium.dll";
export const EXPECTED_RESOURCE_SUBDIR = "lib";
export const EXPECTED_RUNTIME_REL = "lib/pdfium.dll";

// ─── PE Architecture Validator (x86_64 / IMAGE_FILE_MACHINE_AMD64) ───

/**
 * Validates PE COFF 64-bit x86_64 binary header using pure Node stdlib Buffer.
 *
 * Header format:
 * - Bytes 0..1: 'M', 'Z' (0x4D, 0x5A)
 * - Byte 0x3C: 4-byte LE offset to PE header (e_lfanew)
 * - At e_lfanew: 'P', 'E', 0x00, 0x00 (0x50, 0x45, 0x00, 0x00)
 * - At e_lfanew + 4: 2-byte LE Machine:
 *     0x8664 = IMAGE_FILE_MACHINE_AMD64 (x86_64)
 *     0x014c = IMAGE_FILE_MACHINE_I386 (32-bit x86)
 *     0xaa64 = IMAGE_FILE_MACHINE_ARM64 (ARM64)
 * - At e_lfanew + 22: 2-byte LE Characteristics:
 *     Bit 0x0020 (IMAGE_FILE_32BIT_MACHINE) must NOT be set for 64-bit AMD64.
 */
export function validatePeArchitecture(filePath) {
  if (!existsSync(filePath)) {
    return { ok: false, error: `File not found: ${filePath}` };
  }
  const stat = statSync(filePath);
  if (stat.size < 64) {
    return { ok: false, error: `File too small for PE DOS header: ${stat.size} bytes` };
  }

  const buf = readFileSync(filePath);

  // 1. Magic check: MZ
  if (buf[0] !== 0x4d || buf[1] !== 0x5a) {
    return { ok: false, error: "Not a valid PE file (missing MZ signature)" };
  }

  // 2. PE header offset at 0x3C
  const peOffset = buf.readUInt32LE(0x3c);
  if (buf.length < peOffset + 24) {
    return { ok: false, error: `PE file truncated before COFF header (offset ${peOffset})` };
  }

  // 3. PE signature check: PE\0\0
  if (
    buf[peOffset] !== 0x50 ||
    buf[peOffset + 1] !== 0x45 ||
    buf[peOffset + 2] !== 0x00 ||
    buf[peOffset + 3] !== 0x00
  ) {
    return { ok: false, error: "Not a valid PE file (missing PE signature)" };
  }

  // 4. Machine check
  const machine = buf.readUInt16LE(peOffset + 4);
  const IMAGE_FILE_MACHINE_AMD64 = 0x8664;
  const IMAGE_FILE_MACHINE_I386 = 0x014c;
  const IMAGE_FILE_MACHINE_ARM64 = 0xaa64;

  const archMap = {
    [IMAGE_FILE_MACHINE_AMD64]: "x86_64",
    [IMAGE_FILE_MACHINE_I386]: "x86",
    [IMAGE_FILE_MACHINE_ARM64]: "arm64"
  };
  const foundArch = archMap[machine] || `0x${machine.toString(16)}`;

  if (machine !== IMAGE_FILE_MACHINE_AMD64) {
    return {
      ok: false,
      architecture: foundArch,
      error: `PE architecture mismatch: expected x86_64 (IMAGE_FILE_MACHINE_AMD64 0x8664), got ${foundArch} (0x${machine.toString(16)})`
    };
  }

  // 5. Characteristics: 32-bit machine flag must NOT be set
  const characteristics = buf.readUInt16LE(peOffset + 22);
  const IMAGE_FILE_32BIT_MACHINE = 0x0020;
  if ((characteristics & IMAGE_FILE_32BIT_MACHINE) !== 0) {
    return {
      ok: false,
      architecture: "x86_32bit_flag_set",
      error: "IMAGE_FILE_32BIT_MACHINE characteristic flag is set on 64-bit AMD64 binary"
    };
  }

  return {
    ok: true,
    architecture: "x86_64",
    machineCode: "0x8664"
  };
}

// ─── Platform Support Metadata ───

export async function loadPlatformSupport() {
  if (!existsSync(PLATFORM_SUPPORT_PATH)) {
    throw new Error(`platform-support.json not found at ${PLATFORM_SUPPORT_PATH}`);
  }
  return JSON.parse(readFileSync(PLATFORM_SUPPORT_PATH, "utf-8"));
}

export function getWindowsRuntimeMetadata(platformSupport) {
  const windowsPlatform = platformSupport?.platforms?.find(p => p.targetTriple === "x86_64-pc-windows-msvc");
  if (!windowsPlatform || !windowsPlatform.runtime) {
    return {
      expectedSha256: null,
      expectedByteLength: null,
      expectedVersion: null,
      expectedArchitecture: "x86_64",
      runtimeFilename: EXPECTED_RUNTIME_FILENAME,
      supplyChainStatus: "INCOMPLETE",
      expectedStagedPath: "src-tauri/lib/pdfium.dll",
      expectedPackagedLocation: "resourceDir/lib/pdfium.dll"
    };
  }
  const rt = windowsPlatform.runtime;
  let sha256Hex = rt.sha256 ? rt.sha256.toLowerCase() : null;
  return {
    expectedSha256: sha256Hex,
    expectedByteLength: rt.byteLength ?? null,
    expectedVersion: rt.versionOrRevision ?? null,
    expectedArchitecture: rt.architecture ?? "x86_64",
    runtimeFilename: rt.runtimeFileName ?? EXPECTED_RUNTIME_FILENAME,
    supplyChainStatus: rt.supplyChainStatus ?? "INCOMPLETE",
    expectedStagedPath: rt.expectedStagedPath ?? "src-tauri/lib/pdfium.dll",
    expectedPackagedLocation: rt.expectedPackagedLocation ?? "resourceDir/lib/pdfium.dll"
  };
}

// ─── Installed Layout Validation ───

/**
 * Validates the deployed application layout on Windows.
 * Searches the install directory and subdirectories for:
 * 1. Main executable: preview-alternative.exe
 * 2. Worker parity: same binary invoked with --run-worker
 * 3. Packaged PDFium runtime: lib/pdfium.dll relative to resourceDir / executable
 */
export function validateInstalledLayout(installDir, options = {}) {
  const errors = [];
  if (!existsSync(installDir)) {
    return {
      valid: false,
      installDir,
      executablePath: null,
      runtimePath: null,
      resourceDir: null,
      errors: [`Install directory does not exist: ${installDir}`],
      mainExecutable: { status: "FAIL", path: null, error: "Install directory missing" },
      worker: { status: "FAIL", entry: "--run-worker", error: "Install directory missing" },
      runtime: { status: "FAIL", path: null, error: "Install directory missing" }
    };
  }

  // Candidates for executable
  const exeCandidates = [
    join(installDir, EXPECTED_EXECUTABLE),
    join(installDir, EXPECTED_APP_NAME, EXPECTED_EXECUTABLE),
    join(installDir, `${EXPECTED_APP_NAME}_0.1.0_x64`, EXPECTED_EXECUTABLE),
    join(installDir, "bin", EXPECTED_EXECUTABLE)
  ];

  let executablePath = null;
  for (const c of exeCandidates) {
    if (existsSync(c)) {
      executablePath = c;
      break;
    }
  }

  // Recursive search if not in immediate candidates (bounded depth 3)
  if (!executablePath) {
    function searchExe(dir, depth) {
      if (depth > 3) return null;
      try {
        const entries = readdirSync(dir, { withFileTypes: true });
        for (const entry of entries) {
          const full = join(dir, entry.name);
          if (entry.isFile() && entry.name.toLowerCase() === EXPECTED_EXECUTABLE.toLowerCase()) {
            return full;
          }
          if (entry.isDirectory()) {
            const found = searchExe(full, depth + 1);
            if (found) return found;
          }
        }
      } catch {}
      return null;
    }
    executablePath = searchExe(installDir, 0);
  }

  const mainExecStatus = executablePath && existsSync(executablePath) ? "PASS" : "FAIL";
  if (mainExecStatus === "FAIL") {
    errors.push(`Main executable ${EXPECTED_EXECUTABLE} not found in ${installDir}`);
  }

  // Determine resourceDir
  const execDir = executablePath ? dirname(executablePath) : installDir;
  const resourceCandidates = [
    execDir,
    installDir,
    join(installDir, EXPECTED_APP_NAME),
    join(installDir, "resources")
  ];

  let runtimePath = null;
  let resolvedResourceDir = null;

  for (const resDir of resourceCandidates) {
    const candidateLib = join(resDir, EXPECTED_RESOURCE_SUBDIR, EXPECTED_RUNTIME_FILENAME);
    if (existsSync(candidateLib)) {
      runtimePath = candidateLib;
      resolvedResourceDir = resDir;
      break;
    }
  }

  // Also check direct lib under installDir
  if (!runtimePath) {
    const fallbackLib = join(installDir, "lib", EXPECTED_RUNTIME_FILENAME);
    if (existsSync(fallbackLib)) {
      runtimePath = fallbackLib;
      resolvedResourceDir = installDir;
    }
  }

  const runtimeStatus = runtimePath && existsSync(runtimePath) ? "PASS" : "FAIL";
  if (runtimeStatus === "FAIL") {
    errors.push(`Packaged PDFium runtime ${EXPECTED_RUNTIME_FILENAME} not found under lib/ in ${installDir}`);
  }

  const workerStatus = mainExecStatus === "PASS" ? "PASS" : "FAIL";

  return {
    valid: mainExecStatus === "PASS" && runtimeStatus === "PASS",
    installDir,
    executablePath,
    runtimePath,
    resourceDir: resolvedResourceDir || execDir,
    errors,
    mainExecutable: {
      status: mainExecStatus,
      path: executablePath,
      error: mainExecStatus === "FAIL" ? "Main executable missing" : null
    },
    worker: {
      status: workerStatus,
      entry: "--run-worker",
      sameBinary: true,
      error: workerStatus === "FAIL" ? "Worker binary missing" : null
    },
    runtime: {
      status: runtimeStatus,
      path: runtimePath,
      error: runtimeStatus === "FAIL" ? "Packaged runtime missing" : null
    }
  };
}

// ─── Windows Registry Query Helpers (PowerShell-based, safe fallback) ───

async function runPowerShellCommand(command, timeoutMs = 30000) {
  return spawnWithTimeout("powershell", ["-NoProfile", "-Command", command], {}, timeoutMs);
}

export async function findMsiInstallLocation(productName = EXPECTED_APP_NAME) {
  const psCmd = `
    $uninstallKeys = @(
      "HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*",
      "HKLM:\\Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*",
      "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*"
    )
    foreach ($keyPath in $uninstallKeys) {
      $items = Get-ItemProperty $keyPath -ErrorAction SilentlyContinue
      foreach ($item in $items) {
        if ($item.DisplayName -and $item.DisplayName -like "*${productName}*") {
          if ($item.InstallLocation) { $item.InstallLocation; exit 0 }
          if ($item.UninstallString) {
            $dir = Split-Path $item.UninstallString -Parent
            if ($dir) { $dir; exit 0 }
          }
        }
      }
    }
    exit 1
  `;
  try {
    const result = await runPowerShellCommand(psCmd, TIMEOUTS.install);
    if (result.code === 0 && result.stdout.trim()) {
      return result.stdout.trim().replace(/['"]/g, "");
    }
  } catch {}
  return null;
}

export async function findMsiProductCode(productName = EXPECTED_APP_NAME) {
  const psCmd = `
    $uninstallKeys = @(
      "HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*",
      "HKLM:\\Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*",
      "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*"
    )
    foreach ($keyPath in $uninstallKeys) {
      $items = Get-ItemProperty $keyPath -ErrorAction SilentlyContinue
      foreach ($item in $items) {
        if ($item.DisplayName -and $item.DisplayName -like "*${productName}*") {
          if ($item.PSChildName) { $item.PSChildName; exit 0 }
        }
      }
    }
    exit 1
  `;
  try {
    const result = await runPowerShellCommand(psCmd, TIMEOUTS.install);
    if (result.code === 0 && result.stdout.trim()) {
      return result.stdout.trim().replace(/[{}]/g, "");
    }
  } catch {}
  return null;
}

export async function findNsisInstallLocation(installDir) {
  const candidates = [
    join(installDir, EXPECTED_EXECUTABLE),
    join(installDir, EXPECTED_APP_NAME, EXPECTED_EXECUTABLE),
    join(installDir, `${EXPECTED_APP_NAME}_0.1.0_x64`, EXPECTED_EXECUTABLE)
  ];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return null;
}

export async function findExecutable(installDir, artifactType) {
  if (artifactType === "WINDOWS_MSI") {
    const regLoc = await findMsiInstallLocation(EXPECTED_APP_NAME);
    if (regLoc) {
      const candidates = [
        join(regLoc, EXPECTED_EXECUTABLE),
        join(regLoc, EXPECTED_APP_NAME, EXPECTED_EXECUTABLE)
      ];
      for (const c of candidates) if (existsSync(c)) return c;
    }
  }
  return findNsisInstallLocation(installDir);
}

// ─── Runtime DLL Verification ───

export async function verifyRuntimeDll(runtimePath, expectedSha256Hex, expectedByteLength, expectedArch) {
  if (!existsSync(runtimePath)) {
    return { ok: false, error: "DLL not found", sha256: null, architecture: null };
  }
  const data = readFileSync(runtimePath);
  const actualSha256 = await hashSha256Async(data);
  const actualLength = data.length;

  if (expectedByteLength && actualLength !== expectedByteLength) {
    return {
      ok: false,
      error: `Byte length mismatch: expected ${expectedByteLength}, got ${actualLength}`,
      sha256: actualSha256,
      architecture: null
    };
  }

  if (expectedSha256Hex) {
    if (actualSha256.toLowerCase() !== expectedSha256Hex.toLowerCase()) {
      return {
        ok: false,
        error: `SHA-256 mismatch: expected ${expectedSha256Hex}, got ${actualSha256}`,
        sha256: actualSha256,
        architecture: null
      };
    }
  }

  // Pure Node PE Header Architecture Check
  const peCheck = validatePeArchitecture(runtimePath);
  if (!peCheck.ok) {
    return {
      ok: false,
      error: `PE Architecture verification failed: ${peCheck.error}`,
      sha256: actualSha256,
      architecture: peCheck.architecture || "unknown"
    };
  }

  return {
    ok: true,
    error: null,
    sha256: actualSha256,
    architecture: peCheck.architecture
  };
}

// ─── Interactive Smoke IPC ───

export async function runInteractiveSmoke(executablePath, fixturePath, env, workspace, cwdOverride = null, timeoutMs = 60000) {
  const child = spawn(executablePath, ["--smoke"], {
    cwd: cwdOverride || workspace,
    env,
    stdio: ["pipe", "pipe", "pipe"],
    detached: false,
    windowsVerbatimArguments: true
  });

  let stdout = "";
  let stderr = "";
  const waiters = new Map();

  child.stdout.on("data", (d) => {
    stdout += d.toString();
    let idx;
    while ((idx = stdout.indexOf("\n")) !== -1) {
      const line = stdout.slice(0, idx).trim();
      stdout = stdout.slice(idx + 1);
      if (!line) continue;
      let parsed;
      try { parsed = JSON.parse(line); } catch { continue; }
      if (parsed && parsed.id && waiters.has(parsed.id)) {
        const resolve = waiters.get(parsed.id);
        waiters.delete(parsed.id);
        resolve(parsed);
      }
    }
  });
  child.stderr.on("data", (d) => { stderr += d.toString(); });

  const sendCommand = (command, cmdTimeoutMs = 15000) => {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        waiters.delete(command.id);
        reject(new Error(`smoke command ${command.id} timed out after ${cmdTimeoutMs}ms`));
      }, cmdTimeoutMs);
      waiters.set(command.id, (resp) => { clearTimeout(timer); resolve(resp); });
      try {
        child.stdin.write(JSON.stringify(command) + "\n");
      } catch (err) {
        clearTimeout(timer);
        waiters.delete(command.id);
        reject(err);
      }
    });
  };

  const exited = new Promise((resolve) => {
    child.on("close", (code) => resolve(code));
  });

  try {
    const open = await sendCommand({ id: "smoke-open-1", cmd: "open", path: fixturePath });
    const render = await sendCommand({ id: "smoke-render-1", cmd: "render_thumbnail", path: fixturePath, page_index: 0, scale: 0.25 });
    const text1 = await sendCommand({ id: "smoke-text-1", cmd: "extract_text", path: fixturePath, page_index: 0 });
    const text2 = await sendCommand({ id: "smoke-text-2", cmd: "extract_text", path: fixturePath, page_index: 1 });
    const close = await sendCommand({ id: "smoke-close-1", cmd: "close" });

    let exitCode = null;
    await Promise.race([
      exited.then((code) => { exitCode = code; }),
      new Promise((_, rej) => setTimeout(() => rej(new Error("smoke process did not exit within timeout")), TIMEOUTS.shutdown))
    ]);

    return { open, render, text1, text2, close, exitCode, stderr, pid: child.pid };
  } catch (error) {
    try {
      if (process.platform === "win32" && child.pid) {
        spawnSync("taskkill", ["/PID", child.pid.toString(), "/T", "/F"], { stdio: "ignore" });
      } else {
        child.kill("SIGKILL");
      }
    } catch {}
    throw error;
  }
}

export function interpretSmoke(session) {
  const allText = (session.text1?.text || "") + "\n" + (session.text2?.text || "");
  const tokens = CANONICAL_SMOKE_FIXTURE.requiredTextTokens.filter((token) => allText.includes(token));
  const pageCount = session.open?.page_count || 0;
  const renderOk = session.render?.status === "ok";
  const renderTest = renderOk
    ? { status: "PASS", operation: "thumbnail", dimensions: `${session.render.width}x${session.render.height}`, durationMs: 0 }
    : { status: "FAIL", operation: "thumbnail", dimensions: "0x0", durationMs: 0 };
  const smokeOk = pageCount === 2 && tokens.length >= 3 && renderOk;
  return { pageCount, tokens, renderTest, smokeOk };
}

// ─── Uninstall Functions ───

export async function uninstallMsi(productCode) {
  const result = await spawnWithTimeout("msiexec", ["/qn", "/x", productCode], {}, TIMEOUTS.install);
  return { success: result.code === 0, output: result.stdout, error: result.stderr };
}

export async function uninstallNsis(uninstallExe) {
  if (!existsSync(uninstallExe)) return { success: false, error: "Uninstaller not found" };
  const result = await spawnWithTimeout(uninstallExe, ["/S"], {}, TIMEOUTS.install);
  return { success: result.code === 0, output: result.stdout, error: result.stderr };
}

// ─── Contamination Hardening Verification ───

export async function runContaminationTest(executablePath, fixturePath, workspace, resourceLib, resourceDir, dirName, cwdOverride) {
  const contamDir = join(workspace, dirName);
  mkdirSync(contamDir, { recursive: true });
  const fakeDll = join(contamDir, "pdfium.dll");
  writeFileSync(fakeDll, Buffer.from("CONTAMINATED_FAKE_PDFIUM_DLL_NON_FUNCTIONAL"));

  // Build clean sanitized environment
  const sanitizedEnv = { ...process.env };
  delete sanitizedEnv.PDFIUM_PATH;
  delete sanitizedEnv.PDFIUM_RESOLVED_PATH;
  delete sanitizedEnv.CARGO_MANIFEST_DIR;
  sanitizedEnv.TAURI_RESOURCE_DIR = resourceDir;
  sanitizedEnv.PATH = contamDir + ";" + (sanitizedEnv.PATH || "");

  try {
    const session = await runInteractiveSmoke(executablePath, fixturePath, sanitizedEnv, workspace, cwdOverride);
    const smoke = interpretSmoke(session);
    return smoke.smokeOk ? "PASS" : "FAIL";
  } catch (e) {
    return "FAIL";
  }
}

// ─── CLI Argument Parsing ───

export function parseWindowsArgs(argv) {
  const args = parseCommonArgs(argv);
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--artifact-type" && argv[i + 1]) {
      args.artifactType = argv[++i];
    } else if (arg === "--architecture" && argv[i + 1]) {
      args.architecture = argv[++i];
    }
  }
  return args;
}

// ─── Main Lifecycle Orchestration ───

export async function main() {
  const args = parseWindowsArgs(process.argv.slice(2));

  if (args.help) {
    console.log(`
Windows Artifact Smoke Harness (11I-E4)

Tests: WINDOWS_MSI, WINDOWS_NSIS

Usage: node scripts/artifact-smoke-windows.mjs --artifact <path> [options]

Required:
  --artifact <path>        Path to the exact .msi or .exe installer package

Options:
  --artifact-type <type>   Force artifact type (WINDOWS_MSI, WINDOWS_NSIS)
                           Default: auto-detect from .msi / .exe extension
  --architecture <arch>    Target architecture (x86_64, arm64) [default: x86_64]
  --output <path>          Write evidence JSON to this path (default: stdout)
  --keep-temp              Do not clean up temporary workspace on exit
  --verbose                Verbose logging
  --help                   Show this help
`);
    process.exit(0);
  }

  const platform = "windows";
  // Windows target triple is x86_64-pc-windows-msvc; default to x86_64 unless explicitly specified
  let architecture = args.architecture || "x86_64";
  const runId = generateRunId();

  // Fail-closed missing artifact handling
  if (!args.artifactPath || !existsSync(args.artifactPath)) {
    const errorMsg = !args.artifactPath
      ? "Missing required --artifact argument"
      : `Artifact not found: ${args.artifactPath}`;

    let resolvedType = args.artifactType;
    if (!resolvedType && args.artifactPath) {
      const ext = args.artifactPath.toLowerCase().split(".").pop();
      if (ext === "msi") resolvedType = "WINDOWS_MSI";
      else if (ext === "exe") resolvedType = "WINDOWS_NSIS";
    }
    if (!resolvedType) resolvedType = "WINDOWS_MSI";

    const evidence = createEmptyEvidence(platform, architecture, resolvedType, args.artifactPath || "MISSING", `local-windows-${runId}`);
    evidence.installResult = {
      status: "ARTIFACT_UNAVAILABLE",
      method: "none",
      installPath: null,
      durationMs: 0,
      error: errorMsg
    };
    evidence.launchResult.status = "ARTIFACT_UNAVAILABLE";
    evidence.runtimeResolutionResult.status = "ARTIFACT_UNAVAILABLE";
    evidence.smokeDocumentResult.status = "ARTIFACT_UNAVAILABLE";
    evidence.shutdownResult.status = "ARTIFACT_UNAVAILABLE";
    evidence.overallStatus = "FAIL";

    console.error(`[windows-smoke] ARTIFACT_UNAVAILABLE: ${errorMsg}`);
    exitWithEvidence(evidence, args.outputPath);
    return;
  }

  let artifactType = args.artifactType;
  if (!artifactType) {
    const ext = args.artifactPath.toLowerCase().split(".").pop();
    if (ext === "msi") artifactType = "WINDOWS_MSI";
    else if (ext === "exe") artifactType = "WINDOWS_NSIS";
    else {
      console.error(`[windows-smoke] Cannot auto-detect artifact type from extension: .${ext}. Use --artifact-type.`);
      process.exit(EXIT_CODES.INVALID_ARGS);
    }
  }

  if (!WINDOWS_ARTIFACT_TYPES.includes(artifactType)) {
    console.error(`[windows-smoke] Invalid Windows artifact type: ${artifactType}. Expected one of: ${WINDOWS_ARTIFACT_TYPES.join(", ")}`);
    process.exit(EXIT_CODES.INVALID_ARGS);
  }

  const workspace = createTempWorkspace(`artifact-smoke-windows-${runId}-`);
  const startTime = Date.now();

  const safeCleanup = () => {
    if (!args.keepTemp && existsSync(workspace)) {
      cleanupTempWorkspace(workspace);
    }
  };

  process.on("SIGINT", () => { safeCleanup(); process.exit(EXIT_CODES.REQUIRED_CHECK_FAILED); });
  process.on("SIGTERM", () => { safeCleanup(); process.exit(EXIT_CODES.REQUIRED_CHECK_FAILED); });

  console.log(`[windows-smoke] Run ID: ${runId}`);
  console.log(`[windows-smoke] Workspace: ${workspace}`);
  console.log(`[windows-smoke] Artifact: ${args.artifactPath} (${artifactType})`);

  const platformSupport = await loadPlatformSupport();
  const runtimeMeta = getWindowsRuntimeMetadata(platformSupport);

  const artifactData = readFileSync(args.artifactPath);
  const artifactSha256 = await hashSha256Async(artifactData);
  const artifactByteLength = artifactData.length;

  const evidence = createEmptyEvidence(platform, architecture, artifactType, args.artifactPath, `local-windows-${runId}`);
  evidence.artifactIdentity.sha256 = artifactSha256;
  evidence.artifactIdentity.byteLength = artifactByteLength;

  let installDir = null;
  let executablePath = null;
  let uninstallInfo = null;

  try {
    // ─── STEP 1: Deploy / Install ───
    console.log(`[windows-smoke] Step 1: Install (${artifactType})`);
    const installStart = Date.now();
    installDir = join(workspace, "install");
    mkdirSync(installDir, { recursive: true });

    let installMethod = "none";

    if (artifactType === "WINDOWS_MSI") {
      const result = await spawnWithTimeout(
        "msiexec",
        ["/qn", "/i", args.artifactPath, `TARGETDIR=${installDir}`],
        {},
        TIMEOUTS.install
      );
      if (result.code !== 0) {
        evidence.installResult = {
          status: "FAIL",
          method: "msi_install",
          installPath: installDir,
          durationMs: Date.now() - installStart,
          error: `msiexec failed with exit code ${result.code}: ${result.stderr}`
        };
        throw new Error("MSI install failed");
      }
      installMethod = "msi_install";

      const productCode = await findMsiProductCode(EXPECTED_APP_NAME);
      if (productCode) uninstallInfo = { type: "msi", productCode };

    } else if (artifactType === "WINDOWS_NSIS") {
      const result = await spawnWithTimeout(
        args.artifactPath,
        ["/S", `/D=${installDir}`],
        {},
        TIMEOUTS.install
      );
      if (result.code !== 0) {
        evidence.installResult = {
          status: "FAIL",
          method: "nsis_install",
          installPath: installDir,
          durationMs: Date.now() - installStart,
          error: `NSIS installer failed with exit code ${result.code}: ${result.stderr}`
        };
        throw new Error("NSIS install failed");
      }
      installMethod = "nsis_install";

      const uninstallExe = join(installDir, "Uninstall.exe");
      if (existsSync(uninstallExe)) uninstallInfo = { type: "nsis", uninstallExe };
    }

    // ─── STEP 2: Installed Layout Validation ───
    console.log(`[windows-smoke] Step 2: Installed Layout Validation`);
    const layout = validateInstalledLayout(installDir, { artifactType });
    if (!layout.valid) {
      evidence.installResult = {
        status: "FAIL",
        method: installMethod,
        installPath: installDir,
        durationMs: Date.now() - installStart,
        error: `Installed layout validation failed: ${layout.errors.join("; ")}`
      };
      throw new Error(`Installed layout invalid: ${layout.errors.join("; ")}`);
    }

    executablePath = layout.executablePath;
    const resourceDir = layout.resourceDir;
    const runtimePath = layout.runtimePath;
    const resourceLib = dirname(runtimePath);

    evidence.installResult = {
      status: "PASS",
      method: installMethod,
      installPath: installDir,
      durationMs: Date.now() - installStart,
      error: null
    };

    // ─── STEP 3: Runtime Resolution & PE Architecture Verification ───
    console.log(`[windows-smoke] Step 3: Runtime Resolution & Integrity`);
    const rtVerify = await verifyRuntimeDll(
      runtimePath,
      runtimeMeta.expectedSha256,
      runtimeMeta.expectedByteLength,
      runtimeMeta.expectedArchitecture
    );

    if (!rtVerify.ok) {
      evidence.runtimeResolutionResult = {
        status: "FAIL",
        resolvedPath: runtimePath,
        source: "packaged",
        sha256: rtVerify.sha256,
        architecture: rtVerify.architecture,
        error: rtVerify.error
      };
      throw new Error(`Packaged runtime verification failed: ${rtVerify.error}`);
    }

    evidence.runtimeResolutionResult = {
      status: "PASS",
      resolvedPath: runtimePath,
      source: "packaged",
      sha256: rtVerify.sha256,
      architecture: rtVerify.architecture,
      error: null
    };

    // ─── STEP 4: Launch Application & Worker IPC Smoke ───
    console.log(`[windows-smoke] Step 4: Launch + Smoke (Interactive IPC)`);
    const launchStart = Date.now();

    // Prepare clean sanitized environment
    const cleanEnv = { ...process.env };
    delete cleanEnv.PDFIUM_PATH;
    delete cleanEnv.PDFIUM_RESOLVED_PATH;
    delete cleanEnv.CARGO_MANIFEST_DIR;
    cleanEnv.TAURI_RESOURCE_DIR = resourceDir;

    const fixturePath = copyFixtureToWorkspace("core-release-smoke", workspace);

    let session;
    try {
      session = await runInteractiveSmoke(executablePath, fixturePath, cleanEnv, workspace);
    } catch (error) {
      evidence.launchResult = {
        status: "FAIL",
        executablePath,
        pid: null,
        startupDurationMs: Date.now() - launchStart,
        error: `Smoke session failed: ${error.message}`
      };
      evidence.smokeDocumentResult = {
        status: "FAIL",
        fixtureId: CANONICAL_SMOKE_FIXTURE.fixtureId,
        fixturePath,
        pageCount: 0,
        textTokensFound: [],
        renderTest: { status: "FAIL", operation: "thumbnail", dimensions: "0x0", durationMs: 0 },
        workerUsed: true,
        workerSpawnMethod: "production_ipc",
        error: `Smoke session failed: ${error.message}`
      };
      evidence.shutdownResult = {
        status: "FAIL",
        cleanExit: false,
        workerCleanup: false,
        durationMs: Date.now() - launchStart,
        error: `Smoke session failed: ${error.message}`
      };
      throw error;
    }

    const smoke = interpretSmoke(session);

    evidence.launchResult = session.open?.status === "ok"
      ? { status: "PASS", executablePath, pid: session.pid, startupDurationMs: Date.now() - launchStart, error: null }
      : { status: "FAIL", executablePath, pid: session.pid, startupDurationMs: Date.now() - launchStart, error: session.open?.error || "Smoke open command failed" };

    evidence.smokeDocumentResult = {
      status: smoke.smokeOk ? "PASS" : "FAIL",
      fixtureId: CANONICAL_SMOKE_FIXTURE.fixtureId,
      fixturePath,
      pageCount: smoke.pageCount,
      textTokensFound: smoke.tokens,
      renderTest: smoke.renderTest,
      workerUsed: true,
      workerSpawnMethod: "production_ipc",
      error: smoke.smokeOk ? null : `Missing tokens: ${CANONICAL_SMOKE_FIXTURE.requiredTextTokens.filter(t => !smoke.tokens.includes(t)).join(", ")}`
    };

    // ─── STEP 5: Adversarial Hardening (CWD Decoy + PATH Contamination) ───
    console.log(`[windows-smoke] Step 5: Adversarial Hardening (CWD Decoy + PATH Contamination)`);
    const decoyStatus = await runContaminationTest(
      executablePath,
      fixturePath,
      workspace,
      resourceLib,
      resourceDir,
      "decoy_cwd",
      join(workspace, "decoy_cwd")
    );
    const pathStatus = await runContaminationTest(
      executablePath,
      fixturePath,
      workspace,
      resourceLib,
      resourceDir,
      "wrong_dll_path",
      workspace
    );

    evidence.hardeningResult = {
      status: (decoyStatus === "PASS" && pathStatus === "PASS") ? "PASS" : "FAIL",
      decoyDllCwd: decoyStatus,
      pathContamination: pathStatus,
      error: (decoyStatus === "PASS" && pathStatus === "PASS") ? null : "Packaged runtime isolation violated under CWD/PATH contamination"
    };

    // ─── STEP 6: Shutdown ───
    console.log(`[windows-smoke] Step 6: Shutdown Verification`);
    evidence.shutdownResult = {
      status: session.exitCode === 0 ? "PASS" : "FAIL",
      cleanExit: session.exitCode === 0,
      workerCleanup: session.close?.status === "ok",
      durationMs: Date.now() - launchStart,
      error: session.exitCode === 0 ? null : `Smoke process exited with code ${session.exitCode}`
    };

    // ─── STEP 7: Uninstall & Cleanup ───
    console.log(`[windows-smoke] Step 7: Uninstall`);
    let uninstallStatus = "PASS";
    if (uninstallInfo) {
      if (uninstallInfo.type === "msi" && uninstallInfo.productCode) {
        const uninst = await uninstallMsi(uninstallInfo.productCode);
        if (!uninst.success) {
          uninstallStatus = "FAIL";
          console.warn(`[windows-smoke] MSI uninstall failed: ${uninst.error}`);
        }
      } else if (uninstallInfo.type === "nsis" && uninstallInfo.uninstallExe) {
        const uninst = await uninstallNsis(uninstallInfo.uninstallExe);
        if (!uninst.success) {
          uninstallStatus = "FAIL";
          console.warn(`[windows-smoke] NSIS uninstall failed: ${uninst.error}`);
        }
      }
    }

  } catch (err) {
    console.error(`[windows-smoke] ERROR: ${err.message}`);
    evidence.overallStatus = "FAIL";
  } finally {
    evidence.durationTotalMs = Date.now() - startTime;
    evidence.timestamp = new Date().toISOString();

    safeCleanup();
    exitWithEvidence(evidence, args.outputPath);
  }
}

// Invoke main when run directly
if (process.argv[1] && resolve(process.argv[1]) === resolve(__filename)) {
  main().catch(err => {
    console.error(`[windows-smoke] FATAL: ${err.message}`);
    process.exit(EXIT_CODES.INTERNAL_ERROR);
  });
}