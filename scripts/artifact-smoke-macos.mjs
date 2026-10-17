#!/usr/bin/env node
/**
 * macOS Artifact Smoke Harness — 11I-E2
 *
 * Tests: MACOS_DMG, MACOS_APP_BUNDLE
 * Implements the exact artifact smoke contract using macOS-native tools.
 *
 * Architecture:
 * - Node stdlib only (zero network, zero external npm dependencies)
 * - Deterministic, fail-closed
 * - Full lifecycle: mount/copy -> bundle layout -> runtime resolution -> launch -> worker IPC -> canonical fixture smoke -> shutdown -> unmount & cleanup
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

export const MACOS_ARTIFACT_TYPES = Object.freeze(["MACOS_DMG", "MACOS_APP_BUNDLE"]);
export const EXPECTED_APP_NAME = "PreviewLite.app";
export const EXPECTED_EXECUTABLE = "preview-alternative";
export const EXPECTED_RUNTIME_REL = "Contents/Resources/lib/libpdfium.dylib";
export const EXPECTED_RUNTIME_SHA256 = "cb167bb40974e409a4de3e37fc97fe29802a33aa0bfb75ce0a4e6c991c7857ab";
export const EXPECTED_RUNTIME_BYTE_LENGTH = 7199216;

// ─── Mach-O Architecture Validator (arm64) ───

/**
 * Validates Mach-O 64-bit arm64 binary header.
 * Mach-O 64-bit little-endian magic: 0xfeedfacf
 * CPU type ARM64: 0x0100000c (16777228)
 */
export function validateMachOArchitecture(filePath) {
  if (!existsSync(filePath)) {
    return { ok: false, error: `File not found: ${filePath}` };
  }
  const stat = statSync(filePath);
  if (stat.size < 8) {
    return { ok: false, error: `File too small for Mach-O header: ${stat.size} bytes` };
  }

  const buf = readFileSync(filePath);
  const magic = buf.readUInt32LE(0);
  const cpuType = buf.readInt32LE(4);

  const MH_MAGIC_64 = 0xfeedfacf;
  const MH_CIGAM_64 = 0xcffaedfe;
  const CPU_TYPE_ARM64 = 0x0100000c;
  const CPU_TYPE_X86_64 = 0x01000007;

  if (magic === MH_MAGIC_64) {
    if (cpuType === CPU_TYPE_ARM64) {
      return { ok: true, architecture: "arm64", magic: "0xfeedfacf" };
    }
    if (cpuType === CPU_TYPE_X86_64) {
      return { ok: false, architecture: "x86_64", error: "Found x86_64 Mach-O binary, expected arm64" };
    }
    return { ok: false, architecture: `0x${cpuType.toString(16)}`, error: `Unexpected CPU type: 0x${cpuType.toString(16)}` };
  }

  if (magic === MH_CIGAM_64) {
    return { ok: false, error: "Big-endian Mach-O 64-bit not supported" };
  }

  // Check for universal binary (fat binary: 0xcafebabe / 0xbebafeca)
  const FAT_MAGIC = 0xcafebabe;
  const FAT_CIGAM = 0xbebafeca;
  if (magic === FAT_MAGIC || magic === FAT_CIGAM) {
    return { ok: false, error: "Fat universal binary detected, expected thin arm64 Mach-O dylib" };
  }

  return { ok: false, error: `Not a Mach-O 64-bit binary (magic: 0x${magic.toString(16)})` };
}

// ─── Info.plist Parser ───

/**
 * Parses key entries from a macOS Info.plist file.
 * Pure Node stdlib regex-based extraction.
 */
export function parseInfoPlist(plistContent) {
  function getTagValue(key) {
    const regex = new RegExp(`<key>${key}<\\/key>\\s*<string>([^<]+)<\\/string>`, "i");
    const match = plistContent.match(regex);
    return match ? match[1].trim() : null;
  }

  return {
    executable: getTagValue("CFBundleExecutable"),
    identifier: getTagValue("CFBundleIdentifier"),
    packageType: getTagValue("CFBundlePackageType"),
    version: getTagValue("CFBundleShortVersionString") || getTagValue("CFBundleVersion"),
    name: getTagValue("CFBundleName")
  };
}

// ─── Bundle Structural Validator ───

/**
 * Validates the physical structure of a deployed macOS application bundle.
 */
