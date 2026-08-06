import { describe, expect, it, vi } from "vitest";
import { createFeatureRuntime } from "./featureRuntime.js";

describe("featureRuntime lifecycle & façade assertions", () => {
  function setupTestRuntime(overrides = {}) {
    const searchControllerMock = {
      start: vi.fn(),
      destroy: vi.fn(),
      invalidate: vi.fn(),
      handleQueryEvent: vi.fn(),
      handleHighlight: vi.fn(),
    };
    const sensitiveScanControllerMock = {
      start: vi.fn(),
      scan: vi.fn(),
      reconcileContext: vi.fn(),
      destroy: vi.fn(),
    };

    const createExportController = vi.fn(() => ({ exportCurrentPageImage: vi.fn() }));
    const createSearchControllerMock = vi.fn(() => searchControllerMock);
    const createSensitiveScanControllerMock = vi.fn(() => sensitiveScanControllerMock);

    const exportActions = {
      exportPage: vi.fn(),
      extractPages: vi.fn(),
      convertImage: vi.fn(),
      burnAnnotations: vi.fn(),
    };

    const runtime = createFeatureRuntime({
      viewer: { activeImageSourcePath: "", currentPageIndex: 0, pageOrder: [], rotation: 0, flipH: false, flipV: false },
      annotation: { annotationsMap: {} },
      documentSession: { activeSessionId: "session-1", contentGeneration: 1 },
      searchState: { clearHighlightScroll: vi.fn(), reset: vi.fn() },
      sensitiveScanState: {
        logicalHighlightsByPage: {
          0: [{ x: 0.1, y: 0.2, width: 0.3, height: 0.4 }],
        },
        reset: vi.fn(),
      },
      ui: { openModal: vi.fn(), closeModal: vi.fn(), togglePanel: vi.fn(), setPanel: vi.fn(), panels: { sensitiveDataScan: true } },
      asyncPrompt: { request: vi.fn() },
      viewerRuntime: { navigation: { toggleBookMode: vi.fn(), resetZoom: vi.fn() }, render: vi.fn() },
      documentRuntime: { isPdf: true, isLoadedPdfDocument: true, activeDisplayPath: "/test.pdf", fileName: "document.pdf", openFile: vi.fn() },
      callbacks: { isMounted: () => true, saveSettings: vi.fn() },
      tick: async () => {},
      onToast: vi.fn(),
      onError: vi.fn(),
      exportActions,
      createExportController,
      createSearchController: createSearchControllerMock,
      createSensitiveDataScanController: createSensitiveScanControllerMock,
      ...overrides,
    });

    return {
      runtime,
      searchControllerMock,
      sensitiveScanControllerMock,
      createExportController,
    };
  }

  it("calls searchController.start and sensitiveScanController.start exactly once when runtime.start() is invoked", () => {
    const { runtime, searchControllerMock, sensitiveScanControllerMock } = setupTestRuntime();

    expect(searchControllerMock.start).not.toHaveBeenCalled();
    expect(sensitiveScanControllerMock.start).not.toHaveBeenCalled();

    runtime.start();

    expect(searchControllerMock.start).toHaveBeenCalledTimes(1);
    expect(sensitiveScanControllerMock.start).toHaveBeenCalledTimes(1);
  });

  it("calls searchController.destroy and sensitiveScanController.destroy exactly once when runtime.destroy() is invoked", () => {
    const { runtime, searchControllerMock, sensitiveScanControllerMock } = setupTestRuntime();

    runtime.destroy();

    expect(searchControllerMock.destroy).toHaveBeenCalledTimes(1);
    expect(sensitiveScanControllerMock.destroy).toHaveBeenCalledTimes(1);
  });

  it("delegates scanSensitiveData to sensitiveScanController.scan with exact pattern arguments", () => {
    const { runtime, sensitiveScanControllerMock } = setupTestRuntime();
    const patterns = [{ label: "TC", regex: "\\d{11}", active: true }];

    runtime.scanSensitiveData(patterns);

    expect(sensitiveScanControllerMock.scan).toHaveBeenCalledTimes(1);
    expect(sensitiveScanControllerMock.scan).toHaveBeenCalledWith(patterns);
  });

  it("delegates reconcileSensitiveDataContext to sensitiveScanController.reconcileContext", () => {
    const { runtime, sensitiveScanControllerMock } = setupTestRuntime();

    runtime.reconcileSensitiveDataContext();

    expect(sensitiveScanControllerMock.reconcileContext).toHaveBeenCalledTimes(1);
  });

  it("passes pageIndex and viewerRotation to highlight façade selector and returns logical highlights", () => {
    const { runtime } = setupTestRuntime();

    const highlightsPage0 = runtime.getSensitiveDataHighlightsForPage(0, 90);
    expect(highlightsPage0).toBeDefined();
    expect(Array.isArray(highlightsPage0)).toBe(true);

    const highlightsPage1 = runtime.getSensitiveDataHighlightsForPage(1, 0);
    expect(highlightsPage1).toEqual([]);
  });

  it("does NOT expose raw sensitiveScanController on the runtime object", () => {
    const { runtime } = setupTestRuntime();

    expect(runtime.sensitiveScanController).toBeUndefined();
    expect(runtime.sensitiveController).toBeUndefined();
    expect(runtime.rawSensitiveScanController).toBeUndefined();

    // Verify allowed façade methods exist
    expect(typeof runtime.scanSensitiveData).toBe("function");
    expect(typeof runtime.reconcileSensitiveDataContext).toBe("function");
    expect(typeof runtime.getSensitiveDataHighlightsForPage).toBe("function");
  });
});
