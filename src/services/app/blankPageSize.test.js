import { describe, expect, it, vi } from "vitest";
import { A4_BLANK_PAGE_SIZE, blankPageSizeFromNeighbor, resolveBlankPageSize } from "./blankPageSize.js";

describe("blankPageSizeFromNeighbor", () => {
  const sizes = [
    { widthPt: 612, heightPt: 792 },
    // A portrait page with /Rotate 90: the worker already reports it landscape.
    { widthPt: 792, heightPt: 612 },
  ];

  it("uses the neighbour's displayed size unchanged", () => {
    expect(blankPageSizeFromNeighbor(sizes, 0)).toEqual({ width: 612, height: 792 });
    expect(blankPageSizeFromNeighbor(sizes, 1)).toEqual({ width: 792, height: 612 });
  });

  it.each([
    ["no sizes", null, 0],
    ["missing index", sizes, 5],
    ["unreadable size", [null], 0],
    ["zero size", [{ widthPt: 0, heightPt: 0 }], 0],
    ["side over the backend limit", [{ widthPt: 20_000, heightPt: 792 }], 0],
  ])("falls back to A4 for %s", (_, list, index) => {
    expect(blankPageSizeFromNeighbor(list, index)).toEqual(A4_BLANK_PAGE_SIZE);
  });
});

describe("resolveBlankPageSize", () => {
  it("loads the session's page sizes for the given generation", async () => {
    const loadSizes = vi.fn(async () => [{ widthPt: 612, heightPt: 792 }, { widthPt: 842, heightPt: 595 }]);
    await expect(resolveBlankPageSize("s", 3, 1, { loadSizes })).resolves.toEqual({ width: 842, height: 595 });
    expect(loadSizes).toHaveBeenCalledWith("s", 3);
  });

  it("falls back to A4 when the sizes cannot be loaded", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const loadSizes = vi.fn(async () => { throw new Error("worker gone"); });
    await expect(resolveBlankPageSize("s", 3, 0, { loadSizes })).resolves.toEqual(A4_BLANK_PAGE_SIZE);
    expect(warn).toHaveBeenCalledOnce();
    warn.mockRestore();
  });
});
