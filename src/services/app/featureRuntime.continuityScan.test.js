import { describe, expect, it, vi } from "vitest";

// The runtime evaluates features for the platform it runs on; pin macOS production here.
vi.mock("../../featureRegistry.js", async (importOriginal) => ({
  ...(await importOriginal()),
  getRuntimeFeatureContext: () => ({ build: "production", platform: "macos" }),
}));

const { createFeatureRuntime } = await import("./featureRuntime.js");

function setup() {
  const job = { jobId: "job-1", promise: Promise.resolve({ snapshot: { id: "session-1", contentGeneration: 2 }, kind: "pdf" }), cancel: vi.fn() };
  const continuityScanActions = {
    confirmSignedEdit: vi.fn(async () => {}),
    resolvePageFit: vi.fn(async () => ({ width: 612, height: 792 })),
    start: vi.fn(() => job),
    getPageCount: vi.fn(async () => 5),
  };
  const applyMutationSnapshot = vi.fn(async () => {});
  const onToast = vi.fn();
  const ui = { openModal: vi.fn(), closeModal: vi.fn(), togglePanel: vi.fn(), setPanel: vi.fn(), isModalOpen: vi.fn(() => false), panels: {} };
  const controllerStub = () => new Proxy({}, { get: () => vi.fn() });
  const runtime = createFeatureRuntime({
    // Display position 1 is physical page 2 in pageOrder [0, 2, 1].
    viewer: { activeImageSourcePath: "", currentPageIndex: 1, pageOrder: [0, 2, 1], rotation: 0, flipH: false, flipV: false },
    annotation: { annotationsMap: {} },
    documentSession: { activeSessionId: "session-1", contentGeneration: 1 },
    searchState: { clearHighlightScroll: vi.fn(), reset: vi.fn() },
    sensitiveScanState: { logicalHighlightsByPage: {}, reset: vi.fn() },
    secureRedactionState: { pendingRegions: [], reset: vi.fn() },
    sanitizationState: { reset: vi.fn() },
    ui,
    asyncPrompt: { request: vi.fn() },
    viewerRuntime: { navigation: { toggleBookMode: vi.fn(), resetZoom: vi.fn() }, render: vi.fn() },
    documentRuntime: { isPdf: true, isLoadedPdfDocument: true, activeDisplayPath: "/test.pdf", fileName: "document.pdf", openFile: vi.fn(), applyMutationSnapshot },
    callbacks: { isMounted: () => true, saveSettings: vi.fn() },
    tick: async () => {},
    onToast,
    onError: vi.fn(),
    continuityScanActions,
    createExportController: vi.fn(() => ({ exportCurrentPageImage: vi.fn() })),
    createSearchController: controllerStub,
    createSensitiveDataScanController: controllerStub,
    createSecureRedactionController: controllerStub,
    createSanitizationController: controllerStub,
    listenToEvent: vi.fn(async () => vi.fn()),
  });
  return { runtime, continuityScanActions, applyMutationSnapshot, onToast, ui, job };
}

describe("camera-scan command", () => {
  it("scans into the document after the current page's physical index", async () => {
    const { runtime, continuityScanActions, applyMutationSnapshot, onToast, ui } = setup();

    await expect(runtime.commandRouter.dispatch("camera-scan")).resolves.toEqual({ status: "handled" });

    expect(continuityScanActions.confirmSignedEdit).toHaveBeenCalledWith("session-1", 1, "iPhone'dan sayfa ekleme");
    expect(continuityScanActions.resolvePageFit).toHaveBeenCalledWith("session-1", 1, 2);
    expect(continuityScanActions.start).toHaveBeenCalledWith(
      "session-1", 1, 3, { width: 612, height: 792 }, { build: "production", platform: "macos" },
    );
    expect(ui.openModal).toHaveBeenCalledWith("continuity_scan", expect.objectContaining({ oncancel: expect.any(Function) }));
    const [, { mapPageIndex }] = applyMutationSnapshot.mock.calls[0];
    expect([0, 1, 2].map(mapPageIndex)).toEqual([0, 1, 2]);
    expect(onToast).toHaveBeenCalledWith(expect.stringContaining("2 sayfa"));
    expect(ui.closeModal).toHaveBeenCalledWith("continuity_scan");
  });

  it("exposes the cancellation hooks used on destroy and on document changes", () => {
    const { runtime } = setup();
    expect(typeof runtime.reconcileContinuityScanContext).toBe("function");
    expect(runtime.continuityScan.isActive).toBe(false);
    expect(() => runtime.destroy()).not.toThrow();
  });
});
