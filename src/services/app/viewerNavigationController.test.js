import { afterEach, describe, expect, it, vi } from "vitest";
import { createViewerController } from "./viewerController.js";
import { createViewerNavigationController } from "./viewerNavigationController.js";
import { MAX_SCALE, MIN_SCALE } from "./zoomLimits.js";
import { createPageLayoutProvider, VIEWER_ROW_LAYOUT } from "./pageLayoutModel.js";
import { estimateRenderedPageCssSize } from "../../utils/pageGeometry.js";

function createFakeDocument(physicalIndices) {
  const elements = new Map(physicalIndices.map((index) => [`page-${index}`, { scrollIntoView: vi.fn() }]));
  return { elements, getElementById: (id) => elements.get(id) ?? null };
}

function setup({ pageOrder = [0, 1, 2], currentPageIndex = 1, pdf = false, document = null, area = null, now = () => 0 } = {}) {
  const viewer = { historyStack: [], forwardStack: [], currentPageIndex, pageOrder, scale: 1, visualScale: 1, isZooming: false, isBookMode: false, isSinglePageMode: false, renderGenerationId: 0, zoomOriginY: null };
  const commitZoom = vi.fn();
  const cancelPendingZoom = vi.fn(() => true);
  const render = vi.fn();
  const saveSettings = vi.fn();
  const controller = createViewerNavigationController({ viewer, viewerController: { commitZoom, cancelPendingZoom }, isPdf: () => pdf, getViewerArea: () => area, tick: vi.fn(async () => undefined), render, saveSettings, onToast: vi.fn(), getDocument: () => document, now });
  return { viewer, commitZoom, cancelPendingZoom, render, saveSettings, controller };
}

describe("viewerNavigationController", () => {
  it("keeps history and forward transitions inside the viewer state", async () => {
    const { viewer, controller } = setup();
    await controller.jumpToPage(2); await controller.goBack();
    expect(viewer.currentPageIndex).toBe(1);
    expect(viewer.forwardStack).toEqual([2]);
  });
  it("hands debounced zoom rendering to the viewer controller", () => {
    const { viewer, commitZoom, controller } = setup();
    controller.zoomIn();
    expect(viewer.visualScale).toBeCloseTo(1.2);
    expect(commitZoom).toHaveBeenCalledWith(expect.objectContaining({ viewer, nextScale: 1.2 }));
  });
});

describe("viewerNavigationController with a reordered pageOrder", () => {
  const pageOrder = [2, 0, 1];

  it("jumpToPage takes a displayIndex and scrolls the physical DOM page", async () => {
    const document = createFakeDocument([0, 1, 2]);
    const { viewer, controller } = setup({ pageOrder, currentPageIndex: 0, pdf: true, document });
    expect(await controller.jumpToPage(1)).toBe(true);
    expect(viewer.currentPageIndex).toBe(1);
    expect(document.elements.get("page-0").scrollIntoView).toHaveBeenCalledWith({ behavior: "auto", block: "start" });
    expect(document.elements.get("page-1").scrollIntoView).not.toHaveBeenCalled();
  });

  it("jumpToPhysicalPage (outline, scan results) lands on the same physical page", async () => {
    const document = createFakeDocument([0, 1, 2]);
    const { viewer, controller } = setup({ pageOrder, currentPageIndex: 2, pdf: true, document });
    expect(await controller.jumpToPhysicalPage(2)).toBe(true);
    expect(viewer.currentPageIndex).toBe(0);
    expect(document.elements.get("page-2").scrollIntoView).toHaveBeenCalledTimes(1);
    expect(await controller.jumpToPhysicalPage(7)).toBe(false);
  });

  it("next/previous step in display order and scroll the matching physical page", () => {
    const document = createFakeDocument([0, 1, 2]);
    const { viewer, controller } = setup({ pageOrder, currentPageIndex: 0, pdf: true, document });
    controller.goToNextPage();
    expect(viewer.currentPageIndex).toBe(1);
    expect(document.elements.get("page-0").scrollIntoView).toHaveBeenCalledWith({ behavior: "smooth", block: "start" });
    controller.goToPrevPage();
    expect(viewer.currentPageIndex).toBe(0);
    expect(document.elements.get("page-2").scrollIntoView).toHaveBeenCalledWith({ behavior: "smooth", block: "start" });
    controller.goToPrevPage();
    expect(viewer.currentPageIndex).toBe(0);
  });

  it("back/forward return to the same physical pages after pageOrder changes", async () => {
    const { viewer, controller } = setup({ pageOrder: [0, 1, 2], currentPageIndex: 0, pdf: true, document: createFakeDocument([0, 1, 2]) });
    await controller.jumpToPage(2);
    expect(viewer.historyStack).toEqual([0]);
    viewer.pageOrder = [2, 1, 0];
    viewer.currentPageIndex = 0;
    await controller.goBack();
    expect(viewer.currentPageIndex).toBe(2);
    await controller.goForward();
    expect(viewer.currentPageIndex).toBe(0);
  });

  it("skips history entries whose physical page was removed", async () => {
    const { viewer, controller } = setup({ pageOrder: [0, 2], currentPageIndex: 1 });
    viewer.historyStack = [0, 1];
    await controller.goBack();
    expect(viewer.currentPageIndex).toBe(0);
    expect(viewer.historyStack).toEqual([]);
  });

  it("ignores invalid displayIndex values without touching history", async () => {
    const { viewer, controller } = setup({ pageOrder });
    expect(await controller.jumpToPage(3)).toBe(false);
    expect(await controller.jumpToPage(-1)).toBe(false);
    expect(viewer.currentPageIndex).toBe(1);
    expect(viewer.historyStack).toEqual([]);
  });
});