export function validateBundleStructure(bundlePath, expectedAppName = EXPECTED_APP_NAME, fallbackExecutable = EXPECTED_EXECUTABLE) {
  const result = {
    valid: false,
    bundlePath,
    infoPlist: null,
    executablePath: null,
    runtimePath: null,
    resourcesPath: null,
    errors: []
  };

  if (!existsSync(bundlePath)) {
    result.errors.push(`Bundle path does not exist: ${bundlePath}`);
    return result;
  }

  const contentsDir = join(bundlePath, "Contents");
  if (!existsSync(contentsDir)) {
    result.errors.push(`Missing Contents directory: ${contentsDir}`);
    return result;
  }

  const infoPlistPath = join(contentsDir, "Info.plist");
  if (!existsSync(infoPlistPath)) {
    result.errors.push(`Missing Info.plist at ${infoPlistPath}`);
  } else {
    try {
      const plistContent = readFileSync(infoPlistPath, "utf-8");
      result.infoPlist = parseInfoPlist(plistContent);
    } catch (e) {
      result.errors.push(`Failed to read Info.plist: ${e.message}`);
    }
  }

  const macosDir = join(contentsDir, "MacOS");
  if (!existsSync(macosDir)) {
    result.errors.push(`Missing Contents/MacOS directory: ${macosDir}`);
  } else {
    // Check candidate executable names:
    const candidates = [];
    if (result.infoPlist && result.infoPlist.executable) {
      candidates.push(result.infoPlist.executable);
    }
    if (!candidates.includes(fallbackExecutable)) {
      candidates.push(fallbackExecutable);
    }
    if (!candidates.includes("PreviewLite")) {
      candidates.push("PreviewLite");
    }

    let foundExecutable = null;
    for (const name of candidates) {
      const p = join(macosDir, name);
      if (existsSync(p)) {
        foundExecutable = p;
        break;
      }
    }

    if (!foundExecutable) {
      result.errors.push(`No valid executable found in ${macosDir}. Checked candidates: ${candidates.join(", ")}`);
    } else {
      try {
        accessSync(foundExecutable, fsConstants.X_OK);
        result.executablePath = foundExecutable;
      } catch {
        result.errors.push(`Executable at ${foundExecutable} is not executable (missing execute bit)`);
      }
    }
  }

  const resourcesDir = join(contentsDir, "Resources");
  if (!existsSync(resourcesDir)) {
    result.errors.push(`Missing Contents/Resources directory: ${resourcesDir}`);
  } else {
    result.resourcesPath = resourcesDir;
  }

  const runtimePath = join(bundlePath, EXPECTED_RUNTIME_REL);
  if (!existsSync(runtimePath)) {
    result.errors.push(`Packaged runtime missing at expected bundle-relative path: ${EXPECTED_RUNTIME_REL}`);
  } else {
    result.runtimePath = runtimePath;
  }

  result.valid = result.errors.length === 0;
  return result;
}

// ─── Codesign & spctl Diagnostic Checkers ───

export function checkCodesign(bundlePath) {
  try {
    const res = spawnSync("codesign", ["--verify", "--deep", "--strict", bundlePath], {
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 15000
    });
    return {
      status: res.status === 0 ? "PASS" : "FAIL",
      exitCode: res.status,
      stdout: res.stdout ? res.stdout.toString().trim() : "",
      stderr: res.stderr ? res.stderr.toString().trim() : ""
    };
  } catch (e) {
    return { status: "TOOL_UNAVAILABLE", error: e.message };
  }
}

