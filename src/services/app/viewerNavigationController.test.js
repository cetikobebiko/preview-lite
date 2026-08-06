import { describe, expect, it, vi } from "vitest";
import { createViewerNavigationController } from "./viewerNavigationController.js";

function setup() {
  const viewer = { historyStack: [], forwardStack: [], currentPageIndex: 1, pageOrder: [0, 1, 2], scale: 1, visualScale: 1, isZooming: false, isBookMode: false, isSinglePageMode: false, renderGenerationId: 0 };
  const commitZoom = vi.fn();
  const controller = createViewerNavigationController({ viewer, viewerController: { commitZoom }, isPdf: () => false, getViewerArea: () => null, tick: vi.fn(async () => undefined), render: vi.fn(), saveSettings: vi.fn(), onToast: vi.fn() });
  return { viewer, commitZoom, controller };
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
