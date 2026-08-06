import { afterEach, describe, expect, it, vi } from "vitest";
import { createViewerController } from "./viewerController.js";

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