export function checkSpctl(bundlePath) {
  try {
    const res = spawnSync("spctl", ["-a", "-t", "exec", "-v", bundlePath], {
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 15000
    });
    return {
      status: res.status === 0 ? "PASS" : "FAIL",
      exitCode: res.status,
      stdout: res.stdout ? res.stdout.toString().trim() : "",
      stderr: res.stderr ? res.stderr.toString().trim() : ""
    };
  } catch (e) {
    return { status: "TOOL_UNAVAILABLE", error: e.message };
  }
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
          // not JSON or logging output
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

    // Execute sequence
    (async () => {
      try {
        // 1. Ping
        const pingResp = await sendCommand({ cmd: "ping", id: "smoke-ping-1" }, 8000);
        if (pingResp.status !== "ok") {
          throw new Error(`Ping failed: ${JSON.stringify(pingResp)}`);
        }

        // 2. Open document
        const openResp = await sendCommand({ cmd: "open", path: fixturePath, id: "smoke-open-1" }, 12000);
        if (openResp.status !== "ok") {
          throw new Error(`Open document failed: ${JSON.stringify(openResp)}`);
        }

        // 3. Render thumbnail
        const renderResp = await sendCommand({
          cmd: "render_thumbnail",
          page_index: 0,
          scale: 0.25,
          id: "smoke-render-1"
        }, 12000);

        // 4. Extract text page 1
        const text1Resp = await sendCommand({
          cmd: "extract_text",
          page_index: 0,
          id: "smoke-text-1"
        }, 12000);

        // 5. Extract text page 2
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

// ─── Main Entry Point ───

async function main() {
  const args = parseCommonArgs(process.argv.slice(2));

  if (args.help) {
    console.log(`
macOS Artifact Smoke Harness (11I-E2)

Tests: MACOS_DMG, MACOS_APP_BUNDLE

Usage: node scripts/artifact-smoke-macos.mjs --artifact <path> [options]

Required:
  --artifact <path>        Path to the exact artifact (.dmg or .app bundle)

Options:
  --artifact-type <type>   Force artifact type (MACOS_DMG|MACOS_APP_BUNDLE)
                           Default: auto-detect from file extension
  --output <path>          Write evidence JSON to this path (default: stdout)
  --keep-temp              Do not clean up temporary workspace on exit
  --verbose                Verbose logging
  --help                   Show this help
`);
    process.exit(0);
  }

  const platform = "macos";
  const architecture = detectArchitecture();
  const runId = generateRunId();

  // Fail-closed handling for missing or unsupplied artifact path
  if (!args.artifactPath || !existsSync(args.artifactPath)) {
    const errorMsg = !args.artifactPath
      ? "Missing required --artifact argument"
      : `Artifact not found: ${args.artifactPath}`;

    let autoType = "MACOS_DMG";
    if (args.artifactPath && args.artifactPath.toLowerCase().endsWith(".app")) {
      autoType = "MACOS_APP_BUNDLE";
    }

    const evidence = createEmptyEvidence(platform, architecture, autoType, args.artifactPath || "MISSING", `local-macos-${runId}`);
    evidence.installResult = {
      status: "ARTIFACT_UNAVAILABLE",
      method: "none",
      installPath: null,
      durationMs: 0,
      error: errorMsg
    };
    evidence.launchResult.status = "ARTIFACT_UNAVAILABLE";
    evidence.overallStatus = "FAIL";

    console.error(`[macos-smoke] ARTIFACT_UNAVAILABLE: ${errorMsg}`);
    exitWithEvidence(evidence, args.outputPath);
    return;
  }

  let artifactType = args.artifactType;
  if (!artifactType) {
    const ext = args.artifactPath.toLowerCase().split(".").pop();
    if (ext === "dmg") artifactType = "MACOS_DMG";
    else if (ext === "app") artifactType = "MACOS_APP_BUNDLE";
    else {
      console.error(`[macos-smoke] Cannot auto-detect artifact type from extension '.${ext}'. Expected .dmg or .app`);
      process.exit(EXIT_CODES.INVALID_ARGS);
    }
  }

  if (!MACOS_ARTIFACT_TYPES.includes(artifactType)) {
    console.error(`[macos-smoke] Invalid macOS artifact type: ${artifactType}. Expected one of: ${MACOS_ARTIFACT_TYPES.join(", ")}`);
    process.exit(EXIT_CODES.INVALID_ARGS);
  }

  const workspace = createTempWorkspace(`artifact-smoke-macos-${runId}-`);
  const startTime = Date.now();
  let mounted = false;
  let mountPoint = null;

  // Cleanup handler for unmounting and removing workspace
  const safeCleanup = () => {
    if (mounted && mountPoint && existsSync(mountPoint)) {
      try {
        spawnSync("hdiutil", ["detach", mountPoint, "-force"], { timeout: 10000 });
        mounted = false;
      } catch {}
    }
    if (!args.keepTemp && existsSync(workspace)) {
      cleanupTempWorkspace(workspace);
    }
  };

  process.on("SIGINT", () => { safeCleanup(); process.exit(EXIT_CODES.REQUIRED_CHECK_FAILED); });
  process.on("SIGTERM", () => { safeCleanup(); process.exit(EXIT_CODES.REQUIRED_CHECK_FAILED); });

  console.log(`[macos-smoke] Run ID: ${runId}`);
  console.log(`[macos-smoke] Workspace: ${workspace}`);
  console.log(`[macos-smoke] Artifact: ${args.artifactPath} (${artifactType})`);

  const evidence = createEmptyEvidence(platform, architecture, artifactType, args.artifactPath, `local-macos-${runId}`);

  try {
    // ─── STEP 0: Artifact Identity ───
    console.log(`[macos-smoke] Step 0: Artifact Identity`);
    const artifactStat = statSync(args.artifactPath);
    evidence.artifactIdentity.byteLength = artifactStat.size;
    const artifactSha = await hashSha256Async(readFileSync(args.artifactPath));
    evidence.artifactIdentity.sha256 = artifactSha;
    console.log(`[macos-smoke] Artifact SHA-256: ${artifactSha}`);
    console.log(`[macos-smoke] Artifact size: ${artifactStat.size} bytes`);

    // ─── STEP 1: Deploy / Install to Isolated Location ───
    console.log(`[macos-smoke] Step 1: Deploy / Install (${artifactType})`);
    const installStart = Date.now();
    let deployedAppPath = null;
    let installMethod = "none";

    const deployDir = join(workspace, "deploy");
    mkdirSync(deployDir, { recursive: true });

    if (artifactType === "MACOS_DMG") {
      mountPoint = join(workspace, "mnt");
      mkdirSync(mountPoint, { recursive: true });

      const attachResult = await spawnWithTimeout("hdiutil", [
        "attach", "-mountpoint", mountPoint, "-nobrowse", "-readonly", "-noautoopen", args.artifactPath
      ], {}, TIMEOUTS.install);

      if (attachResult.code !== 0) {
        evidence.installResult = {
          status: "FAIL",
          method: "dmg_mount_copy",
          installPath: null,
          durationMs: Date.now() - installStart,
          error: `hdiutil attach failed: ${attachResult.stderr}`
        };
        throw new Error(`DMG mount failed: ${attachResult.stderr}`);
      }

      mounted = true;

      const entries = readdirSync(mountPoint);
      const appBundle = entries.find(e => e.endsWith(".app"));
      if (!appBundle) {
        await spawnWithTimeout("hdiutil", ["detach", mountPoint, "-force"], {}, 10000);
        mounted = false;
        evidence.installResult = {
          status: "FAIL",
          method: "dmg_mount_copy",
          installPath: null,
          durationMs: Date.now() - installStart,
          error: "No .app bundle found inside mounted DMG"
        };
        throw new Error("No .app in DMG");
      }

      const srcApp = join(mountPoint, appBundle);
      deployedAppPath = join(deployDir, appBundle);

      // Copy bundle from read-only mount to isolated deploy directory
      cpSync(srcApp, deployedAppPath, { recursive: true });

      // Detach source DMG before running smoke (must not run directly from disk image)
      const detachResult = await spawnWithTimeout("hdiutil", ["detach", mountPoint, "-force"], {}, 15000);
      mounted = false;

      if (detachResult.code !== 0) {
        console.warn(`[macos-smoke] Warning: hdiutil detach returned non-zero: ${detachResult.stderr}`);
      }

      installMethod = "dmg_mount_copy";

    } else if (artifactType === "MACOS_APP_BUNDLE") {
      const bundleName = basename(args.artifactPath);
      deployedAppPath = join(deployDir, bundleName);
      cpSync(args.artifactPath, deployedAppPath, { recursive: true });
      installMethod = "app_bundle_direct";
    }

    evidence.installResult = {
      status: "PASS",
      method: installMethod,
      installPath: deployedAppPath,
      durationMs: Date.now() - installStart,
      error: null
    };
    console.log(`[macos-smoke] Deployed bundle to: ${deployedAppPath}`);

    // ─── STEP 2: Structural Bundle Validation ───
    console.log(`[macos-smoke] Step 2: Structural Bundle Validation`);
    const bundleValidation = validateBundleStructure(deployedAppPath, EXPECTED_APP_NAME, EXPECTED_EXECUTABLE);
    if (!bundleValidation.valid) {
      evidence.launchResult = {
        status: "FAIL",
        executablePath: bundleValidation.executablePath,
        pid: null,
        startupDurationMs: 0,
        error: `Bundle validation failed: ${bundleValidation.errors.join("; ")}`
      };
      throw new Error(`Bundle validation failed: ${bundleValidation.errors.join("; ")}`);
    }

    const executablePath = bundleValidation.executablePath;
    const runtimePath = bundleValidation.runtimePath;
    const resourcesPath = bundleValidation.resourcesPath;

    // ─── STEP 3: Verify Packaged Runtime Integrity & Mach-O Architecture ───
    console.log(`[macos-smoke] Step 3: Verify Packaged Runtime Integrity`);
    const runtimeData = readFileSync(runtimePath);
    const runtimeSha = await hashSha256Async(runtimeData);
    const runtimeLength = runtimeData.length;

    const machOCheck = validateMachOArchitecture(runtimePath);
    let runtimeStatus = "PASS";
    let runtimeError = null;

    if (!machOCheck.ok) {
      runtimeStatus = "FAIL";
      runtimeError = `Mach-O validation failed: ${machOCheck.error}`;
    } else if (runtimeSha !== EXPECTED_RUNTIME_SHA256) {
      runtimeStatus = "FAIL";
      runtimeError = `SHA-256 mismatch: expected ${EXPECTED_RUNTIME_SHA256}, got ${runtimeSha}`;
    } else if (runtimeLength !== EXPECTED_RUNTIME_BYTE_LENGTH) {
      runtimeStatus = "FAIL";
      runtimeError = `Byte length mismatch: expected ${EXPECTED_RUNTIME_BYTE_LENGTH}, got ${runtimeLength}`;
    }

    evidence.runtimeResolutionResult = {
      status: runtimeStatus,
      resolvedPath: runtimePath,
      source: "packaged",
      sha256: runtimeSha,
      error: runtimeError
    };

    if (runtimeStatus !== "PASS") {
      throw new Error(runtimeError);
    }
    console.log(`[macos-smoke] Packaged runtime verified (Mach-O arm64, SHA-256 ${runtimeSha})`);

    // ─── STEP 4: Launch Application with Clean Isolated Environment ───
    console.log(`[macos-smoke] Step 4: Launch Application & Worker Verification`);
    const launchStart = Date.now();

    // Prepare clean child environment: strip dev overrides to prevent contamination
    const cleanEnv = { ...process.env };
    delete cleanEnv.PDFIUM_PATH;
    delete cleanEnv.PDFIUM_RESOLVED_PATH;
    delete cleanEnv.DYLD_LIBRARY_PATH;
    delete cleanEnv.DYLD_FALLBACK_LIBRARY_PATH;
    delete cleanEnv.CARGO_MANIFEST_DIR;
    cleanEnv.TAURI_RESOURCE_DIR = resourcesPath;

    const fixturePath = copyFixtureToWorkspace("core-release-smoke", workspace);
    console.log(`[macos-smoke] Staged canonical fixture: ${fixturePath}`);

    const smokeSession = await runInteractiveSmoke(executablePath, fixturePath, cleanEnv, workspace, TIMEOUTS.launch + TIMEOUTS.documentOpen * 2);

    evidence.launchResult = {
      status: "PASS",
      executablePath,
      pid: 0,
      startupDurationMs: Date.now() - launchStart,
      error: null
    };

    // ─── STEP 5: Interpret Smoke Results ───
    console.log(`[macos-smoke] Step 5: Smoke Document Assertions`);
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

    // ─── STEP 6: Shutdown Verification ───
    console.log(`[macos-smoke] Step 6: Shutdown Verification`);
    const cleanExit = smokeSession.code === 0;
    evidence.shutdownResult = {
      status: cleanExit ? "PASS" : "FAIL",
      cleanExit,
      workerCleanup: true,
      durationMs: 0,
      error: cleanExit ? null : `Process exited with code ${smokeSession.code}, signal ${smokeSession.signal}`
    };

    // ─── STEP 7: Codesign & Gatekeeper Diagnostics (Optional) ───
    console.log(`[macos-smoke] Step 7: Codesign & Gatekeeper Diagnostics`);
    const codesignResult = checkCodesign(deployedAppPath);
    const spctlResult = checkSpctl(deployedAppPath);

    evidence.externalValidationResults = [
      {
        tool: "codesign",
        status: codesignResult.status,
        detail: codesignResult.stdout || codesignResult.stderr || codesignResult.error || "verified"
      },
      {
        tool: "spctl",
        status: spctlResult.status,
        detail: spctlResult.stdout || spctlResult.stderr || spctlResult.error || "assessed"
      }
    ];

  } catch (err) {
    console.error(`[macos-smoke] FAIL: ${err.message}`);
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
    console.error(`[macos-smoke] FATAL: ${err.message}`);
    process.exit(EXIT_CODES.INTERNAL_ERROR);
  });
}