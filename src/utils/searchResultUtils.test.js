import { describe, expect, it } from "vitest";
import {
  claimHighlightScroll,
  clampSearchIndex,
  groupSearchMatchesByPage,
  normalizeSearchResponse,
  positionSearchBox,
  rotateSearchBox,
  sanitizeSearchBox,
  updateActiveSearchMatchesByPage,
  wrapSearchIndex,
} from "./searchResultUtils.js";

const BOX = { x: 0.1, y: 0.2, width: 0.3, height: 0.4 };

describe("search box geometry", () => {
  it.each([
    [0, { x: 0.1, y: 0.2, width: 0.3, height: 0.4 }],
    [90, { x: 0.4, y: 0.1, width: 0.4, height: 0.3 }],
    [180, { x: 0.6, y: 0.4, width: 0.3, height: 0.4 }],
    [270, { x: 0.2, y: 0.6, width: 0.4, height: 0.3 }],
  ])("%s° dönüşümünü nihai normalize yüzeye uygular", (rotation, expected) => {
    expect(rotateSearchBox(BOX, rotation)).toEqual(expected);
  });

  it("CSS koordinatlarını DPR uygulamadan hesaplar", () => {
    expect(
      positionSearchBox(BOX, {
        rotation: 90,
        pageCssWidth: 100,
        pageCssHeight: 200,
      }),
    ).toMatchObject({
      left: 40,
      top: 20,
      width: 40,
      height: 60,
      rotation: 90,
    });
  });

  it("sayfa dışına taşan kutuyu kırpar ve kaynağı değiştirmez", () => {
    const source = { x: -0.1, y: 0.9, width: 0.4, height: 0.4 };
    const snapshot = structuredClone(source);
    expect(sanitizeSearchBox(source)).toEqual({
      x: 0,
      y: 0.9,
      width: 0.3,
      height: 0.1,
    });
    expect(source).toEqual(snapshot);
  });

  it.each([
    [{ ...BOX, width: 0 }],
    [{ ...BOX, height: -1 }],
    [{ ...BOX, x: Number.NaN }],
    [{ ...BOX, y: Number.POSITIVE_INFINITY }],
    [null],
  ])("bozuk kutuyu reddeder", (box) => {
    expect(sanitizeSearchBox(box)).toBeNull();
  });
});

describe("search result normalization", () => {
  it("kutu kimliğini eşleşme altında korur ve kutusuz eşleşmeleri eler", () => {
    const response = {
      total_matches: 4,
      matches: [
        {
          global_match_index: 8,
          page_index: 0,
          rects: [BOX, { x: 0.5, y: 0.2, width: 0.2, height: 0.1 }],
          snippet: "first",
        },
        {
          global_match_index: 9,
          page_index: 1,
          rects: [{ ...BOX, width: 0 }],
          snippet: "invalid",
        },
        {
          global_match_index: 10,
          page_index: 2,
          rects: [],
          snippet: "empty",
        },
        {
          global_match_index: 11,
          page_index: 2,
          rects: [{ x: 0.2, y: 0.3, width: 0.2, height: 0.1 }],
          snippet: "last",
        },
      ],
    };

    const normalized = normalizeSearchResponse(response);
    expect(normalized.results).toEqual([
      {
        index: 0,
        rustIndex: 8,
        pageIndex: 0,
        boxes: [BOX, { x: 0.5, y: 0.2, width: 0.2, height: 0.1 }],
        text: "first",
      },
      {
        index: 1,
        rustIndex: 11,
        pageIndex: 2,
        boxes: [{ x: 0.2, y: 0.3, width: 0.2, height: 0.1 }],
        text: "last",
      },
    ]);
    expect(normalized.totalMatches).toBe(2);
    expect(normalized.stats).toEqual({
      rustTotalMatches: 4,
      rawMatches: 4,
      normalizedMatches: 2,
      droppedMatches: 2,
      droppedBoxes: 1,
    });
  });

  it("sayfa gruplamasında yalnız eski ve yeni aktif sayfayı yeniler", () => {
    const results = [
      { index: 0, pageIndex: 0, boxes: [BOX], text: "a" },
      { index: 1, pageIndex: 1, boxes: [BOX], text: "b" },
      { index: 2, pageIndex: 2, boxes: [BOX], text: "c" },
    ];
    const grouped = groupSearchMatchesByPage(results, 0);
    const updated = updateActiveSearchMatchesByPage(grouped, results, 0, 1);

    expect(updated[0][0].isActive).toBe(false);
    expect(updated[1][0].isActive).toBe(true);
    expect(updated[2]).toBe(grouped[2]);
  });

  it("indeksi sınırlar ve iki yönde sarar", () => {
    expect(clampSearchIndex(-1, 3)).toBe(0);
    expect(clampSearchIndex(8, 3)).toBe(2);
    expect(clampSearchIndex(4, 0)).toBe(0);
    expect(wrapSearchIndex(2, 1, 3)).toBe(0);
    expect(wrapSearchIndex(0, -1, 3)).toBe(2);
    expect(wrapSearchIndex(0, 1, 1)).toBe(0);
    expect(wrapSearchIndex(0, -1, 1)).toBe(0);
  });

  it("ikinci scroll'u request, frontend index ve sayfa anahtarıyla tekilleştirir", () => {
    const completed = new Set();
    expect(claimHighlightScroll(completed, 7, 2, 4)).toBe(true);
    expect(claimHighlightScroll(completed, 7, 2, 4)).toBe(false);
    expect(claimHighlightScroll(completed, 8, 2, 4)).toBe(true);
    expect(claimHighlightScroll(completed, 8, 3, 4)).toBe(true);
    expect(claimHighlightScroll(completed, 8, 3, 5)).toBe(true);
  });
});
