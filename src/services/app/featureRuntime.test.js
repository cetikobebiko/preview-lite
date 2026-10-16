import { describe, expect, it, vi } from "vitest";
import { createFeatureRuntime } from "./featureRuntime.js";
import { createDocumentScopedStateReconciler } from "./documentScopedState.js";
import { buildAnnotationMarkdown } from "./annotationMarkdown.js";
import { createAnnotationState } from "../../stores/annotationState.svelte.js";

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
    const secureRedactionControllerMock = {
      start: vi.fn(), destroy: vi.fn(), plan: vi.fn(), execute: vi.fn(), cancelPlanning: vi.fn(),
      cancelExecution: vi.fn(), reconcileContext: vi.fn(), addManualRegion: vi.fn(),
      promoteSensitiveScanItem: vi.fn(), removeRegion: vi.fn(), clearRegions: vi.fn(),
      updatePolicy: vi.fn(), setAcknowledgement: vi.fn(), setSelectionMode: vi.fn(), newRedaction: vi.fn(),
    };
    const sanitizationControllerMock = {
      start: vi.fn(), destroy: vi.fn(), inspect: vi.fn(), updatePolicy: vi.fn(), setAcknowledgement: vi.fn(),
      execute: vi.fn(), cancel: vi.fn(), cancelInspection: vi.fn(), cancelExecution: vi.fn(),
      reconcileContext: vi.fn(), reset: vi.fn(),
    };

    const createExportController = vi.fn(() => ({ exportCurrentPageImage: vi.fn() }));
    const createSearchControllerMock = vi.fn(() => searchControllerMock);
    const createSensitiveScanControllerMock = vi.fn(() => sensitiveScanControllerMock);
    const createSecureRedactionControllerMock = vi.fn(() => secureRedactionControllerMock);
    const createSanitizationControllerMock = vi.fn(() => sanitizationControllerMock);

    const exportActions = {
      exportPage: vi.fn(),
      extractPages: vi.fn(),
      convertImage: vi.fn(),
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
      secureRedactionState: { pendingRegions: [], reset: vi.fn() },
      sanitizationState: { reset: vi.fn() },
      ui: { openModal: vi.fn(), closeModal: vi.fn(), togglePanel: vi.fn(), setPanel: vi.fn(), isModalOpen: vi.fn(() => true), panels: { sensitiveDataScan: true } },
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
      createSecureRedactionController: createSecureRedactionControllerMock,
      createSanitizationController: createSanitizationControllerMock,
      ...overrides,
    });

    return {
      runtime,
      searchControllerMock,
      sensitiveScanControllerMock,
      secureRedactionControllerMock,
      sanitizationControllerMock,
      createExportController,
    };
  }

  it("starts search, sensitive scan, and secure redaction controllers", () => {
    const { runtime, searchControllerMock, sensitiveScanControllerMock, secureRedactionControllerMock } = setupTestRuntime();

    expect(searchControllerMock.start).not.toHaveBeenCalled();
    expect(sensitiveScanControllerMock.start).not.toHaveBeenCalled();

    runtime.start();

    expect(searchControllerMock.start).toHaveBeenCalledTimes(1);
    expect(sensitiveScanControllerMock.start).toHaveBeenCalledTimes(1);
    expect(secureRedactionControllerMock.start).toHaveBeenCalledTimes(1);
  });

  it("destroys search, sensitive scan, and secure redaction controllers", () => {
    const { runtime, searchControllerMock, sensitiveScanControllerMock, secureRedactionControllerMock } = setupTestRuntime();

    runtime.destroy();

    expect(searchControllerMock.destroy).toHaveBeenCalledTimes(1);
    expect(sensitiveScanControllerMock.destroy).toHaveBeenCalledTimes(1);
    expect(secureRedactionControllerMock.destroy).toHaveBeenCalledTimes(1);
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

  it("exposes a semantic secure-redaction façade and context reconciliation", () => {
    const { runtime, secureRedactionControllerMock } = setupTestRuntime();
    expect(runtime.secureRedaction).toBeDefined();
    runtime.secureRedaction.reconcileContext();
    expect(secureRedactionControllerMock.reconcileContext).toHaveBeenCalledTimes(1);
    expect(runtime.secureRedactionController).toBeUndefined();
  });

  it("exposes the canonical sanitization façade without a legacy transform action", () => {
    const { runtime, sanitizationControllerMock } = setupTestRuntime();
    expect(runtime.sanitization).toBeDefined();
    runtime.sanitization.reconcileContext();
    runtime.sanitization.cancelExecution();
    expect(sanitizationControllerMock.reconcileContext).toHaveBeenCalledTimes(1);
    expect(sanitizationControllerMock.cancelExecution).toHaveBeenCalledTimes(1);
    expect(runtime.sanitize).toBeUndefined();
    expect(runtime.sanitizationController).toBeUndefined();
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

  it("never hands drawings from a previously open document to notes export", () => {
    const annotation = createAnnotationState();
    const viewer = { activeImageSourcePath: "", currentPageIndex: 0, pageOrder: [0], rotation: 0, flipH: false, flipV: false, documentTextContext: "", bionicText: "" };
    const documentSession = { activeSessionId: null, contentGeneration: 0 };
    let exportContext;
    setupTestRuntime({
      annotation, viewer, documentSession,
      createExportController: vi.fn(({ getContext }) => { exportContext = getContext; return { exportCurrentPageImage: vi.fn() }; }),
    });
    const reconciler = createDocumentScopedStateReconciler({ viewer, annotation });

    viewer.activeImageSourcePath = "/scan.png";
    reconciler.reconcile({ sessionId: null, contentGeneration: 0, imagePath: "/scan.png" });
    annotation.annotationsMap = { 0: [{ page_index: 0, tool_type: "text", text: "görsel notu", color: "#fff" }] };
    expect(buildAnnotationMarkdown({ fileName: "scan.png", annotationsMap: annotation.annotationsMap }).totalNotes).toBe(1);

    viewer.activeImageSourcePath = "";
    documentSession.activeSessionId = "pdf-b";
    documentSession.contentGeneration = 1;
    reconciler.reconcile({ sessionId: "pdf-b", contentGeneration: 1, imagePath: "" });

    expect(exportContext().sessionId).toBe("pdf-b");
    expect(buildAnnotationMarkdown({ fileName: "b.pdf", annotationsMap: annotation.annotationsMap }).totalNotes).toBe(0);
  });

  it("passes the physical page to page-level backend exports while keeping the displayed number", () => {
    let exportContext;
    setupTestRuntime({
      viewer: { activeImageSourcePath: "", currentPageIndex: 0, pageOrder: [2, 0, 1], rotation: 0, flipH: false, flipV: false },
      createExportController: vi.fn(({ getContext }) => { exportContext = getContext; return { exportCurrentPageImage: vi.fn() }; }),
    });
    expect(exportContext()).toMatchObject({ pageIndex: 2, displayPageIndex: 0 });
  });
});
