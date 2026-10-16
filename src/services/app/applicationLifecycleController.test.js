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
      listenToDragDrop: async () => vi.fn(),
      ...overrides,
    });
  }

  /** Captures every drag-drop handler the controller registers, newest last. */
  function createDragDropSource() {
    const handlers = [];
    const disposers = [];
    const listenToDragDrop = vi.fn(async (handler) => {
      handlers.push(handler);
      const dispose = vi.fn();
      disposers.push(dispose);
      return dispose;
    });
    const emit = (payload, index = handlers.length - 1) => handlers[index]({ event: "tauri://drag", id: 1, payload });
    return { listenToDragDrop, handlers, disposers, emit };
  }

  async function startWithDragDrop(overrides = {}) {
    const source = createDragDropSource();
    const ui = { isDraggingFileOver: false };
    const onOpenFile = vi.fn();
    const onToast = vi.fn();
    const onError = vi.fn();
    const controller = createController({ ui, onOpenFile, onToast, onError, listenToDragDrop: source.listenToDragDrop, ...overrides });
    await controller.start();
    await Promise.resolve();
    return { controller, source, ui, onOpenFile, onToast, onError };
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

  it.each([
    ["0", 0, 0.25],
    ["-1", -1, 0.25],
    ["5", 5, 4],
    ['"abc"', "abc", 1],
    ["null", null, 1],
    ["1.2000000000000002", 1.2000000000000002, 1.2],
  ])("restores a saved scale of %s as a safe scale", async (_label, savedScale, expected) => {
    settings.set("antigravity_settings", JSON.stringify({ viewer: { isSmartDarkMode: false, scale: savedScale } }));
    const viewer = { isSmartDarkMode: false, scale: 3, readingTimeSeconds: 0 };
    const controller = createController({ viewer });
    await controller.start();
    expect(viewer.scale).toBe(expected);
    controller.stop();
  });

  it("restores a non-finite scale as 1", async () => {
    // NaN is persisted as null by JSON.stringify; 1e999 parses back as Infinity.
    const saving = createController({ viewer: { isSmartDarkMode: false, scale: Number.NaN } });
    saving.saveViewerSettings();
    const nanViewer = { isSmartDarkMode: false, scale: 3, readingTimeSeconds: 0 };
    const nanController = createController({ viewer: nanViewer });
    await nanController.start();
    expect(nanViewer.scale).toBe(1);
    nanController.stop();

    settings.set("antigravity_settings", '{"viewer":{"scale":1e999}}');
    const infiniteViewer = { isSmartDarkMode: false, scale: 3, readingTimeSeconds: 0 };
    const infiniteController = createController({ viewer: infiniteViewer });
    await infiniteController.start();
    expect(infiniteViewer.scale).toBe(1);
    infiniteController.stop();
  });

  it("does not install runtime resources after stop wins an initialization race", async () => {
    let finishInitialization;
    const initializeConfig = vi.fn(() => new Promise((resolve) => { finishInitialization = resolve; }));
    const getRecentFiles = vi.fn(() => ["recent.pdf"]);
    const listenToDragDrop = vi.fn(async () => vi.fn());
    const onOpenFile = vi.fn();
    const controller = createController({ initializeConfig, getRecentFiles, listenToDragDrop, onOpenFile });

    const starting = controller.start();
    await Promise.resolve();
    controller.stop();
    finishInitialization();

    await expect(starting).resolves.toEqual({ status: "stopped" });
    expect(getRecentFiles).not.toHaveBeenCalled();
    expect(onOpenFile).not.toHaveBeenCalled();
    expect(listenToDragDrop).not.toHaveBeenCalled();
  });

  it("disposes a native listener that resolves after stop", async () => {
    let finishListening;
    const dispose = vi.fn();
    const listenToDragDrop = vi.fn(() => new Promise((resolve) => { finishListening = () => resolve(dispose); }));
    const controller = createController({ listenToDragDrop });

    await expect(controller.start()).resolves.toEqual({ status: "started" });
    controller.stop();
    finishListening();
    await Promise.resolve();

    expect(dispose).toHaveBeenCalledOnce();
  });

  it("is idempotent while running and can start cleanly after stop", async () => {
    const initializeConfig = vi.fn(async () => {});
    const disposeFirst = vi.fn();
    const disposeSecond = vi.fn();
    const listenToDragDrop = vi.fn()
      .mockResolvedValueOnce(disposeFirst)
      .mockResolvedValueOnce(disposeSecond);
    const controller = createController({ initializeConfig, listenToDragDrop });

    await Promise.all([controller.start(), controller.start()]);
    await Promise.resolve();
    expect(initializeConfig).toHaveBeenCalledOnce();
    expect(listenToDragDrop).toHaveBeenCalledOnce();
    controller.stop();
    expect(disposeFirst).toHaveBeenCalledOnce();

    await controller.start();
    await Promise.resolve();
    expect(initializeConfig).toHaveBeenCalledTimes(2);
    expect(listenToDragDrop).toHaveBeenCalledTimes(2);
    controller.stop();
    expect(disposeSecond).toHaveBeenCalledOnce();
  });
  describe("native drag-drop", () => {
    it("shows the drop overlay only while openable files hover the webview", async () => {
      const { controller, source, ui } = await startWithDragDrop();

      source.emit({ type: "enter", paths: ["/docs/report.pdf"], position: { x: 10, y: 10 } });
      expect(ui.isDraggingFileOver).toBe(true);
      source.emit({ type: "over", position: { x: 20, y: 20 } });
      expect(ui.isDraggingFileOver).toBe(true);
      source.emit({ type: "leave" });
      expect(ui.isDraggingFileOver).toBe(false);
      controller.stop();
    });

    it.each([
      ["an in-page drag without paths", []],
      ["unsupported files", ["/docs/notes.txt", "/docs/archive.zip"]],
      ["a missing path list", undefined],
    ])("does not show the drop overlay for %s", async (_label, paths) => {
      const { controller, source, ui } = await startWithDragDrop();

      source.emit({ type: "enter", paths, position: { x: 0, y: 0 } });

      expect(ui.isDraggingFileOver).toBe(false);
      controller.stop();
    });

    it("opens a single dropped PDF by its absolute native path", async () => {
      const { controller, source, ui, onOpenFile, onToast } = await startWithDragDrop();

      source.emit({ type: "enter", paths: ["/docs/Belgeler/rapor.pdf"], position: { x: 0, y: 0 } });
      source.emit({ type: "drop", paths: ["/docs/Belgeler/rapor.pdf"], position: { x: 5, y: 5 } });

      expect(ui.isDraggingFileOver).toBe(false);
      expect(onOpenFile).toHaveBeenCalledExactlyOnceWith("/docs/Belgeler/rapor.pdf");
      expect(onToast).toHaveBeenCalledExactlyOnceWith("Sürüklenen dosya açıldı: rapor.pdf");
      controller.stop();
    });

    it("opens the first PDF or image among several dropped files and reports the rest as ignored", async () => {
      const { controller, source, onOpenFile, onToast } = await startWithDragDrop();

      source.emit({
        type: "drop",
        paths: ["/tmp/notes.txt", "C:\\Scans\\page.PNG", "/tmp/second.pdf"],
        position: { x: 0, y: 0 },
      });

      expect(onOpenFile).toHaveBeenCalledExactlyOnceWith("C:\\Scans\\page.PNG");
      expect(onToast).toHaveBeenCalledExactlyOnceWith("Sürüklenen dosya açıldı: page.PNG. Diğer 2 dosya yok sayıldı.");
      controller.stop();
    });

    it("reports a drop that contains no PDF or image without opening anything", async () => {
      const { controller, source, ui, onOpenFile, onToast } = await startWithDragDrop();
      ui.isDraggingFileOver = true;

      source.emit({ type: "drop", paths: ["/tmp/notes.txt", "/tmp/folder"], position: { x: 0, y: 0 } });

      expect(ui.isDraggingFileOver).toBe(false);
      expect(onOpenFile).not.toHaveBeenCalled();
      expect(onToast).toHaveBeenCalledExactlyOnceWith("Desteklenmeyen dosya türü. PDF veya görsel bırakın.");
      controller.stop();
    });

    it("ignores a drop without paths", async () => {
      const { controller, source, onOpenFile, onToast } = await startWithDragDrop();

      source.emit({ type: "drop", paths: [], position: { x: 0, y: 0 } });

      expect(onOpenFile).not.toHaveBeenCalled();
      expect(onToast).not.toHaveBeenCalled();
      controller.stop();
    });

    it("disposes the listener on stop, clears a stuck overlay and ignores late events", async () => {
      const { controller, source, ui, onOpenFile } = await startWithDragDrop();
      source.emit({ type: "enter", paths: ["/tmp/a.pdf"], position: { x: 0, y: 0 } });

      controller.stop();

      expect(source.disposers[0]).toHaveBeenCalledOnce();
      expect(ui.isDraggingFileOver).toBe(false);
      source.emit({ type: "enter", paths: ["/tmp/a.pdf"], position: { x: 0, y: 0 } });
      source.emit({ type: "drop", paths: ["/tmp/a.pdf"], position: { x: 0, y: 0 } });
      expect(ui.isDraggingFileOver).toBe(false);
      expect(onOpenFile).not.toHaveBeenCalled();
    });

    it("routes events only through the listener of the current start epoch", async () => {
      const { controller, source, onOpenFile } = await startWithDragDrop();
      controller.stop();
      await controller.start();
      await Promise.resolve();

      source.emit({ type: "drop", paths: ["/tmp/old.pdf"], position: { x: 0, y: 0 } }, 0);
      source.emit({ type: "drop", paths: ["/tmp/new.pdf"], position: { x: 0, y: 0 } }, 1);

      expect(onOpenFile).toHaveBeenCalledExactlyOnceWith("/tmp/new.pdf");
      controller.stop();
      expect(source.disposers[1]).toHaveBeenCalledOnce();
    });

    it.each([
      ["throws synchronously", () => { throw new Error("no tauri runtime"); }],
      ["rejects", async () => { throw new Error("listen denied"); }],
    ])("reports a drag-drop listener that %s and still starts", async (_label, listenToDragDrop) => {
      const onError = vi.fn();
      const controller = createController({ listenToDragDrop, onError });

      await expect(controller.start()).resolves.toEqual({ status: "started" });
      await Promise.resolve();
      await Promise.resolve();

      expect(onError).toHaveBeenCalledExactlyOnceWith("Dosya bırakma dinleyicisi kurulamadı", expect.any(Error));
      controller.stop();
    });
  });
});
