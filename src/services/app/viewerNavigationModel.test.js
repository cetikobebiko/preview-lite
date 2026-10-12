import { describe, expect, it } from "vitest";
import {
  findViewportAnchor,
  parsePageInput,
  resolveAnchorScrollTop,
  selectMostVisiblePage,
  toDisplayPageIndex,
  toPhysicalPageIndex,
} from "./viewerNavigationModel.js";

describe("viewerNavigationModel index contract", () => {
  it("maps displayIndex <-> physicalIndex through a reordered pageOrder", () => {
    const pageOrder = [2, 0, 1];
    expect(toPhysicalPageIndex(pageOrder, 0)).toBe(2);
    expect(toPhysicalPageIndex(pageOrder, 2)).toBe(1);
    expect(toDisplayPageIndex(pageOrder, 2)).toBe(0);
    expect(toDisplayPageIndex(pageOrder, 1)).toBe(2);
  });

  it("keeps physical indices that exceed the order length after a local page removal", () => {
    expect(toPhysicalPageIndex([0, 2], 1)).toBe(2);
    expect(toDisplayPageIndex([0, 2], 1)).toBe(-1);
  });

  it("rejects out-of-range and non-integer indices", () => {
    expect(toPhysicalPageIndex([0, 1], -1)).toBe(-1);
    expect(toPhysicalPageIndex([0, 1], 2)).toBe(-1);
    expect(toPhysicalPageIndex([0, 1], 0.5)).toBe(-1);
    expect(toPhysicalPageIndex(null, 0)).toBe(-1);
    expect(toDisplayPageIndex([0, 1], -3)).toBe(-1);
  });
});

describe("parsePageInput", () => {
  it("converts a valid 1-based page number to a displayIndex", () => {
    expect(parsePageInput("1", 5)).toBe(0);
    expect(parsePageInput(" 5 ", 5)).toBe(4);
  });

  it("rejects empty, fractional, signed, garbage and out-of-range input", () => {
    for (const value of ["", "0", "6", "-1", "2.5", "3abc", "abc", null, undefined]) {
      expect(parsePageInput(value, 5)).toBe(-1);
    }
  });
});

describe("selectMostVisiblePage", () => {
  const order = [0, 1, 2, 3];

  it("selects the page with the highest intersection ratio", () => {
    const ratios = new Map([[1, 0.3], [2, 0.8]]);
    expect(selectMostVisiblePage(ratios, 1, order)).toBe(2);
  });

  it("keeps the current page inside the hysteresis band", () => {
    const ratios = new Map([[1, 0.5], [2, 0.53]]);
    expect(selectMostVisiblePage(ratios, 1, order)).toBe(1);
    ratios.set(2, 0.56);
    expect(selectMostVisiblePage(ratios, 1, order)).toBe(2);
  });

  it("switches immediately when the current page is no longer visible", () => {
    expect(selectMostVisiblePage(new Map([[3, 0.12]]), 0, order)).toBe(3);
  });

  it("keeps the current page when nothing is visible", () => {
    expect(selectMostVisiblePage(new Map(), 2, order)).toBe(2);
  });

  it("breaks ties by display order and ignores pages outside pageOrder", () => {
    const reordered = [3, 1, 0];
    const ratios = new Map([[0, 0.6], [1, 0.6], [2, 1]]);
    expect(selectMostVisiblePage(ratios, -1, reordered)).toBe(1);
  });
});

describe("zoom anchor geometry", () => {
  const pages = [
    { physicalIndex: 0, top: 0, height: 1000 },
    { physicalIndex: 1, top: 1024, height: 1000 },
    { physicalIndex: 2, top: 2048, height: 1000 },
  ];

  it("captures the page and in-page ratio at the viewport centre", () => {
    expect(findViewportAnchor(pages, 1124, 800)).toEqual({ physicalIndex: 1, offsetRatio: 0.5 });
  });

  it("falls back to the nearest page edge when the centre sits in a gap", () => {
    expect(findViewportAnchor(pages, 610, 800)).toEqual({ physicalIndex: 0, offsetRatio: 1 });
    expect(findViewportAnchor([], 0, 800)).toBeNull();
  });

  it("restores the same document point after the page grows with zoom", () => {
    const before = findViewportAnchor(pages, 1124, 800);
    const zoomedPage = { physicalIndex: 1, top: 2048, height: 2000 };
    const scrollTop = resolveAnchorScrollTop(before, zoomedPage, 800);
    expect(scrollTop).toBe(2648);
    expect(findViewportAnchor([zoomedPage], scrollTop, 800)).toEqual(before);
  });

  it("clamps the restored scrollTop to the scrollable range", () => {
    const anchor = { physicalIndex: 0, offsetRatio: 0.1 };
    expect(resolveAnchorScrollTop(anchor, { top: 0, height: 1000 }, 800)).toBe(0);
    expect(resolveAnchorScrollTop({ physicalIndex: 2, offsetRatio: 1 }, { top: 2048, height: 1000 }, 800, 2500)).toBe(2500);
  });
});
