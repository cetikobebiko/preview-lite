#!/usr/bin/env node
// 11I-C2 — PDFium runtime manifest validator (offline, dependency-free)
// Validates docs/engineering/platform-support.json as canonical runtime identity owner
// Covers §15 invariants + §16 security (no executable config) + §3-§14 C2 contract
import { readFileSync, existsSync, lstatSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const repoRoot = resolve(__dirname, "..");

const KNOWN_PLATFORMS = Object.freeze(["macos", "linux", "windows"]);
const KNOWN_ARCHS = Object.freeze(["arm64", "x86_64", "aarch64", "amd64"]); // alias handling via normalization, but canonical is arm64/x86_64
const CANONICAL_ARCH = Object.freeze(["arm64", "x86_64"]);
const ALLOWED_PROVENANCE_CLASSES = Object.freeze([
  "unknown",
  "project_bundled_verified",
  "upstream_prebuilt_verified",
  "locally_built_verified",
  "development_external",
]);
const ALLOWED_DISTRIBUTION_CLASSES = Object.freeze([
  "bundled_with_artifact",
  "build_time_provisioned",
  "ci_cache",
  "system_installation",
  "development_external",
  "unknown",
]);
const ALLOWED_SUPPLY_STATUSES = Object.freeze(["COMPLETE", "PARTIAL", "INCOMPLETE", "MISSING"]);
const ALLOWED_STATUSES = Object.freeze(["verified", "experimental", "build_only", "unsupported", "not_evaluated"]);
const SAFE_FILENAME_RE = {
  "macos": /^libpdfium\.dylib$/,
  "linux": /^libpdfium\.so$/,
  "windows": /^pdfium\.dll$/,
};

function isValidSha256(s) {
  return typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
}

function validateLexicalPath(rawPath) {
  if (typeof rawPath !== "string" || rawPath.trim().length === 0) return "empty path";
  if (rawPath.includes("://")) return `URI scheme forbidden: '${rawPath}'`;
  if (rawPath.startsWith("/") ) return `absolute path forbidden: '${rawPath}'`;
  if (rawPath.startsWith("\\") || rawPath.startsWith("//") || rawPath.startsWith("\\\\")) return `UNC/abs forbidden: '${rawPath}'`;
  if (rawPath.includes("\\")) return `backslash forbidden: '${rawPath}'`;
  if (/^[A-Za-z]:/.test(rawPath)) return `Windows drive letter forbidden: '${rawPath}'`;
  for (const seg of rawPath.split("/")) {
    if (seg.length === 0) return `empty segment/double slash: '${rawPath}'`;
    if (seg === "." ) return `'.' segment forbidden: '${rawPath}'`;
    if (seg === "..") return `'..' traversal forbidden: '${rawPath}'`;
    if (seg.startsWith("._")) return `AppleDouble forbidden: '${rawPath}'`;
  }
  return null;
}

function hasForbiddenExecutableConfig(obj, pathStack, errors) {
  const FORBIDDEN_KEYS = new Set([
    "shellCommand", "shell_command", "downloadUrl", "download_url", "fetchUrl", "exec", "postInstall", "postInstallCommand", "installCommand", "installerCommand", "arbitraryDownloader", "downloader", "curl", "wget"
  ]);
  const FORBIDDEN_VALUE_PATTERNS = [
    /\bcurl\b/i,
    /\bwget\b/i,
    /\bapt(-get)?\b/i,
    /\bdnf\b/i,
    /\bpacman\b/i,
    /\bnpm\s+install\b/i,
    /\bpip\s+install\b/i,
  ];
  function walk(node, stack) {
    if (node && typeof node === "object") {
      if (Array.isArray(node)) {
        node.forEach((v, i) => walk(v, [...stack, `[${i}]`]));
      } else {
        for (const [k, v] of Object.entries(node)) {
          if (FORBIDDEN_KEYS.has(k)) {
            errors.push(`${stack.join(".")}.${k}: forbidden executable/downloader config key '${k}' — manifest must be data only (§16)`);
          }
          // also forbid keys that look like URL executor
          if (/url/i.test(k) && k !== "upstreamUrl" && k !== "sourceArtifactChecksum") {
            // allow only provenance upstreamUrl and sourceArtifactChecksum; other url keys are suspicious
            if (k.toLowerCase().includes("download") || k.toLowerCase().includes("fetch")) {
              errors.push(`${stack.join(".")}.${k}: suspicious URL executor key '${k}' forbidden (§16)`);
            }
          }
          if (typeof v === "string" && FORBIDDEN_VALUE_PATTERNS.some(re => re.test(v))) {
            // allow only if it's in documentation string? but be strict: any manifest string containing curl/wget is forbidden as executable config
            // Exception: statusReason or notes may mention curl/wget as forbidden example — allow if field is statusReason
            const allowlistField = ["statusReason", "notes", "description", "message"];
            if (!allowlistField.includes(k)) {
              errors.push(`${stack.join(".")}.${k}: value contains forbidden executable pattern '${v.slice(0,120)}' (§16)`);
            }
          }
          if (v && typeof v === "object") walk(v, [...stack, k]);
        }
      }
    }
  }
  walk(obj, pathStack);
}

function normalizeArch(arch) {
  if (arch === "amd64") return "x86_64";
  if (arch === "aarch64") return "arm64";
  return arch;
}

function run() {
  const errors = [];
  const warnings = [];
  const manifestPath = process.argv[2]
    ? resolve(process.cwd(), process.argv[2])
    : resolve(repoRoot, "docs/engineering/platform-support.json");

  if (!existsSync(manifestPath)) {
    console.error(`FATAL: manifest missing at ${manifestPath}`);
    process.exit(1);
  }

  let doc;
  try {
    doc = JSON.parse(readFileSync(manifestPath, "utf-8"));
  } catch (e) {
    console.error(`FATAL: JSON parse failed: ${e.message}`);
    process.exit(1);
  }

  // §15 schema version valid
  if (typeof doc.schemaVersion !== "string" && typeof doc.schemaVersion !== "number") {
    errors.push(`schemaVersion must be string '1.0.0' or number 1, got ${JSON.stringify(doc.schemaVersion)}`);
  } else {
    const sv = String(doc.schemaVersion);
    if (sv !== "1.0.0" && sv !== "1") {
      errors.push(`schemaVersion must be '1.0.0' (preferred) or 1, got '${sv}'`);
    }
  }
  if (doc.pdfiumRuntimeContractVersion && String(doc.pdfiumRuntimeContractVersion) !== "1.0.0") {
    errors.push(`pdfiumRuntimeContractVersion must be '1.0.0', got '${doc.pdfiumRuntimeContractVersion}'`);
  }
  if (doc.canonicalRuntimeManifest !== "docs/engineering/platform-support.json") {
    warnings.push(`canonicalRuntimeManifest expected 'docs/engineering/platform-support.json', got '${doc.canonicalRuntimeManifest}'`);
  }
  if (doc.pdfiumRenderVersion && doc.pdfiumRenderVersion !== "0.8.37") {
    warnings.push(`pdfiumRenderVersion expected '0.8.37' per Cargo.lock, got '${doc.pdfiumRenderVersion}'`);
  }

  // allowedStatuses
  if (!Array.isArray(doc.allowedStatuses)) {
    errors.push("allowedStatuses must be array");
  } else {
    for (const s of ALLOWED_STATUSES) if (!doc.allowedStatuses.includes(s)) errors.push(`allowedStatuses missing '${s}'`);
  }

  // platforms
  if (!Array.isArray(doc.platforms) || doc.platforms.length === 0) {
    errors.push("platforms must be non-empty array");
  }

  // §16 security scan
  hasForbiddenExecutableConfig(doc, ["root"], errors);

  // verificationContract checks
  if (doc.verificationContract) {
    const vc = doc.verificationContract;
    if (vc.sha256Required !== true) errors.push("verificationContract.sha256Required must be true (§5)");
    if (vc.failClosedOnMismatch !== true) errors.push("verificationContract.failClosedOnMismatch must be true (§9)");
    if (vc.noFallbackToUnverified !== true) errors.push("verificationContract.noFallbackToUnverified must be true (§9)");
    if (vc.noNetworkDownload !== true) warnings.push("verificationContract.noNetworkDownload should be true (§0)");
    if (vc.compatibilityAssumption && !vc.compatibilityAssumption.includes("UNVERIFIED")) {
      warnings.push("verificationContract.compatibilityAssumption should contain UNVERIFIED when no compatibility table exists (§13)");
    }
    if (vc.archiveSecurity) {
      const a = vc.archiveSecurity;
      for (const k of ["rejectTraversal","rejectAbsolutePath","rejectSymlinkEscape","rejectUnexpectedFile","exactExpectedRuntimeSelection","tempExtraction","atomicPublication","hashVerificationBeforeUse"]) {
        if (a[k] !== true) errors.push(`verificationContract.archiveSecurity.${k} must be true (§10)`);
      }
    }
  } else {
    errors.push("verificationContract missing (§9-10)");
  }

  const architectureAliases = doc.architectureAliases || {};
  // Check alias mapping is bounded
  if (Object.keys(architectureAliases).length > 0) {
    for (const [canon, aliases] of Object.entries(architectureAliases)) {
      if (!CANONICAL_ARCH.includes(canon)) errors.push(`architectureAliases canonical '${canon}' must be one of ${CANONICAL_ARCH.join(",")}`);
      if (!Array.isArray(aliases)) errors.push(`architectureAliases.${canon} must be array`);
      else {
        for (const a of aliases) if (!KNOWN_ARCHS.includes(a)) errors.push(`architectureAliases.${canon} alias '${a}' unknown`);
      }
    }
  }

  const seenIds = new Set();
  const seenPlatformArch = new Set();
  let linuxRecordCount = 0;
  let windowsRecordCount = 0;
  let knownShaCount = 0;
  let unknownShaCount = 0;
  let provenanceGapCount = 0;

  for (const plat of doc.platforms || []) {
    const ctx = `platform ${plat.targetTriple || plat.operatingSystem+":"+plat.architecture}`;
    // targetTriple, operatingSystem, architecture, status
    if (!plat.targetTriple || typeof plat.targetTriple !== "string") errors.push(`${ctx}: targetTriple missing`);
    if (!KNOWN_PLATFORMS.includes(plat.operatingSystem)) errors.push(`${ctx}: unknown operatingSystem '${plat.operatingSystem}' (allowed ${KNOWN_PLATFORMS.join(",")})`);
    // architecture: allow canonical, but normalized check
    const normArch = normalizeArch(plat.architecture);
    if (!CANONICAL_ARCH.includes(normArch)) errors.push(`${ctx}: unknown architecture '${plat.architecture}' (canonical ${CANONICAL_ARCH.join(",")}, aliases via architectureAliases)`);
    if (!ALLOWED_STATUSES.includes(plat.status)) errors.push(`${ctx}: unknown status '${plat.status}'`);
    if (typeof plat.publicationAllowed !== "boolean") errors.push(`${ctx}: publicationAllowed must be boolean`);
    // platform+arch uniqueness
    const paKey = `${plat.operatingSystem}:${normalizeArch(plat.architecture)}`;
    if (seenPlatformArch.has(paKey)) errors.push(`Duplicate platform+arch mapping '${paKey}' (§15 unique platform+arch)`);
    seenPlatformArch.add(paKey);

    // runtime existence
    const r = plat.runtime;
    if (!r || typeof r !== "object") {
      errors.push(`${ctx}: runtime missing`);
      continue;
    }
    const rctx = `${ctx} runtime`;

    // §3 minimum fields
    for (const field of ["id","platform","architecture","runtimeFileName","versionOrRevision","sha256","provenance","distributionClass"]) {
      if (!(field in r)) errors.push(`${rctx}: missing required field '${field}' (§3)`);
    }

    // id unique
    if (typeof r.id !== "string" || r.id.trim()==="") {
      errors.push(`${rctx}: id must be non-empty string`);
    } else {
      if (seenIds.has(r.id)) errors.push(`Duplicate runtime ID '${r.id}' (§15 unique runtime ID)`);
      seenIds.add(r.id);
      if (r.id.includes("..") || r.id.includes("/") ) errors.push(`${rctx}: id '${r.id}' must not contain traversal or slash`);
      if (r.id.length > 128) errors.push(`${rctx}: id too long`);
    }

    // platform/arch must match parent canonical normalized
    if (r.platform !== plat.operatingSystem) errors.push(`${rctx}: runtime.platform '${r.platform}' != parent operatingSystem '${plat.operatingSystem}'`);
    if (normalizeArch(r.architecture) !== normArch) errors.push(`${rctx}: runtime.architecture '${r.architecture}' != parent architecture '${plat.architecture}' (normalized mismatch §11)`);

    // runtimeFileName safe filename (§15)
    if (typeof r.runtimeFileName !== "string") errors.push(`${rctx}: runtimeFileName must be string`);
    else {
      const fnErr = validateLexicalPath(r.runtimeFileName);
      // filename should be simple, no slash, but validateLexicalPath checks slash segments — for filename with no slash, it will pass
      // additionally ensure it contains no '/' at all
      if (r.runtimeFileName.includes("/")) errors.push(`${rctx}: runtimeFileName must be bare filename, got '${r.runtimeFileName}' (no traversal (§15))`);
      if (r.runtimeFileName.includes("..") || r.runtimeFileName.startsWith("/") || r.runtimeFileName.includes("\\")) errors.push(`${rctx}: runtimeFileName contains absolute/traversal/backslash`);
      const expectedRe = SAFE_FILENAME_RE[plat.operatingSystem];
      if (expectedRe && !expectedRe.test(r.runtimeFileName)) errors.push(`${rctx}: runtimeFileName '${r.runtimeFileName}' does not match expected for ${plat.operatingSystem} (${expectedRe})`);
      // also check lowercase sha invariant: filename case sensitive, but runtime filename must be exact case
    }
    // filename legacy alias must match runtimeFileName
    if (r.filename && r.filename !== r.runtimeFileName) errors.push(`${rctx}: filename '${r.filename}' != runtimeFileName '${r.runtimeFileName}' (alias must match)`);
    // versionOrRevision: allow null (UNKNOWN) or string
    if (r.versionOrRevision !== null && typeof r.versionOrRevision !== "string") errors.push(`${rctx}: versionOrRevision must be string or null`);
    else if (typeof r.versionOrRevision === "string" && r.versionOrRevision.trim()!=="" && r.versionOrRevision !== "UNKNOWN") {
      if (!/^\d+\.\d+\.\d+\.\d+$/.test(r.versionOrRevision) && r.versionOrRevision !== "UNKNOWN") {
        warnings.push(`${rctx}: versionOrRevision '${r.versionOrRevision}' does not match MAJOR.MINOR.BUILD.PATCH pattern (allowed but check provenance)`);
      }
    }

    // sha256 valid lowercase (§15) or null with UNKNOWN handling §6
    if (r.sha256 !== null) {
      if (!isValidSha256(r.sha256)) errors.push(`${rctx}: sha256 must be 64 lowercase hex, got '${r.sha256}' (§5/15)`);
      else knownShaCount++;
      // if sha known, byteLength must be present + >0, supplyChainStatus should not be INCOMPLETE
      if (typeof r.byteLength !== "number" || r.byteLength <= 0) errors.push(`${rctx}: byteLength must be positive integer when sha256 present`);
      if (r.supplyChainStatus === "INCOMPLETE") warnings.push(`${rctx}: supplyChainStatus INCOMPLETE but sha256 present — should be PARTIAL or COMPLETE`);
    } else {
      unknownShaCount++;
      // null sha must have supplyChainStatus INCOMPLETE per §6
      if (r.supplyChainStatus !== "INCOMPLETE") errors.push(`${rctx}: sha256 null (UNKNOWN) requires supplyChainStatus INCOMPLETE, got '${r.supplyChainStatus}' (§6)`);
      // statusReason must contain SHA256 UNKNOWN marker for linux/windows
      if (plat.operatingSystem === "linux" || plat.operatingSystem === "windows") {
        const sr = (plat.statusReason || "");
        if (!sr.includes("SHA256: UNKNOWN") && !sr.includes("UNKNOWN")) warnings.push(`${rctx}: statusReason should contain 'SHA256: UNKNOWN' when sha256 null (§6)`);
      }
    }

    // byteLength
    if (r.byteLength !== null && r.byteLength !== undefined) {
      if (!Number.isInteger(r.byteLength) || r.byteLength < 0) errors.push(`${rctx}: byteLength must be non-negative integer or null`);
      if (r.byteLength !== null && r.byteLength > 50*1024*1024) warnings.push(`${rctx}: byteLength ${r.byteLength} unusually large >50MiB`);
    }

    // provenance explicit (§15 unknown provenance explicit)
    if (!r.provenance || typeof r.provenance !== "object") {
      errors.push(`${rctx}: provenance must be object (§7)`);
    } else {
      if (!ALLOWED_PROVENANCE_CLASSES.includes(r.provenance.class)) errors.push(`${rctx}: provenance.class '${r.provenance.class}' unknown (allowed ${ALLOWED_PROVENANCE_CLASSES.join(",")}) (§7)`);
      if (r.provenance.class === "unknown") provenanceGapCount++;
      // provenance fields must not be invented if class unknown -> must be null
      if (r.provenance.class === "unknown") {
        for (const f of ["upstreamUrl","sourceArtifactName","sourceArtifactChecksum","sourceCommit","builder","buildDate"]) {
          if (r.provenance[f] !== null && r.provenance[f] !== undefined) {
            // for unknown, these must be null (no invented provenance §6, §7)
            // But allow explicit null only
            if (r.provenance[f] !== null) warnings.push(`${rctx}: provenance.class unknown but provenance.${f} is non-null — ensure not invented (§7)`);
          }
          // also check upstreamUrl if non-null must be valid URL data but not executable
          if (typeof r.provenance[f] === "string" && r.provenance[f].includes("://")) {
            if (!/^https?:\/\//.test(r.provenance[f]) && f === "upstreamUrl") errors.push(`${rctx}: provenance.${f} url must be https:// if present`);
          }
        }
      } else {
        // verified classes require at least upstreamUrl and sourceCommit/buildDate
        if (!r.provenance.upstreamUrl) errors.push(`${rctx}: provenance class '${r.provenance.class}' requires upstreamUrl`);
        if (!r.provenance.sourceCommit) warnings.push(`${rctx}: provenance class '${r.provenance.class}' should have sourceCommit`);
      }
      // forbid provenance containing shell-like fields
      for (const fk of ["shellCommand","downloadUrl","exec"]) if (fk in r.provenance) errors.push(`${rctx}: provenance must not contain executable key '${fk}' (§16)`);
    }

    // distributionClass
    if (!ALLOWED_DISTRIBUTION_CLASSES.includes(r.distributionClass)) errors.push(`${rctx}: distributionClass '${r.distributionClass}' unknown (allowed ${ALLOWED_DISTRIBUTION_CLASSES.join(",")}) (§8)`);

    // licenseReference safe path, no absolute, no traversal
    if (typeof r.licenseReference !== "string" || r.licenseReference.trim()==="") errors.push(`${rctx}: licenseReference must be non-empty string (§14)`);
    else {
      const lrErr = validateLexicalPath(r.licenseReference);
      if (lrErr) errors.push(`${rctx}: licenseReference '${r.licenseReference}' invalid: ${lrErr} (§15 no absolute/traversal)`);
      if (r.licenseReference.includes("://")) errors.push(`${rctx}: licenseReference must not be URL (§16)`);
    }

    // expectedStagedPath / expectedPackagedLocation
    for (const pf of ["expectedStagedPath","expectedPackagedLocation"]) {
      if (!(pf in r)) {
        warnings.push(`${rctx}: missing ${pf} (expected placement contract §2)`);
        continue;
      }
      const v = r[pf];
      if (typeof v !== "string") errors.push(`${rctx}: ${pf} must be string`);
      else {
        // expectedPackagedLocation may be like "resourceDir/lib/libpdfium.so" — this is not a literal filesystem path but a contract term; allow resourceDir prefix
        if (v.startsWith("resourceDir/")) {
          const rest = v.replace("resourceDir/","");
          const restErr = validateLexicalPath(rest);
          if (restErr) errors.push(`${rctx}: ${pf} '${v}' rest '${rest}' invalid: ${restErr}`);
        } else {
          const pErr = validateLexicalPath(v);
          if (pErr) errors.push(`${rctx}: ${pf} '${v}' invalid: ${pErr} (§15 no absolute/traversal)`);
        }
        if (v.includes("..")) errors.push(`${rctx}: ${pf} '${v}' must not contain traversal`);
        if (v.includes("://")) errors.push(`${rctx}: ${pf} must not be URL (§16)`);
      }
    }

    // supplyChainStatus
    if (!ALLOWED_SUPPLY_STATUSES.includes(r.supplyChainStatus)) errors.push(`${rctx}: supplyChainStatus '${r.supplyChainStatus}' unknown (allowed ${ALLOWED_SUPPLY_STATUSES.join(",")})`);

    // known platform/arch already checked
    // safe filename already

    // paths: no absolute, no traversal, no URL (§15)
    if (!Array.isArray(r.paths)) errors.push(`${rctx}: paths must be array`);
    else {
      for (const p of r.paths) {
        const pe = validateLexicalPath(p);
        if (pe) errors.push(`${rctx}: paths entry '${p}' invalid: ${pe} (§15 no absolute/traversal)`);
        if (p.includes("://")) errors.push(`${rctx}: paths entry '${p}' must not be URL (§16)`);
        if (p.includes("..")) errors.push(`${rctx}: paths entry '${p}' traversal forbidden`);
      }
      if (r.present === true && r.paths.length === 0) errors.push(`${rctx}: present true but paths empty`);
      if (r.present === false && r.paths.length !== 0) errors.push(`${rctx}: present false but paths non-empty (expected [] when absent)`);
    }

    // present / verifiedForRelease types
    if (typeof r.present !== "boolean") errors.push(`${rctx}: present must be boolean`);
    if (typeof r.verifiedForRelease !== "boolean") errors.push(`${rctx}: verifiedForRelease must be boolean`);

    // Linux specific count
    if (plat.operatingSystem === "linux") {
      linuxRecordCount++;
      // §4 supported vs unavailable: linux must have exactly one runtime record (we already have per platform one)
      // filename must be libpdfium.so
      if (r.runtimeFileName !== "libpdfium.so") errors.push(`${rctx}: linux runtimeFileName must be 'libpdfium.so'`);
      // §5 sha256 required but currently unknown — already handled as null INCOMPLETE (not error, but flagged)
      // distributionClass must be bundled_with_artifact
      if (r.distributionClass !== "bundled_with_artifact") warnings.push(`${rctx}: linux distributionClass expected 'bundled_with_artifact', got '${r.distributionClass}'`);

      // 11I-C4: Packaging config cross-check
      const tauriLinuxConfPath = resolve(repoRoot, "src-tauri/tauri.linux.conf.json");
      if (!existsSync(tauriLinuxConfPath)) {
        errors.push(`${rctx}: src-tauri/tauri.linux.conf.json missing for bundled_with_artifact Linux runtime`);
      } else {
        try {
          const tauriLinuxConf = JSON.parse(readFileSync(tauriLinuxConfPath, "utf-8"));
          const resources = tauriLinuxConf.bundle?.resources || [];
          const expectedResource = (r.expectedStagedPath || "").replace(/^src-tauri\//, "");
          if (!resources.includes(expectedResource)) {
            errors.push(`${rctx}: tauri.linux.conf.json bundle.resources missing expected resource '${expectedResource}'`);
          }
        } catch (e) {
          errors.push(`${rctx}: failed to parse tauri.linux.conf.json: ${e.message}`);
        }
      }
    }

    // Windows specific checks (11I-D2)
    if (plat.operatingSystem === "windows") {
      windowsRecordCount++;
      if (r.runtimeFileName !== "pdfium.dll") errors.push(`${rctx}: windows runtimeFileName must be 'pdfium.dll'`);
      if (r.distributionClass !== "bundled_with_artifact") warnings.push(`${rctx}: windows distributionClass expected 'bundled_with_artifact', got '${r.distributionClass}'`);

      // 11I-D2 / D4: Packaging config cross-check
      const tauriWindowsConfPath = resolve(repoRoot, "src-tauri/tauri.windows.conf.json");
      if (!existsSync(tauriWindowsConfPath)) {
        errors.push(`${rctx}: src-tauri/tauri.windows.conf.json missing for bundled_with_artifact Windows runtime`);
      } else {
        try {
          const tauriWindowsConf = JSON.parse(readFileSync(tauriWindowsConfPath, "utf-8"));
          const resources = tauriWindowsConf.bundle?.resources || [];
          const expectedResource = (r.expectedStagedPath || "").replace(/^src-tauri\//, "");
          if (!resources.includes(expectedResource)) {
            warnings.push(`${rctx}: tauri.windows.conf.json bundle.resources missing expected resource '${expectedResource}' (currently has ${JSON.stringify(resources)}) — packaging resource discrepancy handed off to 11I-D4`);
          }
        } catch (e) {
          errors.push(`${rctx}: failed to parse tauri.windows.conf.json: ${e.message}`);
        }
      }
    }

    // license metadata gap check §14
    if (!r.licenseReference) warnings.push(`${rctx}: licenseReference missing (§14)`);

    // Check for stray executable keys at runtime level
    for (const fk of ["shellCommand","downloadUrl","fetchCommand","postInstall","exec"]) {
      if (fk in r) errors.push(`${rctx}: forbidden key '${fk}' (§16)`);
    }
  }

  // §15 supported target has exactly intended runtime record — already via per-platform one record
  if (linuxRecordCount !== 1) errors.push(`Supported Linux runtime records must be exactly 1 (found ${linuxRecordCount}) (§4)`);
  if (windowsRecordCount !== 1) errors.push(`Supported Windows runtime records must be exactly 1 (found ${windowsRecordCount}) (11I-D2 §4)`);
  // unique platform+arch already
  // known platform/arch already

  // summary checks
  const totalPlatforms = (doc.platforms||[]).length;
  if (totalPlatforms !== 3) warnings.push(`platforms count expected 3 (macos, linux, windows), got ${totalPlatforms}`);

  // Final counts output
  console.log("PDFium Runtime Manifest Validator — 11I-C2 / 11I-D2 §15");
  console.log(`Manifest: ${manifestPath}`);
  console.log(`SchemaVersion: ${doc.schemaVersion} / contract ${doc.pdfiumRuntimeContractVersion || "n/a"}`);
  console.log(`Platforms: ${totalPlatforms} (linux records: ${linuxRecordCount}, windows records: ${windowsRecordCount})`);
  console.log(`Known SHA256: ${knownShaCount}, Unknown: ${unknownShaCount}, Provenance gaps: ${provenanceGapCount}`);

  if (warnings.length > 0) {
    console.warn(`\n⚠ Warnings (${warnings.length}):`);
    for (const w of warnings) console.warn(`  - ${w}`);
  }

  if (errors.length > 0) {
    console.error(`\n❌ Validation FAILED with ${errors.length} error(s):`);
    for (const e of errors) console.error(`  - ${e}`);
    process.exit(1);
  }

  console.log("\n✅ Validation PASSED — supply-chain contract VALID");
  console.log("  - schema version valid");
  console.log("  - unique runtime ID");
  console.log("  - unique platform+arch");
  console.log("  - known platform/arch");
  console.log("  - SHA-256 format (lowercase) or explicit UNKNOWN");
  console.log("  - safe filename, no absolute/traversal, no URL executor");
  console.log("  - provenance explicit (unknown class allowed, not invented)");
  console.log("  - distributionClass explicit");
  console.log("  - no executable config (§16)");
  process.exit(0);
}

run();
