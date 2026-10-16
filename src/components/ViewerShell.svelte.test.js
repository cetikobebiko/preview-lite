import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/svelte";
import { flushSync } from "svelte";
import ViewerShell from "./ViewerShell.svelte";
import { probeLog, probeValues } from "./ViewerShell.pdfPageProbe.svelte";
import { createViewerState } from "../stores/viewerState.svelte.js";
import { createSearchState } from "../stores/searchState.svelte.js";

vi.mock("./PdfPage.svelte", async () => import("./ViewerShell.pdfPageProbe.svelte"));

const PAGE_COUNT = 4;

function setup({ isPdf = true } = {}) {
  const viewer = createViewerState();
  viewer.pageOrder = Array.from({ length: PAGE_COUNT }, (_, index) => index);
  const searchState = createSearchState();
  const redaction = $state({ selectionMode: false, pendingRegions: [] });
  const sensitive = $state({ byPage: {} });
  const prepress = $state({ coveragePreview: null, processPreview: null, channels: {} });
  const documentSession = { activeSessionId: "session-1", contentGeneration: 1 };
  const getActivePrepressPayload = vi.fn((value, pageIndex, _s, _g, physicalPageIndex, key) =>
    value?.physicalPageIndex === physicalPageIndex && value.physicalPageIndex === pageIndex
      ? value[key] ?? null
      : null);

  render(ViewerShell, {
    props: {
      isPdf,
      viewer,
      annotation: { activeTool: "pointer", activeColor: "#fff" },
      documentSession,
      searchState,
      get spreads() { return viewer.spreads; },
      get physicalPageIndex() { return viewer.physicalPageIndex; },
      prepress,
      featureUi: () => ({ visible: true, runnable: true }),
      errorMsg: "",
      getActivePrepressPayload,
      getSensitiveDataHighlights: (pageIndex) => sensitive.byPage[pageIndex] ?? [],
      secureRedactionState: redaction,
      onSelectFile: vi.fn(), onPageIntersect: vi.fn(), onSearchHighlightReady: vi.fn(),
      onSessionSnapshot: vi.fn(), onCanvasMouseDown: vi.fn(), onCanvasMouseMove: vi.fn(), onCanvasMouseUp: vi.fn(),
    },
  });
  flushSync();
  return { viewer, searchState, redaction, sensitive, prepress, getActivePrepressPayload };
}

function latest(pageIndex, name) {
  return probeValues.get(`${pageIndex}:${name}`);
}

function updatesAfter(mutate) {
  probeLog.length = 0;
  mutate();
  flushSync();
  return probeLog.map(({ pageIndex, name }) => `${pageIndex}:${name}`).sort();
}

afterEach(() => {
  cleanup();
  probeLog.length = 0;
  probeValues.clear();
});