describe("viewerNavigationController page input", () => {
  it("scrolls a PDF to the entered page", async () => {
    const document = createFakeDocument([0, 1, 2]);
    const { viewer, controller, render } = setup({ pageOrder: [2, 0, 1], currentPageIndex: 0, pdf: true, document });
    const target = { value: "3" };
    expect(await controller.handlePageInputChange({ target })).toBe(true);
    expect(viewer.currentPageIndex).toBe(2);
    expect(document.elements.get("page-1").scrollIntoView).toHaveBeenCalledTimes(1);
    expect(viewer.historyStack).toEqual([2]);
    expect(render).not.toHaveBeenCalled();
  });

  it("restores the previous value for invalid input", async () => {
    const { viewer, controller } = setup({ currentPageIndex: 1 });
    for (const value of ["0", "4", "abc", "1.5", ""]) {
      const target = { value };
      expect(await controller.handlePageInputChange({ target })).toBe(false);
      expect(target.value).toBe("2");
    }
    expect(viewer.currentPageIndex).toBe(1);
  });
});

describe("viewerNavigationController current page detection", () => {
  it("selects the most visible page instead of the last reported one", () => {
    const { viewer, controller } = setup({ pageOrder: [0, 1, 2], currentPageIndex: 0 });
    controller.handlePageIntersect(0, 0.7);
    controller.handlePageIntersect(1, 0.3);
    expect(viewer.currentPageIndex).toBe(0);
    controller.handlePageIntersect(0, 0.25);
    controller.handlePageIntersect(1, 0.75);
    expect(viewer.currentPageIndex).toBe(1);
  });

  it("does not flicker between nearly equally visible pages", () => {
    const { viewer, controller } = setup({ pageOrder: [0, 1, 2], currentPageIndex: 0 });
    controller.handlePageIntersect(0, 0.5);
    controller.handlePageIntersect(1, 0.52);
    controller.handlePageIntersect(0, 0.49);
    expect(viewer.currentPageIndex).toBe(0);
  });

  it("reports the display index of the visible physical page in a reordered document", () => {
    const { viewer, controller } = setup({ pageOrder: [2, 0, 1], currentPageIndex: 0 });
    controller.handlePageIntersect(2, 0);
    controller.handlePageIntersect(1, 0.9);
    expect(viewer.currentPageIndex).toBe(2);
  });

  it("holds a programmatic target until it becomes the most visible page", () => {
    let time = 0;
    const document = createFakeDocument([0, 1, 2]);
    const { viewer, controller } = setup({ pageOrder: [0, 1, 2], currentPageIndex: 0, pdf: true, document, now: () => time });
    controller.handlePageIntersect(0, 1);
    controller.goToNextPage();
    controller.handlePageIntersect(1, 0.2);
    controller.handlePageIntersect(0, 0.8);
    expect(viewer.currentPageIndex).toBe(1);
    controller.handlePageIntersect(1, 0.9);
    controller.handlePageIntersect(0, 0.1);
    expect(viewer.currentPageIndex).toBe(1);
    time = 5000;
    controller.handlePageIntersect(0, 0.95);
    controller.handlePageIntersect(1, 0.05);
    expect(viewer.currentPageIndex).toBe(0);
  });
});

