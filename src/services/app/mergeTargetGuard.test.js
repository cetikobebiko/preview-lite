import { describe, expect, it } from "vitest";
import { comparablePath, mergeTargetConflict } from "./mergeTargetGuard.js";

describe("comparablePath", () => {
  it("collapses separators, dot segments and trailing slashes", () => {
    expect(comparablePath("/a//b/./c/../d.pdf/", "linux")).toBe("/a/b/d.pdf");
    expect(comparablePath("/../a.pdf", "linux")).toBe("/a.pdf");
  });

  it("folds case on macOS and Windows but not on Linux", () => {
    expect(comparablePath("/Docs/A.pdf", "macos")).toBe("/docs/a.pdf");
    expect(comparablePath("/Docs/A.pdf", "linux")).toBe("/Docs/A.pdf");
  });

  it("treats decomposed and composed Unicode names as the same file", () => {
    const composed = "/Belgeler/\u00f6zet.pdf";
    const decomposed = "/Belgeler/o\u0308zet.pdf";
    expect(comparablePath(decomposed, "macos")).toBe(comparablePath(composed, "macos"));
  });

  it("normalizes Windows separators, drive case and verbatim prefixes", () => {
    expect(comparablePath("C:\\Users\\Me\\doc.pdf", "windows")).toBe("c:/users/me/doc.pdf");
    expect(comparablePath("\\\\?\\C:\\Users\\Me\\doc.pdf", "windows")).toBe("c:/users/me/doc.pdf");
    expect(comparablePath("\\\\?\\UNC\\server\\share\\doc.pdf", "windows")).toBe("//server/share/doc.pdf");
    expect(comparablePath("\\\\server\\share\\doc.pdf", "windows")).toBe("//server/share/doc.pdf");
  });

  it("returns an empty key for empty input", () => {
    expect(comparablePath("", "macos")).toBe("");
    expect(comparablePath(null, "macos")).toBe("");
  });
});

describe("mergeTargetConflict", () => {
  const options = { documentPath: "/Docs/open.pdf", inputPaths: ["/Docs/other.pdf"], platform: "macos" };

  it("accepts a distinct target", () => {
    expect(mergeTargetConflict("/Docs/merged.pdf", options)).toBeNull();
  });

  it("refuses the open document and every input path", () => {
    expect(mergeTargetConflict("/docs/OPEN.pdf", options)).toContain("açık belgenin kendi dosyasına");
    expect(mergeTargetConflict("/Docs/sub/../other.pdf", options)).toContain("birleştirilen PDF dosyalarından birinin");
  });

  it("is case-sensitive on Linux", () => {
    expect(mergeTargetConflict("/docs/open.pdf", { ...options, platform: "linux" })).toBeNull();
  });

  it("ignores an empty target or missing document path", () => {
    expect(mergeTargetConflict("", options)).toBeNull();
    expect(mergeTargetConflict("/Docs/open.pdf", { platform: "macos" })).toBeNull();
  });
});
