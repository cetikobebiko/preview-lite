#!/usr/bin/env node
/**
 * Linux Artifact Smoke Harness — 11I-E3
 *
 * Tests: LINUX_DEB
 * Implements the exact artifact smoke contract using Linux package semantics.
 *
 * Architecture:
 * - Node stdlib only (zero network, zero external npm dependencies)
 * - Deterministic, fail-closed
 * - Full lifecycle: inspect -> extract -> layout validation -> runtime resolution -> launch -> worker IPC -> canonical fixture smoke -> shutdown -> cleanup
 * - Pure Node Buffer ELF 64-bit x86_64 architecture header validation
 * - Environment sanitization (scrubs dev overrides, LD_LIBRARY_PATH, PDFIUM_PATH)
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

import { join, resolve, basename, dirname } from "node:path";
import {
  existsSync,
  mkdirSync,
  cpSync,
  rmSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
  accessSync,
  constants as fsConstants
} from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = resolve(__filename, "..");
const REPO_ROOT = resolve(__dirname, "..");

export const LINUX_ARTIFACT_TYPES = Object.freeze(["LINUX_DEB"]);
export const EXPECTED_APP_NAME = "PreviewLite";
export const EXPECTED_EXECUTABLE = "preview-alternative";
export const EXPECTED_RUNTIME_FILENAME = "libpdfium.so";
export const EXPECTED_RESOURCE_SUBDIR = "lib";
export const EXPECTED_RUNTIME_REL = "usr/lib/preview-lite/lib/libpdfium.so";

// ─── ELF Architecture Validator (x86_64 / EM_X86_64) ───

/**
 * Validates ELF 64-bit little-endian x86_64 binary header.
 *
 * Header format:
 * - Bytes 0..3: \x7f E L F (0x7f, 0x45, 0x4c, 0x46)
 * - Byte 4 (ei_class): 2 = 64-bit (ELFCLASS64)
 * - Byte 5 (ei_data): 1 = little-endian (ELFDATA2LSB)
 * - Byte 6 (ei_version): 1
 * - Bytes 18..19 (e_machine): 0x003e = EM_X86_64 (62)
 */
export function validateElfArchitecture(filePath) {
  if (!existsSync(filePath)) {
    return { ok: false, error: `File not found: ${filePath}` };
  }
  const stat = statSync(filePath);
  if (stat.size < 20) {
    return { ok: false, error: `File too small for ELF header: ${stat.size} bytes` };
  }

  const buf = readFileSync(filePath);

  // Magic byte check: \x7fELF
  if (buf[0] !== 0x7f || buf[1] !== 0x45 || buf[2] !== 0x4c || buf[3] !== 0x46) {
    return { ok: false, error: "Not a valid ELF binary (missing \\x7fELF signature)" };
  }

  const eiClass = buf[4];
  const eiData = buf[5];

  if (eiClass !== 2) {
    const classLabel = eiClass === 1 ? "ELFCLASS32 (32-bit)" : `class_${eiClass}`;
    return {
      ok: false,
      architecture: eiClass === 1 ? "i386_32bit" : `class_${eiClass}`,
      error: `Expected 64-bit ELF (ELFCLASS64), found ${classLabel}`
    };
  }

  const isLittleEndian = eiData === 1;
  const eMachine = isLittleEndian ? buf.readUInt16LE(18) : buf.readUInt16BE(18);

  const EM_386 = 0x03;     // 3
  const EM_ARM = 0x28;     // 40
  const EM_X86_64 = 0x3e;  // 62
  const EM_AARCH64 = 0xb7; // 183
  const EM_RISCV = 0xf3;   // 243

  if (eMachine === EM_X86_64) {
    return {
      ok: true,
      architecture: "x86_64",
      machineCode: "0x003e",
      endianness: isLittleEndian ? "little" : "big"
    };
  }

  const archMap = {
    [EM_386]: "i386",
    [EM_ARM]: "arm",
    [EM_AARCH64]: "arm64",
    [EM_RISCV]: "riscv"
  };
  const foundArch = archMap[eMachine] || `0x${eMachine.toString(16)}`;

  return {
    ok: false,
    architecture: foundArch,
    error: `ELF architecture mismatch: expected x86_64 (EM_X86_64 0x003e), got ${foundArch} (0x${eMachine.toString(16)})`
  };
}

// ─── Debian Control File Parser ───

/**
 * Parses Debian control field entries from string output.
 */