function createZoomFixture({ scrollTop, clientHeight = 800, pageHeight = 1000, gap = 24 }) {
  const layout = { pageHeight };
  const area = {
    scrollTop,
    clientHeight,
    clientTop: 0,
    get scrollHeight() { return 3 * (layout.pageHeight + gap); },
    getBoundingClientRect: () => ({ top: 100 }),
    querySelector: () => ({ getBoundingClientRect: () => ({ top: 100 - area.scrollTop }) }),
  };
  const elements = new Map([0, 1, 2].map((index) => [`page-${index}`, {
    getBoundingClientRect: () => ({ top: 100 - area.scrollTop + index * (layout.pageHeight + gap), height: layout.pageHeight }),
    scrollIntoView: vi.fn(),
  }]));
  return { area, layout, document: { getElementById: (id) => elements.get(id) ?? null } };
}

describe("viewerNavigationController keyboard zoom limits", () => {
  afterEach(() => vi.useRealTimers());

  it("clamps keyboard zoom to the shared scale bounds", () => {
    const { viewer, controller, saveSettings } = setup();
    for (let step = 0; step < 60; step += 1) controller.adjustZoom(0.1);
    expect(viewer.scale).toBe(MAX_SCALE);
    for (let step = 0; step < 60; step += 1) controller.adjustZoom(-0.1);
    expect(viewer.scale).toBe(MIN_SCALE);
    controller.adjustZoom(null);
    expect(viewer.scale).toBe(1);
    expect(saveSettings).toHaveBeenCalledTimes(121);
  });

  it("rounds keyboard steps to two decimals instead of accumulating float error", () => {
    const { viewer, controller } = setup();
    controller.adjustZoom(0.1);
    controller.adjustZoom(0.1);
    expect(viewer.scale).toBe(1.2);
    controller.adjustZoom(-0.1);
    controller.adjustZoom(-0.1);
    controller.adjustZoom(-0.1);
    expect(viewer.scale).toBe(0.9);
  });

  it("recovers a non-finite committed scale to 1", () => {
    const { viewer, controller } = setup();
    viewer.scale = Number.NaN;
    controller.adjustZoom(0.1);
    expect(viewer.scale).toBe(1);
  });

  it("during a gesture, steps from the scale on screen and replaces the pending commit", () => {
    const { viewer, controller, cancelPendingZoom, render, saveSettings } = setup();
    Object.assign(viewer, { scale: 2, visualScale: 1.1, isZooming: true });
    controller.adjustZoom(0.1);
    expect(cancelPendingZoom).toHaveBeenCalledTimes(1);
    expect(viewer).toMatchObject({ scale: 2.3, visualScale: 1, isZooming: false });
    expect(render).toHaveBeenCalledWith({ silent: true });
    expect(saveSettings).toHaveBeenCalledTimes(1);
  });

  it("a wheel gesture's deferred commit does not overwrite a keyboard zoom made during it", () => {
    vi.useFakeTimers();
    const listeners = new Map();
    const element = { addEventListener: (name, handler) => listeners.set(name, handler), removeEventListener: () => {} };
    const viewerController = createViewerController({
      setActiveRenderDocument: async () => {},
      renderSession: { snapshot: () => ({ contentGeneration: 1 }), activate: () => {}, clear: () => {} },
    });
    const viewer = { historyStack: [], forwardStack: [], currentPageIndex: 0, pageOrder: [0], scale: 1, visualScale: 1, isZooming: false, zoomOriginY: null };
    const onRender = vi.fn();
    const onZoomCommit = vi.fn();
    const navigation = createViewerNavigationController({ viewer, viewerController, isPdf: () => false, getViewerArea: () => null, tick: async () => {}, render: onRender, saveSettings: vi.fn(), onToast: vi.fn() });
    viewerController.attachViewport({ element, viewer, hasDocument: () => true, isMounted: () => true, onRender, onZoomCommit });
    listeners.get("wheel")({ ctrlKey: true, deltaY: -5, preventDefault: () => {} });
    listeners.get("wheel")({ ctrlKey: true, deltaY: -5, preventDefault: () => {} });
    expect(viewer.isZooming).toBe(true);
    navigation.adjustZoom(0.1);
    // e^(2 × 0.05) = 1.105 on screen, + 0.1 → 1.21.
    expect(viewer).toMatchObject({ scale: 1.21, visualScale: 1, isZooming: false });
    vi.advanceTimersByTime(1000);
    expect(viewer.scale).toBe(1.21);
    expect(onRender).toHaveBeenCalledTimes(1);
    expect(onZoomCommit).not.toHaveBeenCalled();
    viewerController.stop();
  });
});

