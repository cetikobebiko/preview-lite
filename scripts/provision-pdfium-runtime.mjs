#!/usr/bin/env node
// Build-time PDFium runtime provisioning for Linux and Windows (dependency-free).
//
// Reads the pinned runtime record from docs/engineering/platform-support.json,
// downloads provenance.upstreamUrl, verifies the archive against
// provenance.sourceArtifactChecksum, extracts exactly provenance.sourceArtifactMember
// in memory and stages it at runtime.expectedStagedPath only after its byte length
// and SHA-256 match the manifest. Any mismatch fails closed; nothing unverified is
// ever written to the staged path. The application itself never downloads PDFium.
import { createHash } from "node:crypto";
import { closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, writeSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = resolve(repoRoot, "docs/engineering/platform-support.json");

const TARGET_TRIPLES = Object.freeze({
  linux: "x86_64-unknown-linux-gnu",
  windows: "x86_64-pc-windows-msvc",
});
const ALLOWED_UPSTREAM_PREFIX = "https://github.com/bblanchon/pdfium-binaries/releases/download/";
const MAX_ARCHIVE_BYTES = 64 * 1024 * 1024;

export class ProvisionError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

export function sha256Hex(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

// Returns the pinned runtime record for a platform, or throws when the manifest
// does not pin every field provisioning depends on.
export function runtimeRecordFor(manifest, platform) {
  const triple = TARGET_TRIPLES[platform];
  if (!triple) throw new ProvisionError("UNSUPPORTED_PLATFORM", `no provisioning for '${platform}'`);
  const entry = manifest.platforms?.find((p) => p.targetTriple === triple);
  const runtime = entry?.runtime;
  if (!runtime) throw new ProvisionError("RUNTIME_RECORD_MISSING", `no runtime record for ${triple}`);

  const provenance = runtime.provenance ?? {};
  const checksum = /^sha256:([0-9a-f]{64})$/.exec(provenance.sourceArtifactChecksum ?? "");
  const problems = [];
  if (runtime.supplyChainStatus !== "COMPLETE") problems.push(`supplyChainStatus is '${runtime.supplyChainStatus}'`);
  if (provenance.class !== "upstream_prebuilt_verified") problems.push(`provenance.class is '${provenance.class}'`);
  if (!/^[0-9a-f]{64}$/.test(runtime.sha256 ?? "")) problems.push("sha256 is not pinned");
  if (!Number.isSafeInteger(runtime.byteLength) || runtime.byteLength <= 0) problems.push("byteLength is not pinned");
  if (!checksum) problems.push("sourceArtifactChecksum must be 'sha256:<64 hex>'");
  if (typeof provenance.upstreamUrl !== "string" || !provenance.upstreamUrl.startsWith(ALLOWED_UPSTREAM_PREFIX)) {
    problems.push(`upstreamUrl must start with ${ALLOWED_UPSTREAM_PREFIX}`);
  } else if (!provenance.upstreamUrl.endsWith(`/${provenance.sourceArtifactName}`)) {
    problems.push("upstreamUrl must end with sourceArtifactName");
  }
  if (!isSafeMemberPath(provenance.sourceArtifactMember ?? "")) problems.push("sourceArtifactMember must be a relative path without traversal");
  else if (!provenance.sourceArtifactMember.endsWith(`/${runtime.runtimeFileName}`)) problems.push("sourceArtifactMember must name runtimeFileName");
  if (runtime.expectedStagedPath !== `src-tauri/lib/${runtime.runtimeFileName}`) problems.push("expectedStagedPath must be src-tauri/lib/<runtimeFileName>");
  if (problems.length > 0) throw new ProvisionError("RUNTIME_METADATA_INCOMPLETE", `${triple}: ${problems.join("; ")}`);

  return {
    triple,
    url: provenance.upstreamUrl,
    archiveSha256: checksum[1],
    member: provenance.sourceArtifactMember,
    sha256: runtime.sha256,
    byteLength: runtime.byteLength,
    stagedPath: runtime.expectedStagedPath,
  };
}

function isSafeMemberPath(path) {
  if (path === "" || path.startsWith("/") || path.includes("\\") || /^[A-Za-z]:/.test(path)) return false;
  return path.split("/").every((segment) => segment !== "" && segment !== "." && segment !== "..");
}

function readTarString(block, offset, length) {
  const field = block.subarray(offset, offset + length);
  const end = field.indexOf(0);
  return field.subarray(0, end < 0 ? length : end).toString("utf8");
}

// Parses a plain ustar archive. Links, PAX/GNU extension headers, unsafe paths and
// duplicate names reject the whole archive instead of being skipped.
export function readTarEntries(tar) {
  const entries = new Map();
  let offset = 0;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;

    const storedChecksum = parseInt(readTarString(header, 148, 8).trim(), 8);
    let checksum = 0;
    for (let index = 0; index < 512; index += 1) checksum += index >= 148 && index < 156 ? 32 : header[index];
    if (storedChecksum !== checksum) throw new ProvisionError("ARCHIVE_INVALID", `bad tar header checksum at offset ${offset}`);

    const prefix = readTarString(header, 345, 155);
    const rawName = readTarString(header, 0, 100);
    const name = (prefix ? `${prefix}/${rawName}` : rawName).replace(/^\.\//, "");
    const type = String.fromCharCode(header[156] || 48);
    const size = parseInt(readTarString(header, 124, 12).trim() || "0", 8);
    if (!Number.isSafeInteger(size) || size < 0) throw new ProvisionError("ARCHIVE_INVALID", `bad size for '${name}'`);

    const entryName = name.replace(/\/$/, "");
    if (!isSafeMemberPath(entryName)) throw new ProvisionError("ARCHIVE_UNSAFE_ENTRY", `rejected entry path '${name}'`);
    if (type !== "0" && type !== "5") throw new ProvisionError("ARCHIVE_UNSAFE_ENTRY", `rejected entry type '${type}' for '${name}'`);
    if (entries.has(entryName)) throw new ProvisionError("ARCHIVE_UNSAFE_ENTRY", `duplicate entry '${entryName}'`);

    const dataStart = offset + 512;
    if (dataStart + size > tar.length) throw new ProvisionError("ARCHIVE_INVALID", `truncated entry '${name}'`);
    if (type === "0") entries.set(entryName, tar.subarray(dataStart, dataStart + size));
    else entries.set(entryName, null);
    offset = dataStart + Math.ceil(size / 512) * 512;
  }
  return entries;
}

// Verifies the archive and returns the runtime bytes; never returns unverified bytes.
export function extractVerifiedRuntime(archive, record) {
  const archiveSha256 = sha256Hex(archive);
  if (archiveSha256 !== record.archiveSha256) {
    throw new ProvisionError("ARCHIVE_DIGEST_MISMATCH", `expected ${record.archiveSha256}, got ${archiveSha256}`);
  }
  const runtime = readTarEntries(gunzipSync(archive)).get(record.member);
  if (!runtime) throw new ProvisionError("ARCHIVE_MEMBER_MISSING", `'${record.member}' is not a regular file in the archive`);
  if (runtime.length !== record.byteLength) {
    throw new ProvisionError("RUNTIME_SIZE_MISMATCH", `expected ${record.byteLength} bytes, got ${runtime.length}`);
  }
  const runtimeSha256 = sha256Hex(runtime);
  if (runtimeSha256 !== record.sha256) {
    throw new ProvisionError("RUNTIME_DIGEST_MISMATCH", `expected ${record.sha256}, got ${runtimeSha256}`);
  }
  return runtime;
}

function stageAtomically(bytes, target) {
  mkdirSync(dirname(target), { recursive: true });
  const temporary = `${target}.provision-${process.pid}.tmp`;
  try {
    const fd = openSync(temporary, "wx", 0o755);
    try {
      writeSync(fd, bytes);
      fsyncSync(fd);
    } finally {
      closeSync(fd);
    }
    renameSync(temporary, target);
  } finally {
    rmSync(temporary, { force: true });
  }
}

async function download(url) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) throw new ProvisionError("DOWNLOAD_FAILED", `${url} answered ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > MAX_ARCHIVE_BYTES) throw new ProvisionError("DOWNLOAD_FAILED", `${url} exceeds ${MAX_ARCHIVE_BYTES} bytes`);
  return bytes;
}

async function main(args) {
  const platformArg = args.find((arg) => arg.startsWith("--platform="))?.slice("--platform=".length);
  const platform = platformArg ?? { linux: "linux", win32: "windows" }[process.platform];
  if (!platform) throw new ProvisionError("UNSUPPORTED_PLATFORM", `pass --platform=linux|windows (host is ${process.platform})`);

  const record = runtimeRecordFor(JSON.parse(readFileSync(manifestPath, "utf8")), platform);
  const target = resolve(repoRoot, record.stagedPath);

  if (existsSync(target)) {
    const staged = readFileSync(target);
    if (staged.length === record.byteLength && sha256Hex(staged) === record.sha256) {
      console.log(`${record.stagedPath} already matches the pinned SHA-256 ${record.sha256}`);
      return;
    }
    throw new ProvisionError("STAGED_RUNTIME_MISMATCH", `${record.stagedPath} exists but does not match the manifest; remove it and rerun`);
  }

  const runtime = extractVerifiedRuntime(await download(record.url), record);
  stageAtomically(runtime, target);
  console.log(`Staged ${record.stagedPath} (${record.byteLength} bytes, SHA-256 ${record.sha256}) from ${record.url}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