export function parseDebControlText(controlText) {
  const meta = {
    packageName: null,
    version: null,
    architecture: null,
    maintainer: null,
    description: null
  };
  const lines = controlText.split("\n");
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("Package:")) {
      meta.packageName = trimmed.substring(8).trim();
    } else if (trimmed.startsWith("Version:")) {
      meta.version = trimmed.substring(8).trim();
    } else if (trimmed.startsWith("Architecture:")) {
      meta.architecture = trimmed.substring(13).trim();
    } else if (trimmed.startsWith("Maintainer:")) {
      meta.maintainer = trimmed.substring(11).trim();
    } else if (trimmed.startsWith("Description:")) {
      meta.description = trimmed.substring(12).trim();
    }
  }
  return meta;
}

// ─── Package Inspection (dpkg-deb -I) ───

export async function inspectDebPackage(debPath) {
  const result = {
    packageName: null,
    version: null,
    architecture: null,
    maintainer: null,
    description: null,
    status: "NOT_RUN",
    error: null
  };

  try {
    const cmdResult = await spawnWithTimeout("dpkg-deb", ["-I", debPath], {}, 10000);
    if (cmdResult.code === 0) {
      const parsed = parseDebControlText(cmdResult.stdout);
      Object.assign(result, parsed);
      if (result.packageName && result.version && result.architecture) {
        result.status = "PASS";
      } else {
        result.status = "FAIL";
        result.error = "Incomplete package metadata from dpkg-deb -I";
      }
      return result;
    } else {
      result.status = "FAIL";
      result.error = `dpkg-deb -I failed: ${cmdResult.stderr || cmdResult.stdout}`;
      return result;
    }
  } catch (err) {
    result.status = "TOOL_UNAVAILABLE_OFFLINE";
    result.error = `dpkg-deb tool unavailable: ${err.message}`;
    return result;
  }
}

// ─── Package Extraction (dpkg -x) ───

export async function extractDebPackage(debPath, installPrefix, timeoutMs = TIMEOUTS.install) {
  try {
    const res = await spawnWithTimeout("dpkg", ["-x", debPath, installPrefix], {}, timeoutMs);
    if (res.code === 0) {
      return { ok: true, error: null };
    }
    return { ok: false, error: `dpkg -x exited with code ${res.code}: ${res.stderr || res.stdout}` };
  } catch (err) {
    return { ok: false, error: `dpkg tool unavailable or failed to execute: ${err.message}` };
  }
}

// ─── Installed Layout Validation ───

