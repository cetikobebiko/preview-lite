import { describe, expect, it, vi } from "vitest";
import { createAnnotationCanvasController } from "./annotationCanvasController.js";

function setup(tool = "pen") {
  const annotation = {
    activeTool: tool, isDrawing: false, startPos: null, currentPoints: [], currentAnnoItem: null,
    annotationsMap: {}, strokeColor: "#fff", strokeSize: 2,
  };
  const viewer = { currentPageIndex: 2, isBookMode: false };
  const canvas = { width: 400, height: 200, getBoundingClientRect: () => ({ left: 10, top: 20, width: 200, height: 100 }) };
  const render = vi.fn();
  const controller = createAnnotationCanvasController({
    annotation, viewer, getCanvases: () => ({ left: canvas, right: null }), promptText: vi.fn(async () => "Not"), render,
  });
  const event = (x, y) => ({ target: canvas, clientX: x, clientY: y });
  return { annotation, viewer, controller, render, event };
}

describe("annotationCanvasController", () => {
  it("keeps the in-progress pen annotation in the store and commits it on pointer up", async () => {
    const { annotation, controller, render, event } = setup();
    await controller.handleMouseDown(event(20, 30));
    controller.handleMouseMove(event(30, 40));
    controller.handleMouseUp();
    expect(annotation.annotationsMap[2]).toEqual([expect.objectContaining({ tool_type: "pen", points: [{ x: 20, y: 20 }, { x: 40, y: 40 }] })]);
    expect(annotation.currentAnnoItem).toBeNull();
    expect(render).toHaveBeenCalledTimes(2);
  });

  it("uses the text prompt as an async boundary without a second annotation state copy", async () => {
    const { annotation, controller, event } = setup("text");
    await controller.handleMouseDown(event(20, 30));
    expect(annotation.annotationsMap[2]).toEqual([expect.objectContaining({ tool_type: "text", text: "Not" })]);
    expect(annotation.isDrawing).toBe(false);
  });
});
