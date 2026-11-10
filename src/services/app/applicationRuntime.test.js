import { describe, expect, it, vi } from "vitest";
import { createApplicationRuntime } from "./applicationRuntime.js";

describe("applicationRuntime", () => {
  it("registers synchronous cleanup and preserves coordinator-before-deactivation order", async () => {
    const calls = [];
    const lifecycle = { start: vi.fn(async () => calls.push("app-start")), stop: vi.fn(() => calls.push("app-stop")), saveViewerSettings: vi.fn() };
    const windowLifecycle = { start: vi.fn(async () => calls.push("window-start")), destroy: vi.fn(() => calls.push("window-stop")) };
    const keyboard = { handleKeyDown: vi.fn() };
    const documentRuntime = {
      activeDisplayPath: "",
      openFile: vi.fn(), selectFile: vi.fn(), save: vi.fn(), saveAs: vi.fn(),
      lifecycleController: { destroy: vi.fn(() => calls.push("document-stop")) },
      coordinator: { stop: vi.fn(async () => { calls.push("coordinator-stop"); return { status: "stopped" }; }) },
      transitionApi: {}, chooseDocumentTransition: vi.fn(),
    };
    const viewerRuntime = {
      navigation: { jumpToPage: vi.fn(), adjustZoom: vi.fn(), goToNextPage: vi.fn(), goToPrevPage: vi.fn() },
      start: vi.fn(() => calls.push("viewer-start")),
      stop: vi.fn(() => calls.push("viewer-stop")),
      deactivatePdf: vi.fn(async () => calls.push("viewer-deactivate")),
    };
    const featureRuntime = { runUiAction: vi.fn(), start: vi.fn(() => calls.push("feature-start")), destroy: vi.fn(() => calls.push("feature-stop")) };
    const asyncPrompt = { request: vi.fn(), cancel: vi.fn(() => calls.push("prompt-cancel")) };
    const presentationController = { showToast: vi.fn(), setError: vi.fn(), destroy: vi.fn(() => calls.push("presentation-stop")) };
    const mounted = [];
    const pluginHookBindings = { configure: vi.fn(), prime: vi.fn(async () => {}) };

    const runtime = createApplicationRuntime({
      viewer: { pageOrder: [], activeImageSourcePath: "" },
      ui: { togglePanel: vi.fn(), isModalOpen: vi.fn(), closeModal: vi.fn(), openModal: vi.fn(), setPanel: vi.fn() },
      documentSession: { active: null, activeSessionId: null, contentGeneration: 5, closeSession: vi.fn() },
      asyncPrompt, presentationController, documentRuntime, viewerRuntime, featureRuntime,
      callbacks: {}, getViewerArea: () => ({}), setMounted: (value) => mounted.push(value),
      windowApi: {},
      createLifecycleController: () => lifecycle,
      createWindowController: () => windowLifecycle,
      createKeyboard: () => keyboard,
      pluginHookBindings,
    });

    // Plugin hooks get the live document identity and the app toast; commands load on mount.
    expect(pluginHookBindings.configure).toHaveBeenCalledOnce();
    const binding = pluginHookBindings.configure.mock.calls[0][0];
    expect(binding.getDocumentContext()).toEqual({ sessionId: null, contentGeneration: 5 });
    expect(binding.showToast).toBe(presentationController.showToast);
    expect(pluginHookBindings.prime).not.toHaveBeenCalled();

    const cleanup = runtime.mount();
    expect(pluginHookBindings.prime).toHaveBeenCalledOnce();
    expect(typeof cleanup).toBe("function");
    cleanup();
    await vi.waitFor(() => expect(viewerRuntime.deactivatePdf).toHaveBeenCalledOnce());

    expect(mounted).toEqual([true, false]);
    expect(asyncPrompt.cancel).toHaveBeenCalledOnce();
    expect(calls.indexOf("coordinator-stop")).toBeLessThan(calls.indexOf("viewer-deactivate"));
    expect(calls).toContain("document-stop");
    expect(calls).toContain("presentation-stop");
  });
});
