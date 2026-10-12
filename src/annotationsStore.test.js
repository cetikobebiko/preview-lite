import { beforeEach, describe, expect, it } from "vitest";
import { get } from "svelte/store";
import {
  addHighlight,
  annotationsStore,
  bindHighlightSession,
  clearHighlights,
  highlightsForPage,
  remapHighlightPages,
  resetHighlightsForTests,
  rotateHighlightPage,
  rotateNormalizedRect,
} from "./annotationsStore.js";
import { pageMappingAfterDelete, pageMappingAfterReorder } from "./services/app/pageIndexMapping.js";

const rect = { x: 0.1, y: 0.2, width: 0.3, height: 0.05 };
const pagesOf = () => get(annotationsStore).highlights.map((highlight) => highlight.pageIndex);

beforeEach(() => resetHighlightsForTests());

describe("session-scoped highlight store", () => {
  it("rejects highlights for a session the store is not bound to", () => {
    expect(addHighlight("doc-a", 0, rect)).toBe(false);
    bindHighlightSession("doc-a");
    expect(addHighlight("doc-b", 0, rect)).toBe(false);
    expect(addHighlight("", 0, rect)).toBe(false);
    expect(addHighlight("doc-a", 0, rect, "#00ff00")).toBe(true);
    expect(get(annotationsStore).highlights).toEqual([expect.objectContaining({ pageIndex: 0, rect, color: "#00ff00" })]);
  });

  it("drops the previous document's highlights when another session is bound", () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 2, rect);
    expect(bindHighlightSession("doc-a")).toBe(false);
    expect(pagesOf()).toEqual([2]);

    bindHighlightSession("doc-b");
    expect(get(annotationsStore)).toEqual({ sessionId: "doc-b", highlights: [] });
    expect(highlightsForPage(get(annotationsStore), "doc-b", 2)).toEqual([]);
  });

  it("never exposes highlights to a page of a different session even before rebinding", () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 1, rect);
    const store = get(annotationsStore);
    expect(highlightsForPage(store, "doc-a", 1)).toHaveLength(1);
    expect(highlightsForPage(store, "doc-b", 1)).toEqual([]);
    expect(highlightsForPage(store, "", 1)).toEqual([]);
  });

  it("clears every highlight when the active document becomes an image or closes", () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 0, rect);
    bindHighlightSession(null);
    expect(get(annotationsStore)).toEqual({ sessionId: null, highlights: [] });
  });

  it("moves highlights with their page after a backend page deletion", () => {
    bindHighlightSession("doc-a");
    [0, 1, 2, 3].forEach((pageIndex) => addHighlight("doc-a", pageIndex, rect));
    expect(remapHighlightPages("doc-a", pageMappingAfterDelete([1]))).toBe(true);
    expect(pagesOf()).toEqual([0, 1, 2]);
    expect(highlightsForPage(get(annotationsStore), "doc-a", 1)).toHaveLength(1);
  });

  it("moves highlights with their page after a backend reorder", () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 0, rect, "#000000");
    addHighlight("doc-a", 2, rect, "#222222");
    // New document order: old page 2, old page 0, old page 1.
    remapHighlightPages("doc-a", pageMappingAfterReorder([2, 0, 1]));
    const store = get(annotationsStore);
    expect(highlightsForPage(store, "doc-a", 0).map((h) => h.color)).toEqual(["#222222"]);
    expect(highlightsForPage(store, "doc-a", 1).map((h) => h.color)).toEqual(["#000000"]);
    expect(highlightsForPage(store, "doc-a", 2)).toEqual([]);
  });

  it("ignores a remap issued for another session", () => {
    bindHighlightSession("doc-b");
    addHighlight("doc-b", 3, rect);
    expect(remapHighlightPages("doc-a", pageMappingAfterDelete([0]))).toBe(false);
    expect(pagesOf()).toEqual([3]);
  });

  it("clears one page or all pages within the bound session", () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 0, rect);
    addHighlight("doc-a", 1, rect);
    clearHighlights(0);
    expect(pagesOf()).toEqual([1]);
    clearHighlights();
    expect(get(annotationsStore)).toEqual({ sessionId: "doc-a", highlights: [] });
  });

  it("rotates a normalized rect clockwise with the page", () => {
    const r = { x: 0.1, y: 0.2, width: 0.3, height: 0.05 };
    // The page's top-left corner moves to the top-right after a clockwise quarter turn.
    expect(rotateNormalizedRect({ x: 0, y: 0, width: 0.1, height: 0.2 }, 90)).toEqual({ x: 0.8, y: 0, width: 0.2, height: 0.1 });
    const expectRectClose = (actual, expected) => {
      for (const key of Object.keys(expected)) expect(actual[key]).toBeCloseTo(expected[key]);
    };
    expectRectClose(rotateNormalizedRect(r, 180), { x: 0.6, y: 0.75, width: 0.3, height: 0.05 });
    expectRectClose(rotateNormalizedRect(rotateNormalizedRect(r, 90), -90), r);
    let turned = r;
    for (let i = 0; i < 4; i += 1) turned = rotateNormalizedRect(turned, 90);
    expectRectClose(turned, r);
    expect(rotateNormalizedRect(r, 270)).toEqual(rotateNormalizedRect(r, -90));
  });

  it("rotates only the highlights of the rotated page of the bound session", () => {
    bindHighlightSession("doc-a");
    addHighlight("doc-a", 0, { x: 0, y: 0, width: 0.1, height: 0.2 });
    addHighlight("doc-a", 1, { x: 0, y: 0, width: 0.1, height: 0.2 });
    expect(rotateHighlightPage("doc-b", 0, 90)).toBe(false);
    expect(rotateHighlightPage("doc-a", 0, 45)).toBe(false);
    expect(rotateHighlightPage("doc-a", 0, 90)).toBe(true);
    const store = get(annotationsStore);
    expect(highlightsForPage(store, "doc-a", 0)[0].rect).toEqual({ x: 0.8, y: 0, width: 0.2, height: 0.1 });
    expect(highlightsForPage(store, "doc-a", 1)[0].rect).toEqual({ x: 0, y: 0, width: 0.1, height: 0.2 });
  });
});