describe("ViewerShell per-page props", () => {
  it("gives pages without data one shared empty array", () => {
    setup();
    for (const name of ["searchMatches", "sensitiveDataHighlights", "pendingRedactionRegions"]) {
      const empty = latest(0, name);
      expect(empty).toEqual([]);
      for (let page = 1; page < PAGE_COUNT; page += 1) expect(latest(page, name)).toBe(empty);
    }
  });

  it("does not touch any page prop when only the current page changes in continuous mode", () => {
    const { viewer, getActivePrepressPayload } = setup();
    getActivePrepressPayload.mockClear();

    expect(updatesAfter(() => { viewer.currentPageIndex = 2; })).toEqual([]);
    expect(getActivePrepressPayload).toHaveBeenCalledTimes(2);
  });

  it("passes the current page in single-page mode", () => {
    const { viewer } = setup();

    updatesAfter(() => { viewer.isSinglePageMode = true; });
    expect(latest(0, "currentPage")).toBe(0);
    expect(updatesAfter(() => { viewer.currentPageIndex = 3; })).toEqual(
      Array.from({ length: PAGE_COUNT }, (_, page) => `${page}:currentPage`),
    );
    expect(latest(1, "currentPage")).toBe(3);
  });

  it("updates only the pages whose search matches changed", () => {
    const { searchState } = setup();
    const match = (index, isActive) => ({ index, isActive, pageIndex: 1, boxes: [] });

    expect(updatesAfter(() => { searchState.matchesByPage = { 1: [match(0, true)] }; }))
      .toEqual(["1:searchMatches"]);
    expect(latest(1, "searchMatches")).toEqual([match(0, true)]);

    expect(updatesAfter(() => {
      searchState.matchesByPage = { ...searchState.matchesByPage, 3: [match(1, false)] };
    })).toEqual(["3:searchMatches"]);

    expect(updatesAfter(() => { searchState.matchesByPage = {}; }))
      .toEqual(["1:searchMatches", "3:searchMatches"]);
    expect(latest(1, "searchMatches")).toBe(latest(0, "searchMatches"));
  });

  it("routes pending redaction regions and sensitive highlights to their pages", () => {
    const { redaction, sensitive } = setup();
    const region = {
      id: "r-1",
      pageIndex: 2,
      normalizedTopLeftRect: { x: 0.1, y: 0.1, width: 0.2, height: 0.1 },
      source: "manual_selection",
    };

    expect(updatesAfter(() => { redaction.pendingRegions = [region]; }))
      .toEqual(["2:pendingRedactionRegions"]);
    expect(latest(2, "pendingRedactionRegions")).toEqual([region]);

    const highlight = { x: 0.1, y: 0.2, width: 0.3, height: 0.05 };
    expect(updatesAfter(() => { sensitive.byPage = { 3: [highlight] }; }))
      .toEqual(["3:sensitiveDataHighlights"]);
    expect(latest(3, "sensitiveDataHighlights")).toEqual([highlight]);
  });

  it("gives a prepress preview only to the current physical page", () => {
    const { viewer, prepress } = setup();
    const result = { heatmapPngBase64: "AA==" };

    updatesAfter(() => {
      viewer.currentPageIndex = 1;
      prepress.coveragePreview = { physicalPageIndex: 1, result };
    });
    expect(latest(1, "approximateCoveragePreview")).toEqual(result);
    expect(latest(0, "approximateCoveragePreview")).toBeNull();
    expect(latest(2, "approximateCoveragePreview")).toBeNull();

    // Moving away from the previewed page drops it everywhere.
    updatesAfter(() => { viewer.currentPageIndex = 2; });
    for (let page = 0; page < PAGE_COUNT; page += 1) {
      expect(latest(page, "approximateCoveragePreview")).toBeNull();
    }
  });
});

describe("ViewerShell image canvas", () => {
  it("lays out the image canvas at its bitmap size times the committed scale", () => {
    const { viewer } = setup({ isPdf: false });
    viewer.activeImageSourcePath = "photo.png";
    flushSync();
    const canvas = document.querySelector(".page-wrapper canvas");
    expect(canvas.style.width).toBe("");

    viewer.imagePixelSize = { width: 800, height: 600 };
    flushSync();
    expect([canvas.style.width, canvas.style.height]).toEqual(["800px", "600px"]);

    viewer.scale = 1.5;
    flushSync();
    expect([canvas.style.width, canvas.style.height]).toEqual(["1200px", "900px"]);
  });
});

describe("ViewerShell zoom transform", () => {
  it("snaps the gesture transform back to scale(1) without a transition when the zoom commits", () => {
    const { viewer } = setup();
    const container = document.querySelector(".canvas-container");

    viewer.visualScale = 1.3;
    viewer.isZooming = true;
    flushSync();
    expect(container.style.transform).toBe("scale(1.3)");

    // Commit: the new scale is laid out in the same update that drops the transform.
    viewer.scale = 1.3;
    viewer.visualScale = 1;
    viewer.isZooming = false;
    flushSync();
    expect(container.style.transform).toBe("scale(1)");
    expect(container.style.transition).toBe("");
  });
});
