import { describe, expect, it, vi } from "vitest";
import { createViewerNavigationController } from "./viewerNavigationController.js";

function createFakeDocument(physicalIndices) {
  const elements = new Map(physicalIndices.map((index) => [`page-${index}`, { scrollIntoView: vi.fn() }]));
  return { elements, getElementById: (id) => elements.get(id) ?? null };
}

function setup({ pageOrder = [0, 1, 2], currentPageIndex = 1, pdf = false, document = null, area = null, now = () => 0 } = {}) {
  const viewer = { historyStack: [], forwardStack: [], currentPageIndex, pageOrder, scale: 1, visualScale: 1, isZooming: false, isBookMode: false, isSinglePageMode: false, renderGenerationId: 0, zoomOriginY: null };
  const commitZoom = vi.fn();
  const render = vi.fn();
  const controller = createViewerNavigationController({ viewer, viewerController: { commitZoom }, isPdf: () => pdf, getViewerArea: () => area, tick: vi.fn(async () => undefined), render, saveSettings: vi.fn(), onToast: vi.fn(), getDocument: () => document, now });
  return { viewer, commitZoom, render, controller };
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
