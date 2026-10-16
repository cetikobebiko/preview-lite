import { afterEach, describe, expect, it, vi } from "vitest";
import { createViewerController, imageCanvasStyle, MAX_WHEEL_ZOOM_STEP, MIN_GESTURE_VISUAL_SCALE, wheelZoomFactor } from "./viewerController.js";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function createRenderSession() {
  return { snapshot: () => ({ contentGeneration: 1 }), activate: vi.fn(), clear: vi.fn() };
}

describe("viewer controller", () => {
  it("serializes PDF activation and deactivation through one queue", async () => {
    const calls = [];
    const renderSession = { snapshot: () => ({ contentGeneration: 1 }), activate: (...args) => calls.push(["activate", ...args]), clear: (...args) => calls.push(["clear", ...args]) };
    const controller = createViewerController({ setActiveRenderDocument: async (payload) => calls.push(["worker", payload]), renderSession });
    await Promise.all([controller.activatePdf({ id: "A", contentGeneration: 1 }), controller.deactivatePdf(1)]);
    expect(calls).toEqual([
      ["worker", { sessionId: "A", contentGeneration: 1 }], ["activate", "A", 1],
      ["worker", { sessionId: null, contentGeneration: 1 }], ["clear", 1],
    ]);
  });

  it("owns viewport wheel and resize listener cleanup", () => {
    const listeners = new Map();
    const element = {
      addEventListener: (name, handler) => listeners.set(name, handler),
      removeEventListener: (name) => listeners.delete(name),
    };
    const viewer = { scale: 1, visualScale: 1, isZooming: false };
    const controller = createViewerController({
      setActiveRenderDocument: async () => {},
      renderSession: { snapshot: () => ({ contentGeneration: 1 }), activate: () => {}, clear: () => {} },
    });

    controller.attachViewport({ element, viewer, hasDocument: () => true, isMounted: () => true, onRender: () => {} });
    expect(listeners.has("wheel")).toBe(true);
    controller.detachViewport();
    expect(listeners.has("wheel")).toBe(false);
  });

  it("brackets a ctrl+wheel gesture with one zoom-start and one zoom-commit hook", () => {
    vi.useFakeTimers();
    const listeners = new Map();
    const element = {
      addEventListener: (name, handler) => listeners.set(name, handler),
      removeEventListener: (name) => listeners.delete(name),
    };
    const viewer = { scale: 1, visualScale: 1, isZooming: false };
    const order = [];
    const controller = createViewerController({
      setActiveRenderDocument: async () => {},
      renderSession: { snapshot: () => ({ contentGeneration: 1 }), activate: () => {}, clear: () => {} },
    });
    controller.attachViewport({
      element, viewer, hasDocument: () => true, isMounted: () => true,
      onRender: () => order.push(["render", viewer.scale]),
      onZoomStart: () => order.push(["start", viewer.visualScale]),
      onZoomCommit: () => order.push(["commit", viewer.scale]),
    });
    const wheel = { ctrlKey: true, deltaY: -5, preventDefault: vi.fn() };
    listeners.get("wheel")(wheel);
    listeners.get("wheel")(wheel);
    vi.advanceTimersByTime(300);
    // e^(2 × 0.05) = 1.105 → 1.11.
    expect(order).toEqual([["start", 1], ["render", 1.11], ["commit", 1.11]]);
    controller.stop();
    vi.useRealTimers();
  });

  it("turns ctrl+wheel deltaY into a proportional, capped zoom factor", () => {
    expect(wheelZoomFactor({ deltaY: 0 })).toBe(1);
    expect(wheelZoomFactor({ deltaY: -0 })).toBe(1);
    expect(wheelZoomFactor({ deltaY: Number.NaN })).toBe(1);
    expect(wheelZoomFactor({})).toBe(1);
    expect(wheelZoomFactor({ deltaY: -1 })).toBeGreaterThan(1);
    expect(wheelZoomFactor({ deltaY: 1 })).toBeLessThan(1);
    expect(wheelZoomFactor({ deltaY: -8 })).toBeGreaterThan(wheelZoomFactor({ deltaY: -2 }));
    expect(wheelZoomFactor({ deltaY: -3 }) * wheelZoomFactor({ deltaY: 3 })).toBeCloseTo(1, 12);
    expect(wheelZoomFactor({ deltaY: -100 })).toBeCloseTo(Math.exp(MAX_WHEEL_ZOOM_STEP), 12);
    expect(wheelZoomFactor({ deltaY: 100 })).toBeCloseTo(Math.exp(-MAX_WHEEL_ZOOM_STEP), 12);
    expect(wheelZoomFactor({ deltaY: -1, deltaMode: 1 })).toBeCloseTo(wheelZoomFactor({ deltaY: -16 }), 12);
    expect(wheelZoomFactor({ deltaY: 1, deltaMode: 2 })).toBeCloseTo(Math.exp(-MAX_WHEEL_ZOOM_STEP), 12);
  });

  it("does not zoom out on the zero-delta events of a trackpad pinch-in", () => {
    vi.useFakeTimers();
    const listeners = new Map();
    const element = { addEventListener: (name, handler) => listeners.set(name, handler), removeEventListener: () => {} };
    const viewer = { scale: 1, visualScale: 1, isZooming: false };
    const controller = createViewerController({ setActiveRenderDocument: async () => {}, renderSession: createRenderSession() });
    const onZoomStart = vi.fn();
    controller.attachViewport({ element, viewer, hasDocument: () => true, isMounted: () => true, onRender: () => {}, onZoomStart });
    const zero = { ctrlKey: true, deltaY: 0, preventDefault: vi.fn() };
    listeners.get("wheel")(zero);
    expect(zero.preventDefault).toHaveBeenCalled();
    expect(viewer).toMatchObject({ scale: 1, visualScale: 1, isZooming: false });
    expect(onZoomStart).not.toHaveBeenCalled();
    // A pinch-in as WKWebView reported it: runs of zero deltas around small negative ones.
    const visualScales = [];
    for (const deltaY of [0, 0, -1, -3, 0, -5, -6, -8, -8, 0, -6, -3, 0, -1, 0]) {
      listeners.get("wheel")({ ctrlKey: true, deltaY, preventDefault: () => {} });
      visualScales.push(viewer.visualScale);
    }
    visualScales.forEach((value, index) => expect(value).toBeGreaterThanOrEqual(visualScales[index - 1] ?? 1));
    expect(onZoomStart).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(300);
    // 41 px of pinch-in: e^0.41 ≈ 1.51.
    expect(viewer).toMatchObject({ scale: 1.51, visualScale: 1, isZooming: false });
    controller.stop();
  });

  it("lets one pinch gesture at most halve the view before it commits", () => {
    vi.useFakeTimers();
    const listeners = new Map();
    const element = { addEventListener: (name, handler) => listeners.set(name, handler), removeEventListener: () => {} };
    const viewer = { scale: 4, visualScale: 1, isZooming: false };
    const controller = createViewerController({ setActiveRenderDocument: async () => {}, renderSession: createRenderSession() });
    const visualScales = [];
    controller.attachViewport({ element, viewer, hasDocument: () => true, isMounted: () => true, onRender: () => {} });
    for (let step = 0; step < 60; step += 1) {
      listeners.get("wheel")({ ctrlKey: true, deltaY: 10, preventDefault: () => {} });
      visualScales.push(viewer.visualScale);
    }
    expect(Math.min(...visualScales)).toBe(MIN_GESTURE_VISUAL_SCALE);
    vi.advanceTimersByTime(300);
    expect(viewer).toMatchObject({ scale: 2, visualScale: 1, isZooming: false });
    controller.stop();
  });

  it("commits a large zoom-out at once instead of showing it as a transform", () => {
    vi.useFakeTimers();
    const viewer = { scale: 4, visualScale: 1, isZooming: false };
    const controller = createViewerController({ setActiveRenderDocument: async () => {}, renderSession: createRenderSession() });
    const render = vi.fn(() => expect(viewer.visualScale).toBe(1));
    controller.commitZoom({ viewer, nextScale: 3.8, onRender: render });
    expect(viewer.isZooming).toBe(true);
    controller.commitZoom({ viewer, nextScale: 1, onRender: render });
    expect(viewer).toMatchObject({ scale: 1, visualScale: 1, isZooming: false });
    expect(render).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1000);
    expect(render).toHaveBeenCalledTimes(1);
    expect(viewer.scale).toBe(1);
    controller.stop();
  });

  it("reports the painted image bitmap size", () => {
    const onImagePainted = vi.fn();
    const paint = vi.fn((canvas, rendered) => { canvas.width = rendered.width; canvas.height = rendered.height; });
    const controller = createViewerController({ setActiveRenderDocument: vi.fn(), renderSession: createRenderSession(), paint, onImagePainted });
    controller.paintImage({}, { width: 640, height: 480, data: new Uint8Array(640 * 480 * 4) });
    expect(onImagePainted).toHaveBeenCalledWith({ width: 640, height: 480 });
    controller.paintImage(null, { width: 1, height: 1 });
    expect(onImagePainted).toHaveBeenCalledTimes(1);
  });

  it("sizes the image canvas from its bitmap and the committed scale", () => {
    expect(imageCanvasStyle(null, 2)).toBe("");
    expect(imageCanvasStyle({ width: 0, height: 10 }, 2)).toBe("");
    expect(imageCanvasStyle({ width: 640, height: 480 }, 1)).toBe("width: 640px; height: 480px;");
    expect(imageCanvasStyle({ width: 640, height: 480 }, 1.5)).toBe("width: 960px; height: 720px;");
    // The style follows the same bounds as the committed scale.
    expect(imageCanvasStyle({ width: 100, height: 50 }, 10)).toBe("width: 400px; height: 200px;");
    expect(imageCanvasStyle({ width: 100, height: 50 }, Number.NaN)).toBe("width: 100px; height: 50px;");
  });

  it("drops a late image result after the active image identity changes", async () => {
    let runFrame;
    let resolveImage;
    const context = { mounted: true, imagePath: "first.png" };
    const paint = vi.fn();
    const onRendered = vi.fn();
    vi.stubGlobal("requestAnimationFrame", vi.fn((callback) => { runFrame = callback; return 1; }));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const controller = createViewerController({
      setActiveRenderDocument: vi.fn(),
      renderSession: createRenderSession(),
      getRenderContext: () => context,
      readImage: vi.fn(() => new Promise((resolve) => { resolveImage = resolve; })),
      paint,
      onRendered,
    });

    controller.renderImage({ canvas: {}, rotation: 0, flipH: false, flipV: false });
    runFrame();
    context.imagePath = "second.png";
    resolveImage({ width: 1, height: 1, data: new Uint8Array(4) });
    await Promise.resolve();
    await Promise.resolve();

    expect(paint).not.toHaveBeenCalled();
    expect(onRendered).not.toHaveBeenCalled();
  });

  it("clears pending zoom and resize timers on stop", () => {
    vi.useFakeTimers();
    let resizeCallback;
    const disconnect = vi.fn();
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback) { resizeCallback = callback; }
      observe() {}
      disconnect() { disconnect(); }
    });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const element = { addEventListener: vi.fn(), removeEventListener: vi.fn() };
    const viewer = { scale: 1, visualScale: 1, isZooming: false };
    const render = vi.fn();
    const controller = createViewerController({
      setActiveRenderDocument: vi.fn(),
      renderSession: createRenderSession(),
    });

    controller.attachViewport({ element, viewer, hasDocument: () => true, isMounted: () => true, onRender: render });
    controller.commitZoom({ viewer, nextScale: 2, onRender: render, delayMs: 50 });
    resizeCallback();
    controller.stop();
    vi.runAllTimers();

    expect(render).not.toHaveBeenCalled();
    expect(disconnect).toHaveBeenCalledOnce();
  });

  it("cancels a scheduled animation frame and tolerates repeated stop", () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 17));
    const cancelAnimationFrame = vi.fn();
    vi.stubGlobal("cancelAnimationFrame", cancelAnimationFrame);
    const controller = createViewerController({
      setActiveRenderDocument: vi.fn(),
      renderSession: createRenderSession(),
      getRenderContext: () => ({ mounted: true, imagePath: "image.png" }),
    });
    controller.renderImage({ canvas: {}, rotation: 0, flipH: false, flipV: false });
    expect(() => { controller.stop(); controller.stop(); }).not.toThrow();
    expect(cancelAnimationFrame).toHaveBeenCalledWith(17);
  });
});