describe("viewerNavigationController zoom anchoring", () => {
  it("button zoom keeps the viewport-centre document point in place", async () => {
    const fixture = createZoomFixture({ scrollTop: 1124 });
    const { viewer, commitZoom, controller, render } = setup({ pdf: true, document: fixture.document, area: fixture.area });
    controller.zoomIn();
    expect(viewer.zoomOriginY).toBe(1524);
    const { onRender } = commitZoom.mock.calls[0][0];
    viewer.scale = 2;
    viewer.visualScale = 1;
    viewer.isZooming = false;
    fixture.layout.pageHeight = 2000;
    onRender({ silent: true });
    expect(render).toHaveBeenCalledWith({ silent: true });
    await new Promise((resolve) => setTimeout(resolve, 0));
    // Page 1 now starts at 2024; its midpoint (2024 + 1000) sits at the viewport centre.
    expect(fixture.area.scrollTop).toBe(2624);
  });

  it("wheel hooks capture once per gesture and restore after commit", async () => {
    const fixture = createZoomFixture({ scrollTop: 0 });
    const { viewer, controller } = setup({ pdf: true, document: fixture.document, area: fixture.area });
    controller.captureZoomAnchor();
    viewer.scale = 1.5;
    fixture.layout.pageHeight = 1500;
    await controller.restoreZoomAnchor();
    // Centre was page 0 at 40%; it maps to 600px of the 1500px page.
    expect(fixture.area.scrollTop).toBe(200);
    fixture.area.scrollTop = 999;
    await controller.restoreZoomAnchor();
    expect(fixture.area.scrollTop).toBe(999);
  });

  it("keyboard zoom keeps the anchor too", async () => {
    const fixture = createZoomFixture({ scrollTop: 1124 });
    const { viewer, controller } = setup({ pdf: true, document: fixture.document, area: fixture.area });
    const pending = controller.adjustZoom(1);
    expect(viewer.scale).toBe(2);
    fixture.layout.pageHeight = 2000;
    await pending;
    expect(fixture.area.scrollTop).toBe(2624);
  });
});

/**
 * A viewport where no page is mounted: positions come only from the layout model.
 * The page list starts 32px below the scroll content top (viewer padding).
 */
function createModelFixture({ scrollTop = 0, clientHeight = 800, pageHeight = 1000, gap = 24, pageCount = 50 } = {}) {
  const layout = { pageHeight, order: Array.from({ length: pageCount }, (_, index) => index) };
  const area = {
    scrollTop,
    clientHeight,
    clientTop: 0,
    get scrollHeight() { return 32 + layout.order.length * (layout.pageHeight + gap); },
    getBoundingClientRect: () => ({ top: 100 }),
    querySelector: (selector) => (selector === ".pdf-continuous-scroll"
      ? { getBoundingClientRect: () => ({ top: 100 + 32 - area.scrollTop }) }
      : null),
  };
  const getPageLayout = vi.fn(() => ({
    pageRect: (physicalIndex) => {
      const row = layout.order.indexOf(physicalIndex);
      return row < 0 ? null : { physicalIndex, top: row * (layout.pageHeight + gap), height: layout.pageHeight };
    },
  }));
  return { area, layout, getPageLayout, document: { getElementById: () => null } };
}

