import { describe, expect, it, vi } from "vitest";
import { createViewerEditActions } from "./viewerEditActions.js";

function setup() {
  const viewer = { currentPageIndex: 1, pageOrder: [0, 1, 2], rotation: 0, flipH: false, flipV: false };
  const annotation = { annotationsMap: { 1: [{ id: "a" }] } };
  const render = vi.fn();
  const onToast = vi.fn();
  return { viewer, annotation, render, onToast, actions: createViewerEditActions({ viewer, annotation, render, onToast }) };
}

describe("viewer edit actions", () => {
  it("updates the authoritative annotation store before requesting a paint", () => {
    const { annotation, render, actions } = setup();
    expect(actions.undoLastAnnotation()).toBe(true);
    expect(annotation.annotationsMap[1]).toEqual([]);
    expect(render).toHaveBeenCalledWith({ silent: true });
  });

  it("keeps page index valid after a deletion and requests a rerender", () => {
    const { viewer, render, actions } = setup();
    actions.deletePage(2);
    expect(viewer.pageOrder).toEqual([0, 1]);
    expect(viewer.currentPageIndex).toBe(1);
    expect(render).toHaveBeenCalledWith();
  });

  it("applies visual transforms through the shared viewer store", () => {
    const { viewer, render, actions } = setup();
    actions.rotateLeft();
    actions.toggleFlipH();
    expect(viewer).toMatchObject({ rotation: 270, flipH: true });
    expect(render).toHaveBeenCalledTimes(2);
  });
});
