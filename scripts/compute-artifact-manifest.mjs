#!/usr/bin/env node
/**
 * Artifact Manifest Producer — 11I-F2
 *
 * Computes the SHA-256 identity of every file under an artifact directory and
 * writes a machine-readable manifest. The manifest is the producer half of the
 * producer→consumer byte-identity contract (docs/engineering/11i-f-handoff.md:
 * "the artifact that is smoke-tested is byte-identical to the artifact that was
 * produced").
 *
 * Design:
 *  - stdlib only, deterministic, offline
 *  - hashes every regular file (recursively) with a repo-relative path
 *  - fail-closed: no input == error; hash error == error; any IO error == exit 1
 *  - filename is NOT identity; sha256 + byteLength is
 *
 * Usage:
 *   node scripts/compute-artifact-manifest.mjs --dir <bundleDir> --output <manifestPath>
 *
 * Output manifest schema (1.0.0):
 *   {
 *     "schemaVersion": "1.0.0",
 *     "producerRef": "compute-artifact-manifest.mjs",
 *     "generatedAt": "<ISO-8601>",
 *     "artifacts": [ { "file": "rel/path", "sha256": "<64hex>", "byteLength": 123 } ]
 *   }
 */

import { createHash } from "node:crypto";
import {
  readFileSync,
  writeFileSync,
  existsSync,
  statSync,
  readdirSync,
} from "node:fs";
import { resolve, relative, join, normalize, sep } from "node:path";

function parseArgs() {
  const argv = process.argv.slice(2);
  const opts = { dir: null, output: null, help: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir" && argv[i + 1]) opts.dir = resolve(argv[++i]);
    else if (a === "--output" && argv[i + 1]) opts.output = resolve(argv[++i]);
    else if (a === "--help" || a === "-h") opts.help = true;
  }
  return opts;
}

// Recursively collect regular files under `root` (no symlink following).
function collectFiles(root, list = [], prefix = "") {
  for (const entry of readdirSync(join(root, prefix), { withFileTypes: true })) {
    const rel = prefix ? `${prefix}${sep}${entry.name}` : entry.name;
    const full = join(root, rel);
    if (entry.isDirectory()) {
      collectFiles(root, list, rel);
    } else if (entry.isFile()) {
      list.push({ full, rel: normalize(rel) });
    } else if (entry.isSymbolicLink()) {
      // Symlinks are not followed; reject structurally identical escape surface.
      throw new Error(`Symbolic link rejected (not a regular file): ${rel}`);
    }
  }
  return list;
}

function hashSha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

function run() {
  const opts = parseArgs();
  if (opts.help) {
    console.log("Usage: node scripts/compute-artifact-manifest.mjs --dir <bundleDir> --output <manifestPath>");
    console.log("Recursively hashes every regular file and writes the producer identity manifest.");
    process.exit(0);
  }

  const errors = [];
  if (!opts.dir) errors.push("--dir <bundleDir> is required");
  if (!opts.output) errors.push("--output <manifestPath> is required");
  if (errors.length) {
    console.error(`ERROR: ${errors.join("; ")}`);
    process.exit(1);
  }

  if (!existsSync(opts.dir) || !statSync(opts.dir).isDirectory()) {
    console.error(`ERROR: artifact directory not found or not a directory: ${opts.dir}`);
    process.exit(1);
  }

  let files;
  try {
    files = collectFiles(opts.dir);
  } catch (e) {
    console.error(`ERROR: failed to walk artifact directory: ${e.message}`);
    process.exit(1);
  }

  files.sort((a, b) => (a.rel < b.rel ? -1 : a.rel > b.rel ? 1 : 0));

  if (files.length === 0) {
    console.error(`ERROR: no files found under ${opts.dir} — refusing to emit an empty manifest`);
    process.exit(1);
  }

  const artifacts = [];
  for (const f of files) {
    const buf = readFileSync(f.full);
    artifacts.push({
      file: f.rel.split(sep).join("/"),
      sha256: hashSha256(buf),
      byteLength: buf.length,
    });
  }

  const manifest = {
    schemaVersion: "1.0.0",
    producerRef: "compute-artifact-manifest.mjs",
    generatedAt: new Date().toISOString(),
    artifacts,
  };

  writeFileSync(opts.output, JSON.stringify(manifest, null, 2) + "\n", "utf-8");
  console.log(`Artifact manifest written: ${opts.output}`);
  console.log(`  files: ${artifacts.length}`);
  for (const a of artifacts) {
    console.log(`  ${a.file}  sha256=${a.sha256}  bytes=${a.byteLength}`);
  }
  process.exit(0);
}

run();