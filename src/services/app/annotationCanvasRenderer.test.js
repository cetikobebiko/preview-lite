import { describe, expect, it, vi } from "vitest";
import { paintAnnotations } from "./annotationCanvasRenderer.js";

function createContext() {
  return Object.assign({}, ...["save", "restore", "beginPath", "moveTo", "lineTo", "stroke", "rect", "closePath", "fill", "fillText"].map((name) => ({ [name]: vi.fn() })));
}

describe("annotationCanvasRenderer", () => {
  it("paints committed and active authoritative annotation items", () => {
    const context = createContext();
    const annotation = {
      annotationsMap: { 1: [{ tool_type: "pen", color: "#fff", size: 2, points: [{ x: 1, y: 2 }, { x: 3, y: 4 }] }] },
      isDrawing: true, currentAnnoItem: { tool_type: "rectangle", color: "#000", size: 1, x: 1, y: 2, w: 3, h: 4 },
    };
    paintAnnotations({ canvas: { getContext: vi.fn(() => context) }, pageIndex: 1, annotation, currentPageIndex: 1 });
    expect(context.stroke).toHaveBeenCalledTimes(2);
    expect(context.rect).toHaveBeenCalledWith(1, 2, 3, 4);
  });

  it("does not paint the in-progress item for another page", () => {
    const context = createContext();
    paintAnnotations({ canvas: { getContext: vi.fn(() => context) }, pageIndex: 0, currentPageIndex: 1, annotation: { annotationsMap: {}, isDrawing: true, currentAnnoItem: { tool_type: "text", color: "#fff", size: 1, x: 1, y: 1, text: "x" } } });
    expect(context.save).not.toHaveBeenCalled();
  });
});
