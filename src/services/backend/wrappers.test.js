import { beforeEach, describe, expect, it, vi } from "vitest";
import * as ai from "./ai.js";
import { CONTRACTS } from "./contracts.js";
import * as document from "./document.js";
import * as manipulation from "./manipulation.js";
import * as ocr from "./ocr.js";
import * as platform from "./platform.js";
import * as plugins from "./plugins.js";
import * as prepress from "./prepress.js";
import * as redaction from "./redaction.js";
import * as session from "./session.js";
import * as viewer from "./viewer.js";
import { encodeRenderFrame } from "./renderFrameTestUtils.js";

const invokeMock = vi.hoisted(() => vi.fn());
const JOB_ID = "00000000-0000-4000-8000-000000000001";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const rect = { x: 1, y: 2, width: 3, height: 4 };
const redactions = [{ page_index: 0, ...rect }];
const history = [{ role: "user", content: "Earlier question" }];
const renderSession = {
  sessionId: "document-1",
  pageIndex: 2,
  contentGeneration: 7,
  requestId: "request-1",
};
const accessibilityAudit = {
  schemaVersion: "accessibility-audit/2",
  auditVersion: "basic-accessibility-audit/1",
  referenceProfile: "pdf_ua1_reference",
  summaryStatus: "findings_detected",
  documentFacts: {
    headerVersion: "1.7", catalogVersion: null, effectiveVersion: "1.7",
    pdfuaDeclaration: { part: null, revision: null, state: "missing" },
    pageCount: 1, encrypted: false, hasStructureTree: false, hasMarkInfo: false, hasMarkedEntry: false, marked: false,
    displayDocumentTitle: false, documentLanguage: "missing", xmpTitle: "missing", infoTitle: "missing",
  },
  checks: [],
  coverage: { implementedCheckIds: [], notImplementedCheckIds: [], incompleteCheckIds: [], machineCheckCount: 0, humanReviewCount: 0, categories: [], limitations: [] },
  humanReview: [], warnings: [], truncated: false,
};
const accessibilityStructure = {
  schemaVersion: "accessibility-structure/1", inspectionVersion: "bounded-structure-inspection/1",
  root: null, findings: [], warnings: [], truncated: false, limitations: [],
};
const renderFrame = {
  width: 1,
  height: 1,
  data: new Uint8Array([10, 20, 30, 255]),
  pageIndex: 2,
  generation: 7,
  requestId: "request-1",
};
const decodedRenderFrame = { ...renderFrame, data: new Uint8ClampedArray(renderFrame.data) };
const decodedImageFrame = { width: 1, height: 1, data: new Uint8ClampedArray(renderFrame.data) };
const prepressIdentity = { sessionId: "session-1", contentGeneration: 7, physicalPageIndex: 2, requestId: "request-1", methodologyVersion: "naive-rgb-to-cmyk/1" };
const previewMetadata = { schemaVersion: "rgb-derived-print-preview/1", methodology: "naive-rgb-to-cmyk/1", sourceColorModel: "pdfium_display_rgb", approximate: true, colorManaged: false, outputIntentApplied: false, spotColorsPreserved: false, overprintSimulated: false, renderingIntent: "not_applied", warnings: [] };
const emptyInspectionCounts = { visitedObjects: 0, resourceReferences: 0, xobjects: 0, patterns: 0, shadings: 0, colorSpaceDefinitions: 0, uniqueColorSpaceObjects: 0, iccProfiles: 0, outputIntents: 0, spotColors: 0, fonts: 0, transparencyObservations: 0 };
const inspectionReport = { schemaVersion: "print-resource-inspection/1", inspectionVersion: "print-resource-inspection/1.0", identity: { ...prepressIdentity, methodologyVersion: "print-resource-inspection/1.0" }, summaryStatus: "inspection_incomplete", documentFacts: { pdfVersion: "1.7", pageCount: 1, encrypted: false }, colorSpaces: [], iccProfiles: [], outputIntents: [], spotColors: [], transparency: [], fonts: [], pageBoxes: [], coverage: { inspectedAreas: [], notInspectedAreas: ["content_stream_usage"], incompleteAreas: [], limitations: [], resourceCounts: emptyInspectionCounts, pagesInspected: 1, areas: [] }, warnings: [], truncated: false, conformanceClaim: false };
const coveragePreview = { identity: prepressIdentity, metadata: previewMetadata, pageIndex: 2, width: 1, height: 1, thresholdPercent: 300, heatmapPngBase64: "AAAA", encodedPngBytes: 3, pixelStatistics: { pixelsAboveThreshold: 0, totalPixels: 1, maximumEstimatedChannelSumPercent: 0, averageEstimatedChannelSumPercent: 0 }, truncated: false };
const processPreview = { identity: prepressIdentity, metadata: previewMetadata, pageIndex: 2, width: 1, height: 1, cPreviewPngBase64: "AAAA", mPreviewPngBase64: "AAAA", yPreviewPngBase64: "AAAA", kPreviewPngBase64: "AAAA", encodedPngBytes: [3, 3, 3, 3], totalEncodedPngBytes: 12, truncated: false };

