#!/usr/bin/env node
/**
 * Artifact Manifest Consumer — 11I-F2
 *
 * Re-hashes the downloaded artifact directory and compares it, unconditionally,
 * against the producer manifest. This is the consumer half of the producer→consumer
 * SHA-256 identity contract (docs/engineering/11i-f-handoff.md §2/§9). The check
 * does NOT rely on GitHub's transfer-channel integrity; a mismatch fails closed.
 *
 * Rules (fail-closed):
 *  - every manifest entry must exist and match sha256 + byteLength
 *  - no unexpected file may be present in the download directory
 *  - any mismatch / missing file / extra file / IO error => exit 1
 *
 * Usage:
 *   node scripts/verify-artifact-identity.mjs --manifest <manifestPath> --dir <downloadDir>
 */

import { createHash } from "node:crypto";
import { readFileSync, existsSync, statSync, readdirSync } from "node:fs";
import { resolve, join, relative, normalize, sep } from "node:path";

function parseArgs() {
  const argv = process.argv.slice(2);
  const opts = { manifest: null, dir: null, help: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--manifest" && argv[i + 1]) opts.manifest = resolve(argv[++i]);
    else if (a === "--dir" && argv[i + 1]) opts.dir = resolve(argv[++i]);
    else if (a === "--help" || a === "-h") opts.help = true;
  }
  return opts;
}

function collectFiles(root, list = [], prefix = "") {
  for (const entry of readdirSync(join(root, prefix), { withFileTypes: true })) {
    const rel = prefix ? `${prefix}${sep}${entry.name}` : entry.name;
    const full = join(root, rel);
    if (entry.isDirectory()) collectFiles(root, list, rel);
    else if (entry.isFile()) list.push({ full, rel: normalize(rel).split(sep).join("/") });
    else if (entry.isSymbolicLink()) list.push({ full, rel: normalize(rel).split(sep).join("/"), symlink: true });
  }
  return list;
}

function hashSha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

function run() {
  const opts = parseArgs();
  if (opts.help) {
    console.log("Usage: node scripts/verify-artifact-identity.mjs --manifest <manifestPath> --dir <downloadDir>");
    process.exit(0);
  }

  const errors = [];
  if (!opts.manifest) errors.push("--manifest <manifestPath> is required");
  if (!opts.dir) errors.push("--dir <downloadDir> is required");
  if (errors.length) {
    console.error(`ERROR: ${errors.join("; ")}`);
    process.exit(1);
  }

  let manifest;
  try {
    manifest = JSON.parse(readFileSync(opts.manifest, "utf-8"));
  } catch (e) {
    console.error(`ERROR: cannot read manifest ${opts.manifest}: ${e.message}`);
    process.exit(1);
  }

  if (manifest.schemaVersion !== "1.0.0" || !Array.isArray(manifest.artifacts)) {
    console.error(`ERROR: invalid manifest schema (expected schemaVersion 1.0.0 with artifacts[]): ${opts.manifest}`);
    process.exit(1);
  }

  if (!existsSync(opts.dir) || !statSync(opts.dir).isDirectory()) {
    console.error(`ERROR: download directory not found or not a directory: ${opts.dir}`);
    process.exit(1);
  }

  const expected = new Map(manifest.artifacts.map((a) => [a.file, a]));
  const actualFiles = collectFiles(opts.dir);

  const failures = [];
  const seen = new Set();

  for (const f of actualFiles) {
    const rec = expected.get(f.rel);
    if (!rec) {
      failures.push(`Unexpected file present (not in manifest): ${f.rel}`);
      continue;
    }
    seen.add(f.rel);
    if (f.symlink) {
      failures.push(`Symbolic link rejected: ${f.rel}`);
      continue;
    }
    const buf = readFileSync(f.full);
    const sha = hashSha256(buf);
    if (sha !== rec.sha256) {
      failures.push(`SHA-256 mismatch: ${f.rel} (expected ${rec.sha256}, got ${sha})`);
    }
    if (buf.length !== rec.byteLength) {
      failures.push(`Byte length mismatch: ${f.rel} (expected ${rec.byteLength}, got ${buf.length})`);
    }
  }

  for (const [file] of expected) {
    if (!seen.has(file)) {
      failures.push(`Missing artifact file: ${file}`);
    }
  }

  if (manifest.artifacts.length !== actualFiles.length) {
    failures.push(
      `File count mismatch: manifest=${manifest.artifacts.length} download=${actualFiles.length}`,
    );
  }

  if (failures.length > 0) {
    console.error(`Artifact identity verification FAILED (${failures.length} failure(s)):`);
    for (const f of failures) console.error(`  - ${f}`);
    process.exit(1);
  }

  console.log(`Artifact identity verified: ${actualFiles.length} file(s) byte-identical to producer manifest.`);
  for (const f of actualFiles) {
    console.log(`  ${f.rel}  ${expected.get(f.rel).sha256}`);
  }
  process.exit(0);
}

run();