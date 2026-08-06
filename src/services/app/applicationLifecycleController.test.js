import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApplicationLifecycleController } from "./applicationLifecycleController.js";

let settings = new Map();

beforeEach(() => {
  settings = new Map();
  vi.stubGlobal("localStorage", {
    getItem: (key) => settings.get(key) ?? null,
    setItem: (key, value) => settings.set(key, value),
  });
});

afterEach(() => vi.unstubAllGlobals());

describe("applicationLifecycleController", () => {
  function createBrowserWindow() {
    return {
      innerWidth: 1200,
      innerHeight: 800,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    };
  }

  function createController(overrides = {}) {
    return createApplicationLifecycleController({
      viewer: { isSmartDarkMode: false, scale: 1, readingTimeSeconds: 0 },
      ui: {},
      onOpenFile: vi.fn(),
      onToast: vi.fn(),
      onError: vi.fn(),
      isDocumentOpen: () => false,
      initializeConfig: async () => {},
      getRecentFiles: () => [],
      listenToEvent: async () => vi.fn(),
      browserWindow: createBrowserWindow(),
      ...overrides,
    });
  }

  it("round-trips viewer preferences without owning a second viewer state", async () => {
    const viewer = { isSmartDarkMode: true, scale: 1.25 };
    const controller = createController({ viewer });

    controller.saveViewerSettings();

    expect(JSON.parse(settings.get("antigravity_settings"))).toEqual({
      viewer: { isSmartDarkMode: true, scale: 1.25 },
    });

    const restoredViewer = { isSmartDarkMode: false, scale: 1, readingTimeSeconds: 0 };
    const restoredController = createController({ viewer: restoredViewer });
    await restoredController.start();
    expect(restoredViewer).toMatchObject({ isSmartDarkMode: true, scale: 1.25 });
    restoredController.stop();
  });

  it("does not install runtime resources after stop wins an initialization race", async () => {
    let finishInitialization;
    const initializeConfig = vi.fn(() => new Promise((resolve) => { finishInitialization = resolve; }));
    const browserWindow = createBrowserWindow();
    const getRecentFiles = vi.fn(() => ["recent.pdf"]);
    const listenToEvent = vi.fn(async () => vi.fn());
    const onOpenFile = vi.fn();
    const controller = createController({ initializeConfig, browserWindow, getRecentFiles, listenToEvent, onOpenFile });

    const starting = controller.start();
    await Promise.resolve();
    controller.stop();
    finishInitialization();

    await expect(starting).resolves.toEqual({ status: "stopped" });
    expect(getRecentFiles).not.toHaveBeenCalled();
    expect(onOpenFile).not.toHaveBeenCalled();
    expect(listenToEvent).not.toHaveBeenCalled();
    expect(browserWindow.addEventListener).not.toHaveBeenCalled();
  });

  it("disposes a native listener that resolves after stop", async () => {
    let finishListening;
    const dispose = vi.fn();
    const listenToEvent = vi.fn(() => new Promise((resolve) => { finishListening = () => resolve(dispose); }));
    const controller = createController({ listenToEvent });

    await expect(controller.start()).resolves.toEqual({ status: "started" });
    controller.stop();
    finishListening();
    await Promise.resolve();

    expect(dispose).toHaveBeenCalledOnce();
  });

  it("is idempotent while running and can start cleanly after stop", async () => {
    const initializeConfig = vi.fn(async () => {});
    const browserWindow = createBrowserWindow();
    const disposeFirst = vi.fn();
    const disposeSecond = vi.fn();
    const listenToEvent = vi.fn()
      .mockResolvedValueOnce(disposeFirst)
      .mockResolvedValueOnce(disposeSecond);
    const controller = createController({ initializeConfig, browserWindow, listenToEvent });

    await Promise.all([controller.start(), controller.start()]);
    expect(initializeConfig).toHaveBeenCalledOnce();
    expect(browserWindow.addEventListener).toHaveBeenCalledTimes(3);
    controller.stop();
    expect(disposeFirst).toHaveBeenCalledOnce();

    await controller.start();
    expect(initializeConfig).toHaveBeenCalledTimes(2);
    expect(browserWindow.addEventListener).toHaveBeenCalledTimes(6);
    controller.stop();
    expect(disposeSecond).toHaveBeenCalledOnce();
  });
});
