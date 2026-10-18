import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { addAnnotation, getPageAnnotations } from "../pdfState.js";
import AnnotationLayer from "./AnnotationLayer.svelte";

vi.mock("../pdfState.js", () => ({
  getPageAnnotations: vi.fn(),
  addAnnotation: vi.fn(),
  deleteAnnotation: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(),
}));
vi.mock("../services/asyncPrompt.svelte.js", () => ({
  requireAsyncPromptCoordinator: vi.fn(() => ({
    request: vi.fn(),
  })),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("AnnotationLayer session identity", () => {
  it("loads annotations with sessionId and content generation", async () => {
    getPageAnnotations.mockResolvedValue([]);
    render(AnnotationLayer, {
      props: {
        sessionId: "annotation-session",
        contentGeneration: 5,
        pageIndex: 2,
      },
    });

    await waitFor(() => {
      expect(getPageAnnotations).toHaveBeenCalledWith(
        "annotation-session",
        5,
        2,
      );
    });
  });

  it("does not paint annotations returned for an old session", async () => {
    let resolveOld;
    getPageAnnotations
      .mockImplementationOnce(
        () => new Promise((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValueOnce([]);

    const view = render(AnnotationLayer, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageIndex: 0,
        pageWidth: 100,
        pageHeight: 100,
      },
    });
    await waitFor(() => expect(getPageAnnotations).toHaveBeenCalledTimes(1));

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
    });
    await waitFor(() => expect(getPageAnnotations).toHaveBeenCalledTimes(2));

    resolveOld([
      {
        id: "old-annotation",
        annot_type: "Highlight",
        normalized_rect: { x: 0, y: 0, width: 0.1, height: 0.1 },
        color: "#ffff00",
      },
    ]);
    await Promise.resolve();
    expect(view.queryByTitle("Highlight")).toBeNull();
  });
});

/**
 * jsdom layout yapmaz; katmanın transformsuz boyutu ve döndürülmüş/ölçeklenmiş
 * sınır kutusu elle verilir.
 */
function stubLayerLayout(layer, { width, height, boundingRect }) {
  Object.defineProperty(layer, "offsetWidth", { configurable: true, value: width });
  Object.defineProperty(layer, "offsetHeight", { configurable: true, value: height });
  layer.getBoundingClientRect = () => ({
    ...boundingRect,
    right: boundingRect.left + boundingRect.width,
    bottom: boundingRect.top + boundingRect.height,
    x: boundingRect.left,
    y: boundingRect.top,
  });
}

describe("AnnotationLayer tools", () => {
  it("does not draw while a form design tool is active", async () => {
    getPageAnnotations.mockResolvedValue([]);
    const { container } = render(AnnotationLayer, {
      props: { sessionId: "s", contentGeneration: 1, pageIndex: 0, pageWidth: 300, pageHeight: 500, activeTool: "form-text" },
    });
    const layer = container.querySelector(".annotation-layer");
    expect(layer.classList.contains("drawing")).toBe(false);
    stubLayerLayout(layer, { width: 300, height: 500, boundingRect: { left: 0, top: 0, width: 300, height: 500 } });
    await fireEvent.pointerDown(layer, { button: 0, clientX: 30, clientY: 50 });
    await fireEvent.pointerMove(layer, { clientX: 200, clientY: 300 });
    await fireEvent.pointerUp(layer, { clientX: 200, clientY: 300 });
    expect(addAnnotation).not.toHaveBeenCalled();
  });
});

describe("AnnotationLayer coordinates", () => {
  it("sends the drawn rect in normalized display space under viewer rotation and zoom", async () => {
    getPageAnnotations.mockResolvedValue([]);
    addAnnotation.mockResolvedValue({ contentGeneration: 6 });
    const { container } = render(AnnotationLayer, {
      props: {
        sessionId: "annotation-session",
        contentGeneration: 5,
        pageIndex: 0,
        rotation: 90,
        pageWidth: 300,
        pageHeight: 500,
        activeTool: "highlight",
      },
    });
    const layer = container.querySelector(".annotation-layer");
    // 300×500 katman, 90° döndürülmüş ve toplam 2× ölçeklenmiş: sınır kutusu 1000×600.
    stubLayerLayout(layer, {
      width: 300,
      height: 500,
      boundingRect: { left: 100, top: 50, width: 1000, height: 600 },
    });

    // 90°'de yerel (u, v) ekranda (left + 1000·(1 − v), top + 600·u) noktasındadır.
    await fireEvent.pointerDown(layer, { button: 0, clientX: 100 + 1000 * 0.8, clientY: 50 + 600 * 0.1 });
    await fireEvent.pointerMove(layer, { clientX: 100 + 1000 * 0.4, clientY: 50 + 600 * 0.5 });

    const preview = container.querySelector(".annot-preview");
    expect(parseFloat(preview.style.left)).toBeCloseTo(10, 9);
    expect(parseFloat(preview.style.top)).toBeCloseTo(20, 9);
    expect(parseFloat(preview.style.width)).toBeCloseTo(40, 9);
    expect(parseFloat(preview.style.height)).toBeCloseTo(40, 9);

    await fireEvent.pointerUp(layer, { clientX: 100 + 1000 * 0.4, clientY: 50 + 600 * 0.5 });

    await waitFor(() => expect(addAnnotation).toHaveBeenCalledTimes(1));
    const [session, generation, page, type, normalizedRect] = addAnnotation.mock.calls[0];
    expect([session, generation, page, type]).toEqual(["annotation-session", 5, 0, "Highlight"]);
    expect(normalizedRect.x).toBeCloseTo(0.1, 9);
    expect(normalizedRect.y).toBeCloseTo(0.2, 9);
    expect(normalizedRect.width).toBeCloseTo(0.4, 9);
    expect(normalizedRect.height).toBeCloseTo(0.4, 9);
  });

  it("positions stored annotations from their normalized display rect", async () => {
    getPageAnnotations.mockResolvedValue([
      {
        id: "0",
        annot_type: "Highlight",
        color: "#ffcc00",
        normalized_rect: { x: 0.25, y: 0.5, width: 0.125, height: 0.0625 },
        contents: null,
      },
    ]);
    const view = render(AnnotationLayer, {
      props: { sessionId: "annotation-session", contentGeneration: 1, pageIndex: 0 },
    });

    const item = await view.findByTitle("Highlight");
    expect(item.style.left).toBe("25%");
    expect(item.style.top).toBe("50%");
    expect(item.style.width).toBe("12.5%");
    expect(item.style.height).toBe("6.25%");
  });
});