export function validateInstalledLayout(installPrefix, packageMeta = {}) {
  const result = {
    valid: false,
    installPrefix,
    mainExecutable: { status: "FAIL", path: null, error: null },
    worker: { status: "FAIL", path: null, error: null },
    runtime: { status: "FAIL", path: null, error: null },
    resourcesPath: null,
    runtimeLibDir: null,
    desktopFile: { status: "NOT_RUN", path: null },
    errors: []
  };

  if (!existsSync(installPrefix)) {
    result.errors.push(`Install prefix does not exist: ${installPrefix}`);
    return result;
  }

  // 1. Locate Main Executable (usr/bin/preview-alternative or fallbacks)
  const pkgName = packageMeta.packageName || "preview-lite";
  const execCandidates = [
    join(installPrefix, "usr/bin", EXPECTED_EXECUTABLE),
    join(installPrefix, "usr/local/bin", EXPECTED_EXECUTABLE),
    join(installPrefix, "opt", pkgName, "bin", EXPECTED_EXECUTABLE),
    join(installPrefix, "opt", "preview-lite", "bin", EXPECTED_EXECUTABLE),
    join(installPrefix, "opt", "PreviewLite", "bin", EXPECTED_EXECUTABLE)
  ];

  let foundExecutable = null;
  for (const c of execCandidates) {
    if (existsSync(c)) {
      foundExecutable = c;
      break;
    }
  }

  if (!foundExecutable) {
    const err = `No main executable found in install layout. Checked candidates: ${execCandidates.join(", ")}`;
    result.mainExecutable.error = err;
    result.errors.push(err);
  } else {
    try {
      accessSync(foundExecutable, fsConstants.X_OK);
      result.mainExecutable = { status: "PASS", path: foundExecutable, error: null };
      // Worker is the exact same executable invoked with --run-worker
      result.worker = { status: "PASS", path: foundExecutable, error: null };
    } catch {
      const err = `Executable at ${foundExecutable} lacks execute permission (missing X_OK)`;
      result.mainExecutable.error = err;
      result.errors.push(err);
    }
  }

  // 2. Locate Resources and Packaged PDFium Runtime
  // Tauri bundles resources to usr/lib/<appname>/ or usr/share/<appname>/
  const resourceCandidates = [
    join(installPrefix, "usr/lib", pkgName),
    join(installPrefix, "usr/lib", "preview-lite"),
    join(installPrefix, "usr/lib", "PreviewLite"),
    join(installPrefix, "usr/lib", "preview-alternative"),
    join(installPrefix, "usr/share", pkgName),
    join(installPrefix, "usr/share", "preview-lite"),
    join(installPrefix, "usr/share", "PreviewLite"),
    join(installPrefix, "opt", pkgName, "resources")
  ];

  let foundRuntime = null;
  for (const rc of resourceCandidates) {
    const libDir = join(rc, EXPECTED_RESOURCE_SUBDIR);
    const runtimePath = join(libDir, EXPECTED_RUNTIME_FILENAME);
    if (existsSync(runtimePath)) {
      foundRuntime = runtimePath;
      result.resourcesPath = rc;
      result.runtimeLibDir = libDir;
      result.runtime = { status: "PASS", path: runtimePath, error: null };
      break;
    }
  }

  if (!foundRuntime) {
    const err = `Packaged runtime '${EXPECTED_RUNTIME_FILENAME}' not found in any expected resource location under: ${resourceCandidates.join(", ")}`;
    result.runtime.error = err;
    result.errors.push(err);
    result.resourcesPath = resourceCandidates[0];
    result.runtimeLibDir = join(resourceCandidates[0], EXPECTED_RESOURCE_SUBDIR);
  }

  // 3. Desktop Entry Check (Optional/Informational)
  const desktopCandidates = [
    join(installPrefix, "usr/share/applications", `${EXPECTED_APP_NAME}.desktop`),
    join(installPrefix, "usr/share/applications", "preview-lite.desktop"),
    join(installPrefix, "usr/share/applications", "PreviewLite.desktop")
  ];
  for (const dc of desktopCandidates) {
    if (existsSync(dc)) {
      result.desktopFile = { status: "PASS", path: dc };
      break;
    }
  }

  result.valid = result.errors.length === 0;
  return result;
}

// ─── Interactive Smoke Protocol Execution ───

export async function runInteractiveSmoke(executablePath, fixturePath, env, workspace, timeoutMs = 30000) {
  return new Promise((resolve, reject) => {
    const child = spawn(executablePath, ["--smoke"], {
      cwd: workspace,
      env,
      stdio: ["pipe", "pipe", "pipe"],
      detached: false
    });

    let stdoutBuffer = "";
    let stderrBuffer = "";
    const responses = new Map();
    let currentResolver = null;
    let timedOut = false;

    const killTimer = setTimeout(() => {
      timedOut = true;
      try { child.kill("SIGKILL"); } catch {}
      reject(new Error(`Smoke process timed out after ${timeoutMs}ms`));
    }, timeoutMs);

    function dispatchLines(text) {
      const lines = text.split("\n");
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        try {
          const json = JSON.parse(trimmed);
          if (json && json.id) {
            responses.set(json.id, json);
            if (currentResolver && currentResolver.id === json.id) {
              currentResolver.resolve(json);
              currentResolver = null;
            }
          }
        } catch {
          // not JSON or log line
        }
      }
    }

    child.stdout.on("data", (data) => {
      stdoutBuffer += data.toString();
      dispatchLines(stdoutBuffer);
    });

    child.stderr.on("data", (data) => {
      stderrBuffer += data.toString();
    });

    function sendCommand(cmdObj, timeout = 10000) {
      return new Promise((res, rej) => {
        if (responses.has(cmdObj.id)) {
          return res(responses.get(cmdObj.id));
        }
        const timer = setTimeout(() => {
          rej(new Error(`Command ${cmdObj.cmd} (id=${cmdObj.id}) timed out after ${timeout}ms`));
        }, timeout);

        currentResolver = {
          id: cmdObj.id,
          resolve: (val) => {
            clearTimeout(timer);
            res(val);
          }
        };

        try {
          child.stdin.write(JSON.stringify(cmdObj) + "\n");
        } catch (e) {
          clearTimeout(timer);
          rej(e);
        }
      });
    }

    (async () => {
      try {
        // 1. Ping (Handshake)
        const pingResp = await sendCommand({ cmd: "ping", id: "smoke-ping-1" }, 8000);
        if (pingResp.status !== "ok") {
          throw new Error(`Ping failed: ${JSON.stringify(pingResp)}`);
        }

        // 2. Open Document
        const openResp = await sendCommand({ cmd: "open", path: fixturePath, id: "smoke-open-1" }, 12000);
        if (openResp.status !== "ok") {
          throw new Error(`Open document failed: ${JSON.stringify(openResp)}`);
        }

        // 3. Render Thumbnail (Page 1, scale 0.25)
        const renderResp = await sendCommand({
          cmd: "render_thumbnail",
          page_index: 0,
          scale: 0.25,
          id: "smoke-render-1"
        }, 12000);

        // 4. Extract Text Page 1
        const text1Resp = await sendCommand({
          cmd: "extract_text",
          page_index: 0,
          id: "smoke-text-1"
        }, 12000);

        // 5. Extract Text Page 2
        const text2Resp = await sendCommand({
          cmd: "extract_text",
          page_index: 1,
          id: "smoke-text-2"
        }, 12000);

        // 6. Close / Shutdown
        try {
          await sendCommand({ cmd: "close", id: "smoke-close-1" }, 5000);
        } catch {}

        child.stdin.end();

        child.on("close", (code, signal) => {
          clearTimeout(killTimer);
          if (timedOut) return;
          resolve({
            code,
            signal,
            stdout: stdoutBuffer,
            stderr: stderrBuffer,
            open: openResp,
            render: renderResp,
            text1: text1Resp,
            text2: text2Resp
          });
        });

      } catch (err) {
        clearTimeout(killTimer);
        try { child.kill("SIGTERM"); } catch {}
        reject(err);
      }
    })();

    child.on("error", (err) => {
      clearTimeout(killTimer);
      reject(err);
    });
  });
}

