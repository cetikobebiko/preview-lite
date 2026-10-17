import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { gzipSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import {
  extractVerifiedRuntime,
  readTarEntries,
  runtimeRecordFor,
  sha256Hex,
} from "./provision-pdfium-runtime.mjs";

const manifest = JSON.parse(
  readFileSync(resolve(process.cwd(), "docs/engineering/platform-support.json"), "utf8"),
);

function tarHeader(name, { size = 0, type = "0" } = {}) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "utf8");
  header.write("0000755\0", 100);
  header.write("0000000\0", 108);
  header.write("0000000\0", 116);
  header.write(`${size.toString(8).padStart(11, "0")}\0`, 124);
  header.write("00000000000\0", 136);
  header.write(type, 156);
  header.write("ustar\0", 257);
  header.write("00", 263);
  header.fill(32, 148, 156);
  const checksum = header.reduce((sum, byte) => sum + byte, 0);
  header.write(`${checksum.toString(8).padStart(6, "0")}\0 `, 148);
  return header;
}

function tar(entries) {
  const blocks = entries.flatMap(({ name, data = Buffer.alloc(0), type }) => {
    const padding = Buffer.alloc((512 - (data.length % 512)) % 512);
    return [tarHeader(name, { size: data.length, type }), data, padding];
  });
  return Buffer.concat([...blocks, Buffer.alloc(1024)]);
}

function recordFor(archive, runtime, member = "lib/libpdfium.so") {
  return {
    archiveSha256: sha256Hex(archive),
    member,
    sha256: sha256Hex(runtime),
    byteLength: runtime.length,
  };
}

describe("PDFium runtime provisioning", () => {
  it("reads fully pinned Linux and Windows records from the manifest", () => {
    const linux = runtimeRecordFor(manifest, "linux");
    expect(linux.stagedPath).toBe("src-tauri/lib/libpdfium.so");
    expect(linux.member).toBe("lib/libpdfium.so");
    expect(linux.url).toMatch(/^https:\/\/github\.com\/bblanchon\/pdfium-binaries\/releases\/download\/chromium\/7961\/pdfium-linux-x64\.tgz$/);

    const windows = runtimeRecordFor(manifest, "windows");
    expect(windows.stagedPath).toBe("src-tauri/lib/pdfium.dll");
    expect(windows.member).toBe("bin/pdfium.dll");
    expect(windows.sha256).toMatch(/^[0-9a-f]{64}$/);
  });

  it("refuses to provision a record that is not pinned", () => {
    const unpinned = structuredClone(manifest);
    const linux = unpinned.platforms.find((p) => p.targetTriple === "x86_64-unknown-linux-gnu");
    linux.runtime.sha256 = null;
    linux.runtime.supplyChainStatus = "INCOMPLETE";
    expect(() => runtimeRecordFor(unpinned, "linux")).toThrow(/RUNTIME_METADATA_INCOMPLETE/);
    expect(() => runtimeRecordFor(manifest, "macos")).toThrow(/UNSUPPORTED_PLATFORM/);

    const traversal = structuredClone(manifest);
    traversal.platforms.find((p) => p.targetTriple === "x86_64-pc-windows-msvc").runtime.provenance.sourceArtifactMember = "../pdfium.dll";
    expect(() => runtimeRecordFor(traversal, "windows")).toThrow(/sourceArtifactMember/);
  });

  it("extracts only the pinned member after both digests match", () => {
    const runtime = Buffer.from("\x7fELF runtime bytes");
    const archive = gzipSync(tar([
      { name: "lib/", type: "5" },
      { name: "lib/libpdfium.so", data: runtime },
      { name: "LICENSE", data: Buffer.from("license") },
    ]));
    expect(extractVerifiedRuntime(archive, recordFor(archive, runtime)).equals(runtime)).toBe(true);
  });

  it("fails closed on an archive or runtime digest mismatch", () => {
    const runtime = Buffer.from("runtime");
    const archive = gzipSync(tar([{ name: "lib/libpdfium.so", data: runtime }]));
    const record = recordFor(archive, runtime);
    expect(() => extractVerifiedRuntime(archive, { ...record, archiveSha256: "0".repeat(64) })).toThrow(/ARCHIVE_DIGEST_MISMATCH/);
    expect(() => extractVerifiedRuntime(archive, { ...record, sha256: "0".repeat(64) })).toThrow(/RUNTIME_DIGEST_MISMATCH/);
    expect(() => extractVerifiedRuntime(archive, { ...record, byteLength: 1 })).toThrow(/RUNTIME_SIZE_MISMATCH/);
    expect(() => extractVerifiedRuntime(archive, { ...record, member: "bin/pdfium.dll" })).toThrow(/ARCHIVE_MEMBER_MISSING/);
  });

  it("rejects traversal, absolute paths, links, extension headers and duplicates", () => {
    const data = Buffer.from("x");
    expect(() => readTarEntries(tar([{ name: "../lib/libpdfium.so", data }]))).toThrow(/ARCHIVE_UNSAFE_ENTRY/);
    expect(() => readTarEntries(tar([{ name: "/lib/libpdfium.so", data }]))).toThrow(/ARCHIVE_UNSAFE_ENTRY/);
    expect(() => readTarEntries(tar([{ name: "lib/libpdfium.so", type: "2" }]))).toThrow(/ARCHIVE_UNSAFE_ENTRY/);
    expect(() => readTarEntries(tar([{ name: "lib/libpdfium.so", type: "1" }]))).toThrow(/ARCHIVE_UNSAFE_ENTRY/);
    expect(() => readTarEntries(tar([{ name: "PaxHeader", data, type: "x" }]))).toThrow(/ARCHIVE_UNSAFE_ENTRY/);
    expect(() => readTarEntries(tar([{ name: "a", data }, { name: "a", data }]))).toThrow(/duplicate/);
  });

  it("rejects a corrupted tar header", () => {
    const archive = tar([{ name: "lib/libpdfium.so", data: Buffer.from("x") }]);
    archive[0] ^= 0xff;
    expect(() => readTarEntries(archive)).toThrow(/ARCHIVE_INVALID/);
  });
});
