import { describe, expect, it } from "vitest";
import { OPENABLE_FILE_EXTENSIONS, fileBaseName, isOpenableFilePath } from "./openableFiles.js";

describe("openableFiles", () => {
  it.each([
    ["/docs/rapor.pdf", true],
    ["/docs/RAPOR.PDF", true],
    ["C:\\Scans\\page.jpeg", true],
    ["/tmp/photo.webp", true],
    ["/tmp/drawing.svg", true],
    ["/tmp/notes.txt", false],
    ["/tmp/folder", false],
    ["/tmp/.pdf", false],
    ["/tmp/archive.pdf.zip", false],
    ["/tmp/dir.pdf/notes", false],
    ["", false],
    [null, false],
  ])("treats %s as openable: %s", (path, expected) => {
    expect(isOpenableFilePath(path)).toBe(expected);
  });

  it("matches the open dialog's PDF and image extensions", () => {
    expect(OPENABLE_FILE_EXTENSIONS).toEqual(["pdf", "png", "jpg", "jpeg", "webp", "bmp", "svg"]);
    expect(Object.isFrozen(OPENABLE_FILE_EXTENSIONS)).toBe(true);
  });

  it("takes the base name of POSIX and Windows paths", () => {
    expect(fileBaseName("/docs/Belgeler/rapor.pdf")).toBe("rapor.pdf");
    expect(fileBaseName("C:\\Scans\\page.png")).toBe("page.png");
  });
});