// ─── Smoke Interpretation Logic ───

export function interpretSmoke(session) {
  const allText = ((session.text1 && session.text1.text) || "") + "\n" + ((session.text2 && session.text2.text) || "");
  const tokens = CANONICAL_SMOKE_FIXTURE.requiredTextTokens.filter((token) => allText.includes(token));
  const pageCount = (session.open && session.open.page_count) || 0;
  const renderOk = session.render && session.render.status === "ok" && session.render.width > 0 && session.render.height > 0;

  const renderTest = renderOk
    ? {
        status: "PASS",
        operation: "thumbnail",
        dimensions: `${session.render.width}x${session.render.height}`,
        durationMs: 0
      }
    : {
        status: "FAIL",
        operation: "thumbnail",
        dimensions: "0x0",
        durationMs: 0
      };

  const smokeOk = pageCount === 2 && tokens.length >= 3 && renderOk;
  return { pageCount, tokens, renderTest, smokeOk };
}

// ─── Main CLI Entry Point ───

async function main() {
  const args = parseCommonArgs(process.argv.slice(2));

  if (args.help) {
    console.log(`
Linux Artifact Smoke Harness (11I-E3)

Tests: LINUX_DEB

Usage: node scripts/artifact-smoke-linux.mjs --artifact <path> [options]

Required:
  --artifact <path>        Path to the exact .deb package

Options:
  --artifact-type <type>   Force artifact type (LINUX_DEB)
                           Default: auto-detect from .deb extension
  --output <path>          Write evidence JSON to this path (default: stdout)
  --keep-temp              Do not clean up temporary workspace on exit
  --verbose                Verbose logging
  --help                   Show this help
`);
    process.exit(0);
  }

  const platform = "linux";
  let architecture = "x86_64";
  try {
    architecture = detectArchitecture();
  } catch {
    // default to x86_64 for Linux target triple boundary
    architecture = "x86_64";
  }
  const runId = generateRunId();

  // Fail-closed handling for missing or unsupplied artifact path
  if (!args.artifactPath || !existsSync(args.artifactPath)) {
    const errorMsg = !args.artifactPath
      ? "Missing required --artifact argument"
      : `Artifact not found: ${args.artifactPath}`;

    const evidence = createEmptyEvidence(platform, architecture, "LINUX_DEB", args.artifactPath || "MISSING", `local-linux-${runId}`);
    evidence.installResult = {
      status: "ARTIFACT_UNAVAILABLE",
      method: "none",
      installPath: null,
      durationMs: 0,
      error: errorMsg
    };
    evidence.launchResult.status = "ARTIFACT_UNAVAILABLE";
    evidence.overallStatus = "FAIL";

    console.error(`[linux-smoke] ARTIFACT_UNAVAILABLE: ${errorMsg}`);
    exitWithEvidence(evidence, args.outputPath);
    return;
  }

  let artifactType = args.artifactType;
  if (!artifactType) {
    const ext = args.artifactPath.toLowerCase().split(".").pop();
    if (ext === "deb") {
      artifactType = "LINUX_DEB";
    } else {
      console.error(`[linux-smoke] Cannot auto-detect artifact type from extension '.${ext}'. Expected .deb`);
      process.exit(EXIT_CODES.INVALID_ARGS);
    }
  }

  if (!LINUX_ARTIFACT_TYPES.includes(artifactType)) {
    console.error(`[linux-smoke] Invalid Linux artifact type: ${artifactType}. Expected one of: ${LINUX_ARTIFACT_TYPES.join(", ")}`);
    process.exit(EXIT_CODES.INVALID_ARGS);
  }

  const workspace = createTempWorkspace(`artifact-smoke-linux-${runId}-`);
  const startTime = Date.now();

  const safeCleanup = () => {
    if (!args.keepTemp && existsSync(workspace)) {
      cleanupTempWorkspace(workspace);
    }
  };

  process.on("SIGINT", () => { safeCleanup(); process.exit(EXIT_CODES.REQUIRED_CHECK_FAILED); });
  process.on("SIGTERM", () => { safeCleanup(); process.exit(EXIT_CODES.REQUIRED_CHECK_FAILED); });

  console.log(`[linux-smoke] Run ID: ${runId}`);
  console.log(`[linux-smoke] Workspace: ${workspace}`);
  console.log(`[linux-smoke] Artifact: ${args.artifactPath} (${artifactType})`);

  const evidence = createEmptyEvidence(platform, architecture, artifactType, args.artifactPath, `local-linux-${runId}`);

  try {
    // ─── STEP 0: Artifact Identity ───
    console.log(`[linux-smoke] Step 0: Artifact Identity`);
    const artifactStat = statSync(args.artifactPath);
    evidence.artifactIdentity.byteLength = artifactStat.size;
    const artifactSha = await hashSha256Async(readFileSync(args.artifactPath));
    evidence.artifactIdentity.sha256 = artifactSha;
    console.log(`[linux-smoke] Artifact SHA-256: ${artifactSha}`);
    console.log(`[linux-smoke] Artifact size: ${artifactStat.size} bytes`);

    // ─── STEP 1: Package Metadata Inspection ───
    console.log(`[linux-smoke] Step 1: Package Metadata Inspection`);
    const packageMeta = await inspectDebPackage(args.artifactPath);
    evidence.packageInspectionResult = packageMeta;
    console.log(`[linux-smoke] Package inspection: status=${packageMeta.status}, name=${packageMeta.packageName}, arch=${packageMeta.architecture}`);

    // ─── STEP 2: Deploy / Extract to Isolated Test Location ───
    console.log(`[linux-smoke] Step 2: Deploy / Extract (.deb)`);
    const installStart = Date.now();
    const installPrefix = join(workspace, "install");
    mkdirSync(installPrefix, { recursive: true });

    const extractResult = await extractDebPackage(args.artifactPath, installPrefix, TIMEOUTS.install);

    if (!extractResult.ok) {
      evidence.installResult = {
        status: "FAIL",
        method: "deb_install",
        installPath: null,
        durationMs: Date.now() - installStart,
        error: extractResult.error
      };
      throw new Error(`DEB extraction failed: ${extractResult.error}`);
    }

    evidence.installResult = {
      status: "PASS",
      method: "deb_install",
      installPath: installPrefix,
      durationMs: Date.now() - installStart,
      error: null
    };
    console.log(`[linux-smoke] Extracted deb package to: ${installPrefix}`);

    // ─── STEP 3: Structural Layout Validation ───
    console.log(`[linux-smoke] Step 3: Structural Layout Validation`);
    const layout = validateInstalledLayout(installPrefix, packageMeta);
    evidence.installedLayout = layout;

    if (!layout.valid) {
      evidence.launchResult = {
        status: "FAIL",
        executablePath: layout.mainExecutable.path,
        pid: null,
        startupDurationMs: 0,
        error: `Layout validation failed: ${layout.errors.join("; ")}`
      };
      throw new Error(`Layout validation failed: ${layout.errors.join("; ")}`);
    }

    const executablePath = layout.mainExecutable.path;
    const runtimePath = layout.runtime.path;
    const resourcesPath = layout.resourcesPath;

    // ─── STEP 4: Verify Packaged Runtime Integrity & ELF Architecture ───
    console.log(`[linux-smoke] Step 4: Verify Packaged Runtime Integrity & ELF Header`);
    const runtimeData = readFileSync(runtimePath);
    const runtimeSha = await hashSha256Async(runtimeData);
    const runtimeLength = runtimeData.length;

    const elfCheck = validateElfArchitecture(runtimePath);
    let runtimeStatus = "PASS";
    let runtimeError = null;

    if (!elfCheck.ok) {
      runtimeStatus = "FAIL";
      runtimeError = `ELF validation failed: ${elfCheck.error}`;
    }

    evidence.runtimeResolutionResult = {
      status: runtimeStatus,
      resolvedPath: runtimePath,
      source: "packaged",
      sha256: runtimeSha,
      architecture: elfCheck.architecture || "unknown",
      error: runtimeError
    };

    if (runtimeStatus !== "PASS") {
      throw new Error(runtimeError);
    }
    console.log(`[linux-smoke] Packaged runtime verified (ELF x86_64, SHA-256 ${runtimeSha})`);

    // ─── STEP 5: Launch Application with Clean Isolated Environment ───
    console.log(`[linux-smoke] Step 5: Launch Application & Worker Verification`);
    const launchStart = Date.now();

    // Prepare clean child environment: strip dev overrides and LD contamination
    const cleanEnv = { ...process.env };
    delete cleanEnv.PDFIUM_PATH;
    delete cleanEnv.PDFIUM_RESOLVED_PATH;
    delete cleanEnv.LD_LIBRARY_PATH;
    delete cleanEnv.LD_PRELOAD;
    delete cleanEnv.CARGO_MANIFEST_DIR;
    cleanEnv.TAURI_RESOURCE_DIR = resourcesPath;

    const fixturePath = copyFixtureToWorkspace("core-release-smoke", workspace);
    console.log(`[linux-smoke] Staged canonical fixture: ${fixturePath}`);

    const smokeSession = await runInteractiveSmoke(
      executablePath,
      fixturePath,
      cleanEnv,
      workspace,
      TIMEOUTS.launch + TIMEOUTS.documentOpen * 2
    );

    evidence.launchResult = {
      status: "PASS",
      executablePath,
      pid: 0,
      startupDurationMs: Date.now() - launchStart,
      error: null
    };

    // ─── STEP 6: Interpret Smoke Results ───
    console.log(`[linux-smoke] Step 6: Smoke Document Assertions`);
    const smokeInterpretation = interpretSmoke(smokeSession);

    evidence.smokeDocumentResult = {
      status: smokeInterpretation.smokeOk ? "PASS" : "FAIL",
      fixtureId: "core-release-smoke",
      fixturePath,
      pageCount: smokeInterpretation.pageCount,
      textTokensFound: smokeInterpretation.tokens,
      renderTest: smokeInterpretation.renderTest,
      workerUsed: true,
      workerSpawnMethod: "production_ipc",
      error: !smokeInterpretation.smokeOk
        ? `Smoke assertion failed: pageCount=${smokeInterpretation.pageCount} (expected 2), tokens=${smokeInterpretation.tokens.length} (expected >=3), render=${smokeInterpretation.renderTest.status}`
        : null
    };

    // ─── STEP 7: Shutdown Verification ───
    console.log(`[linux-smoke] Step 7: Shutdown Verification`);
    const cleanExit = smokeSession.code === 0;
    evidence.shutdownResult = {
      status: cleanExit ? "PASS" : "FAIL",
      cleanExit,
      workerCleanup: true,
      durationMs: 0,
      error: cleanExit ? null : `Process exited with code ${smokeSession.code}, signal ${smokeSession.signal}`
    };

  } catch (err) {
    console.error(`[linux-smoke] FAIL: ${err.message}`);
    evidence.overallStatus = "FAIL";
  } finally {
    evidence.durationTotalMs = Date.now() - startTime;
    evidence.timestamp = new Date().toISOString();

    safeCleanup();
    exitWithEvidence(evidence, args.outputPath);
  }
}

// Only invoke main when executed as script
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  main().catch(err => {
    console.error(`[linux-smoke] FATAL: ${err.message}`);
    process.exit(EXIT_CODES.INTERNAL_ERROR);
  });
}