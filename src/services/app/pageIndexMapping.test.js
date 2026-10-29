import { describe, expect, it } from "vitest";
import { pageMappingAfterDelete, pageMappingAfterImport, pageMappingAfterReorder, remapPageKeyedDrawings } from "./pageIndexMapping.js";

describe("page index mapping", () => {
  it("shifts later pages down and removes deleted ones", () => {
    const map = pageMappingAfterDelete([3, 1]);
    expect([0, 1, 2, 3, 4, 5].map(map)).toEqual([0, -1, 1, -1, 2, 3]);
    expect(map(-1)).toBe(-1);
    expect(map(1.5)).toBe(-1);
  });

  it("inverts a full permutation (newOrder[newIndex] = oldIndex)", () => {
    const map = pageMappingAfterReorder([2, 0, 3, 1]);
    expect([0, 1, 2, 3].map(map)).toEqual([1, 3, 0, 2]);
    expect(map(4)).toBe(-1);
  });

  it("re-keys drawings and their page_index, dropping deleted pages", () => {
    const drawings = { 0: [{ page_index: 0, id: "a" }], 1: [{ page_index: 1, id: "b" }], 2: [{ page_index: 2, id: "c" }], 3: [] };
    expect(remapPageKeyedDrawings(drawings, pageMappingAfterDelete([1]))).toEqual({
      0: [{ page_index: 0, id: "a" }],
      1: [{ page_index: 1, id: "c" }],
    });
    expect(remapPageKeyedDrawings(drawings, pageMappingAfterReorder([2, 0, 1]))).toEqual({
      0: [{ page_index: 0, id: "c" }],
      1: [{ page_index: 1, id: "a" }],
      2: [{ page_index: 2, id: "b" }],
    });
  });

  it("shifts pages after an insert and drops replaced ones", () => {
    const insert = pageMappingAfterImport(1, 2);
    expect([0, 1, 2].map(insert)).toEqual([0, 3, 4]);
    const replace = pageMappingAfterImport(1, 1, 2);
    expect([0, 1, 2, 3].map(replace)).toEqual([0, -1, -1, 2]);
    expect(replace(-1)).toBe(-1);
  });
});