function setupWithModel(fixture, options = {}) {
  const viewer = { historyStack: [], forwardStack: [], currentPageIndex: 0, pageOrder: fixture.layout.order, scale: 1, visualScale: 1, isZooming: false, isBookMode: false, isSinglePageMode: false, renderGenerationId: 0, zoomOriginY: null, ...options };
  const commitZoom = vi.fn();
  const render = vi.fn();
  const controller = createViewerNavigationController({
    viewer, viewerController: { commitZoom, cancelPendingZoom: vi.fn(() => false) }, isPdf: () => true, getViewerArea: () => fixture.area,
    tick: vi.fn(async () => undefined), render, saveSettings: vi.fn(), onToast: vi.fn(),
    getDocument: () => fixture.document, getPageLayout: fixture.getPageLayout, now: () => 0,
  });
  return { viewer, controller, render, commitZoom };
}

describe("viewerNavigationController with pages that are not mounted", () => {
  it("jumps to a page through the layout model", async () => {
    const fixture = createModelFixture();
    const { viewer, controller } = setupWithModel(fixture);
    expect(await controller.jumpToPage(40)).toBe(true);
    expect(viewer.currentPageIndex).toBe(40);
    expect(fixture.area.scrollTop).toBe(32 + 40 * 1024);
  });

  it("centres a page for search and keeps the list top as origin while scrolled", () => {
    const fixture = createModelFixture({ scrollTop: 5000 });
    const { controller } = setupWithModel(fixture);
    expect(controller.scrollToPhysicalPage(10, { block: "center" })).toBe(true);
    expect(fixture.area.scrollTop).toBe(32 + 10 * 1024 + 500 - 400);
  });

  it("prefers the mounted element and reports pages it cannot locate", () => {
    const fixture = createModelFixture();
    const element = { scrollIntoView: vi.fn() };
    fixture.document = { getElementById: (id) => (id === "page-3" ? element : null) };
    const { controller } = setupWithModel(fixture);
    expect(controller.scrollToPhysicalPage(3, { block: "center" })).toBe(true);
    expect(element.scrollIntoView).toHaveBeenCalledWith({ behavior: "auto", block: "center" });
    expect(fixture.area.scrollTop).toBe(0);
    expect(controller.scrollToPhysicalPage(99)).toBe(false);
  });

  it("keeps the zoom anchor from model positions", async () => {
    // Viewport centre at 32 + 2 * 1024 + 400: page 2 at 40%.
    const fixture = createModelFixture({ scrollTop: 32 + 2 * 1024 });
    const { viewer, controller } = setupWithModel(fixture);
    controller.captureZoomAnchor();
    viewer.scale = 2;
    fixture.layout.pageHeight = 2000;
    await controller.restoreZoomAnchor();
    expect(fixture.area.scrollTop).toBe(32 + 2 * 2024 + 800 - 400);
  });

  it("keeps the centre page in place when book mode regroups the rows", async () => {
    const fixture = createModelFixture({ scrollTop: 32 + 10 * 1024 });
    const { viewer, controller, render } = setupWithModel(fixture);
    const before = fixture.area.scrollTop;
    fixture.getPageLayout.mockImplementation(() => ({
      pageRect: (physicalIndex) => (viewer.isBookMode
        ? { physicalIndex, top: Math.floor(physicalIndex / 2) * 1024, height: 1000 }
        : { physicalIndex, top: physicalIndex * 1024, height: 1000 }),
    }));
    await controller.toggleBookMode();
    expect(viewer.isBookMode).toBe(true);
    expect(render).toHaveBeenCalled();
    // The viewport centre was 40% into page 10; in book mode page 10 sits on row 5.
    expect(before).toBe(32 + 10 * 1024);
    expect(fixture.area.scrollTop).toBe(32 + 5 * 1024);
  });

  it("does not scroll to a guessed position while page sizes are unknown", async () => {
    const fixture = createModelFixture({ scrollTop: 700 });
    fixture.getPageLayout.mockImplementation(() => null);
    const { viewer, controller } = setupWithModel(fixture);
    expect(controller.scrollToPhysicalPage(10, { block: "center" })).toBe(false);
    expect(await controller.jumpToPage(40)).toBe(true);
    expect(viewer.currentPageIndex).toBe(40);
    expect(fixture.area.scrollTop).toBe(700);
  });

  it("anchors zoom on mounted pages only while page sizes are unknown", async () => {
    // Page 3 is mounted at 2000..3000 in content coordinates; the rest are unknown.
    const fixture = createModelFixture({ scrollTop: 2100 });
    fixture.getPageLayout.mockImplementation(() => null);
    let mountedTop = 2000;
    let mountedHeight = 1000;
    const element = { getBoundingClientRect: () => ({ top: 100 + mountedTop - fixture.area.scrollTop, height: mountedHeight }) };
    fixture.document = { getElementById: (id) => (id === "page-3" ? element : null) };
    const { viewer, controller } = setupWithModel(fixture);
    controller.captureZoomAnchor();
    viewer.scale = 2;
    mountedTop = 4000;
    mountedHeight = 2000;
    await controller.restoreZoomAnchor();
    // Centre was 2500 (50% into page 3); it stays 50% into page 3 after the zoom.
    expect(fixture.area.scrollTop).toBe(4000 + 1000 - 400);
  });

  it("scrolls by the real layout provider only once page sizes are loaded", () => {
    const fixture = createModelFixture({ pageCount: 3 });
    let sizes = null;
    const viewer = { historyStack: [], forwardStack: [], currentPageIndex: 0, pageOrder: [0, 1, 2], spreads: [[0], [1], [2]], scale: 1, rotation: 0, visualScale: 1, isZooming: false, isBookMode: false, isSinglePageMode: false, renderGenerationId: 0, zoomOriginY: null };
    const getPageLayout = createPageLayoutProvider({
      viewer,
      documentSession: { activeSessionId: "s1", contentGeneration: 1 },
      peekPageSizes: () => sizes,
      getDevicePixelRatio: () => 2,
    });
    const controller = createViewerNavigationController({
      viewer, viewerController: { commitZoom: vi.fn(), cancelPendingZoom: vi.fn(() => false) }, isPdf: () => true, getViewerArea: () => fixture.area,
      tick: vi.fn(async () => undefined), render: vi.fn(), saveSettings: vi.fn(), onToast: vi.fn(),
      getDocument: () => fixture.document, getPageLayout, now: () => 0,
    });
    expect(controller.scrollToPhysicalPage(2)).toBe(false);
    expect(fixture.area.scrollTop).toBe(0);
    const A3 = { widthPt: 841.89, heightPt: 1190.551 };
    sizes = [A3, A3, A3];
    expect(controller.scrollToPhysicalPage(2)).toBe(true);
    const rowPitch = Math.max(VIEWER_ROW_LAYOUT.minRowHeight, estimateRenderedPageCssSize({ pageSize: A3, scale: 1, devicePixelRatio: 2 }).height + VIEWER_ROW_LAYOUT.pageGapContinuous) + VIEWER_ROW_LAYOUT.rowGap;
    expect(fixture.area.scrollTop).toBe(32 + 2 * rowPitch);
  });

  it("leaves a running zoom gesture's anchor alone", async () => {
    const fixture = createModelFixture({ scrollTop: 3000 });
    const { viewer, controller } = setupWithModel(fixture, { isZooming: true });
    const mutate = vi.fn();
    await controller.preserveViewportAnchor(mutate);
    expect(mutate).toHaveBeenCalledTimes(1);
    expect(fixture.area.scrollTop).toBe(3000);
    expect(fixture.getPageLayout).not.toHaveBeenCalled();
    expect(viewer.isZooming).toBe(true);
  });
});
