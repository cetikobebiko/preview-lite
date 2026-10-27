import { describe, expect, it } from "vitest";
import { buildCompareHighlightsByPage, currentSide, otherSide, selectCompareHighlights } from "./documentCompare.js";

const rect = (x, y) => ({ x, y, width: 0.1, height: 0.02 });
const side = (pageIndex, rects, near = null, intrinsicRotation = 0) => ({ text: "", textTruncated: false, pageIndex, intrinsicRotation, rects, near });

const result = (currentIsOld) => ({
  currentIsOld,
  totalChanges: 2,
  oldWords: 4,
  newWords: 5,
  changes: [
    { kind: "replaced", old: side(0, [rect(0.1, 0.1)]), new: side(1, [rect(0.2, 0.2)]) },
    { kind: "inserted", old: side(2, [], rect(0.3, 0.3)), new: side(3, [rect(0.4, 0.4)]) },
  ],
});

describe("document compare highlights", () => {
  it("picks the open document's side of each change", () => {
    const change = result(true).changes[0];
    expect(currentSide(change, true)).toBe(change.old);
    expect(otherSide(change, true)).toBe(change.new);
    expect(currentSide(change, false)).toBe(change.new);
  });

  it("highlights the open document's words and marks words only in the other file", () => {
    expect(buildCompareHighlightsByPage(result(false))).toEqual({
      1: [{ ...rect(0.2, 0.2), intrinsicRotation: 0, kind: "replaced", marker: false, changeIndex: 0 }],
      3: [{ ...rect(0.4, 0.4), intrinsicRotation: 0, kind: "inserted", marker: false, changeIndex: 1 }],
    });
    const asOld = buildCompareHighlightsByPage(result(true));
    expect(asOld[2]).toEqual([{ ...rect(0.3, 0.3), intrinsicRotation: 0, kind: "inserted", marker: true, changeIndex: 1 }]);
    expect(buildCompareHighlightsByPage(null)).toEqual({});
  });

  it("rotates highlights with the page and the viewer and keeps their kind", () => {
    const byPage = { 0: [{ ...rect(0.1, 0.2), intrinsicRotation: 90, kind: "deleted", marker: false, changeIndex: 4 }] };
    const [unrotated] = selectCompareHighlights(byPage, 0, 270);
    expect(unrotated).toMatchObject({ x: 0.1, y: 0.2, kind: "deleted", changeIndex: 4 });
    const [quarter] = selectCompareHighlights(byPage, 0, 0);
    expect(quarter).toMatchObject({ x: 0.78, y: 0.1, width: 0.02, height: 0.1, kind: "deleted" });
    expect(selectCompareHighlights(byPage, 1, 0)).toEqual([]);
  });
});