const cases = [
  ["renderPage", () => viewer.renderPage({ ...renderSession, scale: 1.5, rotation: 90 }), "render_session_page", { sessionId: "document-1", pageIndex: 2, scale: 1.5, rotation: 90, contentGeneration: 7, requestId: "request-1" }],
  ["renderThumbnail", () => viewer.renderThumbnail({ ...renderSession, targetWidth: 180, scope: "sidebar_thumbnail" }), "render_session_thumbnail", { sessionId: "document-1", pageIndex: 2, targetWidth: 180, contentGeneration: 7, requestId: "request-1", scope: "sidebar_thumbnail" }],
  ["setActiveRenderDocument", () => viewer.setActiveRenderDocument({ sessionId: "document-1", contentGeneration: 7 }), "set_active_session_render_document", { sessionId: "document-1", contentGeneration: 7 }],
  ["cancelRenderRequest", () => viewer.cancelRenderRequest({ ...renderSession, scope: "main_page" }), "cancel_session_render_request", { sessionId: "document-1", pageIndex: 2, contentGeneration: 7, requestId: "request-1", scope: "main_page" }],
  ["cancelRenderRequestWithReason", () => viewer.cancelRenderRequest({ ...renderSession, scope: "sidebar_thumbnail" }, "viewport_exit"), "cancel_session_render_request", { sessionId: "document-1", pageIndex: 2, contentGeneration: 7, requestId: "request-1", scope: "sidebar_thumbnail", reason: "viewport_exit" }],
  ["getPageTextLayout", () => viewer.getPageTextLayout("session-1", 7, 2), "get_session_page_text_layout", { sessionId: "session-1", expectedContentGeneration: 7, pageIndex: 2 }],
  ["getPageSizes", () => viewer.getPageSizes("session-1", 7), "get_session_page_sizes", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["retryPage", () => viewer.retryPage("session-1", 7, 2), "retry_session_page", { sessionId: "session-1", expectedContentGeneration: 7, pageIndex: 2 }],
  ["openDocumentSession", () => session.openDocumentSession("/a.pdf"), "open_document_session", { path: "/a.pdf" }],
  ["getActiveDocumentSessionSnapshot", () => session.getActiveDocumentSessionSnapshot(), "get_active_document_session_snapshot"],
  ["probeCandidateRender", () => session.probeCandidateRender("candidate-1", 1), "probe_candidate_render", { sessionId: "candidate-1", expectedContentGeneration: 1 }],
  ["saveDocumentSession", () => session.saveDocumentSession("session-1", 3), "save_document_session", { sessionId: "session-1", expectedStateRevision: 3 }],
  ["saveDocumentSessionAs", () => session.saveDocumentSessionAs("session-1", "/b.pdf", 3), "save_document_session_as", { sessionId: "session-1", targetPath: "/b.pdf", expectedStateRevision: 3 }],
  ["saveTransition", () => session.saveTransition("transition-1", null), "save_transition", { token: "transition-1", targetPath: null }],
  ["prepareCandidateActivation", () => session.prepareCandidateActivation("candidate-1", "session-1", "transition-1"), "prepare_candidate_activation", { candidateSessionId: "candidate-1", previousSessionId: "session-1", previousTransitionToken: "transition-1" }],
  ["cancelSessionJob", () => session.cancelSessionJob("session-1", JOB_ID), "cancel_session_job", { sessionId: "session-1", jobId: JOB_ID }],
  ["exportSessionPagesAsImages", () => session.exportSessionPagesAsImages("session-1", 7, [0, 1], "/tmp", "png", 300), "export_session_pages_as_images", { sessionId: "session-1", expectedContentGeneration: 7, pages: [0, 1], outputDir: "/tmp", format: "png", dpi: 300, overwrite: false }],
  ["exportSessionPagesAsImages overwrite", () => session.exportSessionPagesAsImages("session-1", 7, [0, 1], "/tmp", "png", 300, true), "export_session_pages_as_images", { sessionId: "session-1", expectedContentGeneration: 7, pages: [0, 1], outputDir: "/tmp", format: "png", dpi: 300, overwrite: true }],
  ["exportSessionTextContent", () => session.exportSessionTextContent("session-1", 7, [0, 1], "markdown"), "export_session_text_content", { sessionId: "session-1", expectedContentGeneration: 7, pages: [0, 1], format: "markdown" }],
  ["printSessionPdf", () => session.printSessionPdf("session-1", 7, null), "print_session_pdf", { sessionId: "session-1", expectedContentGeneration: 7, pageRange: null }],
  ["extractSessionPages", () => session.extractSessionPages("session-1", 7, [0, 2], "/b.pdf"), "extract_session_pages", { sessionId: "session-1", expectedContentGeneration: 7, pageIndices: [0, 2], targetPath: "/b.pdf" }],
  ["getSessionStructureTree", () => session.getSessionStructureTree("session-1", 7), "get_session_structure_tree", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["auditSessionAccessibility", () => session.auditSessionAccessibility("session-1", 7), "audit_session_accessibility", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["getSessionPdfMetadata", () => session.getSessionPdfMetadata("session-1", 7), "get_session_pdf_metadata", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["inspectSessionPrintResources", () => prepress.inspectSessionPrintResources("session-1", 7, 2, "request-1").promise, "inspect_session_print_resources", { sessionId: "session-1", expectedContentGeneration: 7, physicalPageIndex: 2, requestId: "request-1" }],
  ["generateSessionRgbDerivedCoveragePreview", () => prepress.generateSessionRgbDerivedCoveragePreview("session-1", 7, 2, "request-1", 300, 1.5).promise, "generate_session_rgb_derived_coverage_preview", { sessionId: "session-1", expectedContentGeneration: 7, physicalPageIndex: 2, requestId: "request-1", thresholdPercent: 300, scale: 1.5 }],
  ["generateSessionRgbDerivedProcessPreview", () => prepress.generateSessionRgbDerivedProcessPreview("session-1", 7, 2, "request-1", 1.5).promise, "generate_session_rgb_derived_process_preview", { sessionId: "session-1", expectedContentGeneration: 7, physicalPageIndex: 2, requestId: "request-1", scale: 1.5 }],

  ["readImageFile", () => document.readImageFile("/a.png", 90, true, false), "read_image_file", { path: "/a.png", rotation: 90, flipH: true, flipV: false }],
  ["convertAndSaveImage", () => document.convertAndSaveImage("/a.png", "/b.webp", "webp", 90, true, false), "convert_and_save_image", { sourcePath: "/a.png", targetPath: "/b.webp", format: "webp", rotation: 90, flipH: true, flipV: false }],

  ["mergePdfs", () => manipulation.mergePdfs(["/a.pdf", "/b.pdf"], "/out.pdf"), "merge_pdfs", { filePaths: ["/a.pdf", "/b.pdf"], outputPath: "/out.pdf" }],
  ["mergeSessionPdfs", () => manipulation.mergeSessionPdfs("sid", 1, ["/a.pdf", "/b.pdf"], "/out.pdf"), "merge_session_pdfs", { sessionId: "sid", expectedContentGeneration: 1, additionalPaths: ["/a.pdf", "/b.pdf"], targetPath: "/out.pdf" }],
  ["splitSessionPdf", () => manipulation.splitSessionPdf("sid", 1, [[0, 2], [3, 4]], "/tmp"), "split_session_pdf", { sessionId: "sid", expectedContentGeneration: 1, ranges: [[0, 2], [3, 4]], outputDir: "/tmp", overwrite: false }],
  ["splitSessionPdf overwrite", () => manipulation.splitSessionPdf("sid", 1, [[0, 2], [3, 4]], "/tmp", true), "split_session_pdf", { sessionId: "sid", expectedContentGeneration: 1, ranges: [[0, 2], [3, 4]], outputDir: "/tmp", overwrite: true }],
  ["detectScannedPages", () => ocr.detectScannedPages("sid", 1), "detect_scanned_pages", { sessionId: "sid", expectedContentGeneration: 1 }],
  ["getAvailableOcrLanguages", () => ocr.getAvailableOcrLanguages(), "get_available_ocr_languages"],
  ["runOcrOnPage", () => ocr.runOcrOnPage("sid", 1, 1, "tur"), "run_ocr_on_page", { sessionId: "sid", expectedContentGeneration: 1, pageIndex: 1, lang: "tur" }],
  ["exportAsSearchablePdf", () => ocr.exportAsSearchablePdf("sid", 1, "/b.pdf", "tur"), "export_as_searchable_pdf", { sessionId: "sid", expectedContentGeneration: 1, outputPath: "/b.pdf", lang: "tur" }],

  ["configureAiEndpoint", () => ai.configureAiEndpoint("http://127.0.0.1:11434", false, false), "configure_ai_endpoint", { url: "http://127.0.0.1:11434", remoteApproved: false, privateNetworkApproved: false }],
  ["previewAiEndpoint", () => ai.previewAiEndpoint("http://127.0.0.1:11434"), "preview_ai_endpoint", { url: "http://127.0.0.1:11434" }],
  ["getAiEndpointStatus", () => ai.getAiEndpointStatus(), "get_ai_endpoint_status"],
  ["clearAiEndpointConfig", () => ai.clearAiEndpointConfig(), "clear_ai_endpoint_config"],
  ["cancelAiRequest", () => ai.cancelAiRequest("req_00000000-0000-4000-8000-000000000001", "sid", 1), "cancel_ai_request", { requestId: "req_00000000-0000-4000-8000-000000000001", sessionId: "sid", expectedContentGeneration: 1 }],

  ["buildSemanticIndex", () => ai.buildSemanticIndex("sid", 1, 500, 50, "embed", "req_00000000-0000-4000-8000-000000000001"), "build_semantic_index", { sessionId: "sid", expectedContentGeneration: 1, chunkSize: 500, chunkOverlap: 50, embedModel: "embed", requestId: "req_00000000-0000-4000-8000-000000000001" }],
  ["semanticSearch", () => ai.semanticSearch("sid", 1, "query", 3, "embed", 500, 50, "req_00000000-0000-4000-8000-000000000001"), "semantic_search", { sessionId: "sid", expectedContentGeneration: 1, query: "query", topK: 3, embedModel: "embed", expectedChunkSize: 500, expectedChunkOverlap: 50, requestId: "req_00000000-0000-4000-8000-000000000001" }],
  ["queryDocumentRag", () => ai.queryDocumentRag("sid", 1, "prompt", history, "llm", "embed", 500, 50, "req_00000000-0000-4000-8000-000000000001"), "query_document_rag", { sessionId: "sid", expectedContentGeneration: 1, prompt: "prompt", history, llmModel: "llm", embedModel: "embed", expectedChunkSize: 500, expectedChunkOverlap: 50, requestId: "req_00000000-0000-4000-8000-000000000001" }],
  ["summarizeSelectionOrPage", () => ai.summarizeSelectionOrPage("text", "short", "llm", "req_00000000-0000-4000-8000-000000000001", "attempt_00000000-0000-4000-8000-000000000001"), "summarize_selection_or_page", { textContent: "text", targetLength: "short", llmModel: "llm", requestId: "req_00000000-0000-4000-8000-000000000001", callerAttemptId: "attempt_00000000-0000-4000-8000-000000000001" }],

  ["inspectPluginPackage", () => plugins.inspectPluginPackage("/pkg"), "inspect_plugin_package", { packagePath: "/pkg" }],
  ["installPluginPackage", () => plugins.installPluginPackage("/pkg", "digest123"), "install_plugin_package", { packagePath: "/pkg", expectedDigest: "digest123" }],
  ["uninstallPlugin", () => plugins.uninstallPlugin("plugin-1"), "uninstall_plugin", { pluginId: "plugin-1" }],
  ["setPluginEnabled", () => plugins.setPluginEnabled("plugin-1", true), "set_plugin_enabled", { pluginId: "plugin-1", enabled: true }],
  ["listInstalledPlugins", () => plugins.listInstalledPlugins(), "list_installed_plugins"],
  ["startPluginExecution", () => plugins.startPluginExecution("plugin-1", "open", "{}", "exec-1"), "start_plugin_execution", { pluginId: "plugin-1", hookName: "open", payloadJson: "{}", executionId: "exec-1" }],
  ["cancelPluginExecution", () => plugins.cancelPluginExecution("exec-1"), "cancel_plugin_execution", { executionId: "exec-1" }],
  ["getPluginExecutionStatus", () => plugins.getPluginExecutionStatus("exec-1"), "get_plugin_execution_status", { executionId: "exec-1" }],
  ["addBatchJob", () => plugins.addBatchJob("ocr", "/a.pdf", "{}"), "add_batch_job", { jobType: "ocr", filePath: "/a.pdf", parameters: "{}" }],


  ["applyRedactions", () => redaction.applyRedactions("sid", 1, "/b.pdf", redactions), "apply_redactions", { sessionId: "sid", expectedContentGeneration: 1, outputPath: "/b.pdf", redactions }],
  ["scanSensitiveData", () => redaction.scanSensitiveData("sid", 1, ["secret"]), "scan_sensitive_data", { sessionId: "sid", expectedContentGeneration: 1, patterns: ["secret"] }],

  ["speakText", () => platform.speakText("hello", "Yelda"), "speak_text", { text: "hello", voice: "Yelda" }],
  ["stopSpeaking", () => platform.stopSpeaking(), "stop_speaking"],
  ["shareFileMacos", () => platform.shareFileMacos("sid", 1), "share_file_macos", { sessionId: "sid", expectedContentGeneration: 1 }],
  ["shareExternalFileMacos", () => platform.shareExternalFileMacos("/a.png"), "share_external_file_macos", { path: "/a.png" }],
  ["performOcrOnPage", () => platform.performOcrOnPage("sid", 1, 2), "perform_ocr_on_page", { sessionId: "sid", expectedContentGeneration: 1, pageIndex: 2 }],
];

beforeEach(() => {
  invokeMock.mockReset();
  vi.spyOn(globalThis.crypto, "randomUUID").mockReturnValue(JOB_ID);
  invokeMock.mockImplementation(async (command) => {
    const contract = Object.values(CONTRACTS).find(
      (candidate) => candidate.command === command,
    );
    if (contract?.payload?.required?.jobId && command !== "cancel_session_job") {
      return {
        status: "completed",
        jobId: JOB_ID,
        sessionId: "session-1",
        contentGeneration: 1,
        value: command === "scan_sensitive_data" ? { schema_version: 2, items: [], warnings: [], truncated: false }
          : command === "inspect_session_print_resources" ? inspectionReport
          : command === "generate_session_rgb_derived_coverage_preview" ? coveragePreview
          : command === "generate_session_rgb_derived_process_preview" ? processPreview
          : "result",
        cancelRequested: false,
        cancellationReason: null,
        cleanup: { status: "cleaned" },
      };
    }
    if (command === "audit_session_accessibility") return accessibilityAudit;
    if (command === "get_session_structure_tree") return accessibilityStructure;
    if (command === "render_session_page" || command === "render_session_thumbnail") {
      return encodeRenderFrame(renderFrame);
    }
    if (command === "read_image_file") {
      return encodeRenderFrame({ ...renderFrame, pageIndex: 0, generation: 0, requestId: "read_image_file" });
    }
    return "result";
  });
});

describe("canonical backend wrappers", () => {
  it("keeps every exercised wrapper aligned with a unique contract", () => {
    expect(Object.keys(CONTRACTS).length).toBeGreaterThanOrEqual(cases.length);
    expect(
      new Set(Object.values(CONTRACTS).map((contract) => contract.command)).size,
    ).toBe(Object.keys(CONTRACTS).length);
  });

  it.each(cases)("%s sends the exact command and payload", async (_name, action, command, payload) => {
    const expected = command === "audit_session_accessibility"
      ? accessibilityAudit
      : command === "get_session_structure_tree"
        ? accessibilityStructure
      : command === "scan_sensitive_data"
        ? { schemaVersion: 2, items: [], warnings: [], truncated: false }
      : command === "inspect_session_print_resources" ? inspectionReport
      : command === "generate_session_rgb_derived_coverage_preview" ? coveragePreview
      : command === "generate_session_rgb_derived_process_preview" ? processPreview
      : command === "render_session_page" || command === "render_session_thumbnail" ? decodedRenderFrame
      : command === "read_image_file" ? decodedImageFrame
        : "result";
    await expect(action()).resolves.toEqual(expected);
    if (payload === undefined) {
      expect(invokeMock).toHaveBeenCalledWith(command);
    } else {
      const requiresJobId = Object.values(CONTRACTS).find(
        (contract) => contract.command === command,
      )?.payload?.required?.jobId;
      expect(invokeMock).toHaveBeenCalledWith(
        command,
        requiresJobId ? { jobId: JOB_ID, ...payload } : payload,
      );
    }
    expect(invokeMock).toHaveBeenCalledOnce();
  });
});
