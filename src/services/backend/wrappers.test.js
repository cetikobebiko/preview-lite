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
import * as signatures from "./signatures.js";
import * as session from "./session.js";
import * as viewer from "./viewer.js";

const invokeMock = vi.hoisted(() => vi.fn());
const JOB_ID = "00000000-0000-4000-8000-000000000001";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const rect = { x: 1, y: 2, width: 3, height: 4 };
const redactions = [{ page_index: 0, ...rect }];
const sanitization = {
  remove_metadata: true,
  remove_annotations: false,
  remove_javascript: true,
  remove_embedded_files: true,
  remove_hidden_layers: false,
};
const signatureOptions = {
  reason: "Approval",
  location: "Istanbul",
  contact_info: null,
  visual_signature: null,
};
const history = [{ role: "user", content: "Earlier question" }];
const renderSession = {
  sessionId: "document-1",
  pageIndex: 2,
  contentGeneration: 7,
  requestId: "request-1",
};

const cases = [
  ["renderPage", () => viewer.renderPage({ ...renderSession, scale: 1.5, rotation: 90 }), "render_session_page", { sessionId: "document-1", pageIndex: 2, scale: 1.5, rotation: 90, contentGeneration: 7, requestId: "request-1" }],
  ["renderThumbnail", () => viewer.renderThumbnail({ ...renderSession, targetWidth: 180, scope: "sidebar_thumbnail" }), "render_session_thumbnail", { sessionId: "document-1", pageIndex: 2, targetWidth: 180, contentGeneration: 7, requestId: "request-1", scope: "sidebar_thumbnail" }],
  ["setActiveRenderDocument", () => viewer.setActiveRenderDocument({ sessionId: "document-1", contentGeneration: 7 }), "set_active_session_render_document", { sessionId: "document-1", contentGeneration: 7 }],
  ["cancelRenderRequest", () => viewer.cancelRenderRequest({ ...renderSession, scope: "main_page" }), "cancel_session_render_request", { sessionId: "document-1", pageIndex: 2, contentGeneration: 7, requestId: "request-1", scope: "main_page" }],
  ["getPageTextLayout", () => viewer.getPageTextLayout("session-1", 7, 2), "get_session_page_text_layout", { sessionId: "session-1", expectedContentGeneration: 7, pageIndex: 2 }],
  ["openDocumentSession", () => session.openDocumentSession("/a.pdf"), "open_document_session", { path: "/a.pdf" }],
  ["getActiveDocumentSessionSnapshot", () => session.getActiveDocumentSessionSnapshot(), "get_active_document_session_snapshot"],
  ["probeCandidateRender", () => session.probeCandidateRender("candidate-1", 1), "probe_candidate_render", { sessionId: "candidate-1", expectedContentGeneration: 1 }],
  ["saveDocumentSession", () => session.saveDocumentSession("session-1", 3), "save_document_session", { sessionId: "session-1", expectedStateRevision: 3 }],
  ["saveDocumentSessionAs", () => session.saveDocumentSessionAs("session-1", "/b.pdf", 3), "save_document_session_as", { sessionId: "session-1", targetPath: "/b.pdf", expectedStateRevision: 3 }],
  ["saveTransition", () => session.saveTransition("transition-1", null), "save_transition", { token: "transition-1", targetPath: null }],
  ["prepareCandidateActivation", () => session.prepareCandidateActivation("candidate-1", "session-1", "transition-1"), "prepare_candidate_activation", { candidateSessionId: "candidate-1", previousSessionId: "session-1", previousTransitionToken: "transition-1" }],
  ["cancelSessionJob", () => session.cancelSessionJob("session-1", JOB_ID), "cancel_session_job", { sessionId: "session-1", jobId: JOB_ID }],
  ["exportSessionPagesAsImages", () => session.exportSessionPagesAsImages("session-1", 7, [0, 1], "/tmp", "png", 300), "export_session_pages_as_images", { sessionId: "session-1", expectedContentGeneration: 7, pages: [0, 1], outputDir: "/tmp", format: "png", dpi: 300 }],
  ["exportSessionTextContent", () => session.exportSessionTextContent("session-1", 7, [0, 1], "markdown"), "export_session_text_content", { sessionId: "session-1", expectedContentGeneration: 7, pages: [0, 1], format: "markdown" }],
  ["printSessionPdf", () => session.printSessionPdf("session-1", 7, null), "print_session_pdf", { sessionId: "session-1", expectedContentGeneration: 7, pageRange: null }],
  ["extractSessionPages", () => session.extractSessionPages("session-1", 7, [0, 2], "/b.pdf"), "extract_session_pages", { sessionId: "session-1", expectedContentGeneration: 7, pageIndices: [0, 2], targetPath: "/b.pdf" }],
  ["getSessionStructureTree", () => session.getSessionStructureTree("session-1", 7), "get_session_structure_tree", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["validateSessionPdfUa", () => session.validateSessionPdfUa("session-1", 7), "validate_session_pdf_ua", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["getSessionPdfMetadata", () => session.getSessionPdfMetadata("session-1", 7), "get_session_pdf_metadata", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["updateSessionAltText", () => session.updateSessionAltText("session-1", 7, "4_0", "Figure", "tr-TR"), "update_session_alt_text", { sessionId: "session-1", expectedContentGeneration: 7, objectId: "4_0", altText: "Figure", language: "tr-TR" }],
  ["analyzeSessionColorSpaces", () => prepress.analyzeSessionColorSpaces("session-1", 7), "analyze_session_color_spaces", { sessionId: "session-1", expectedContentGeneration: 7 }],
  ["calculateSessionInkCoverage", () => prepress.calculateSessionInkCoverage("session-1", 7, 2, 300, 1.5), "calculate_session_ink_coverage", { sessionId: "session-1", expectedContentGeneration: 7, pageIndex: 2, maxLimit: 300, scale: 1.5 }],
  ["renderSessionSeparationChannels", () => prepress.renderSessionSeparationChannels("session-1", 7, 2, 1.5), "render_session_separation_channels", { sessionId: "session-1", expectedContentGeneration: 7, pageIndex: 2, scale: 1.5 }],
  ["exportSessionPdfX", () => prepress.exportSessionPdfX("session-1", 7, "/b.pdf", "PDF/X-4", "FOGRA39"), "export_session_pages_as_pdf_x", { sessionId: "session-1", expectedContentGeneration: 7, targetPath: "/b.pdf", standard: "PDF/X-4", profileName: "FOGRA39" }],

  ["readImageFile", () => document.readImageFile("/a.png", 90, true, false), "read_image_file", { path: "/a.png", rotation: 90, flipH: true, flipV: false }],
  ["convertAndSaveImage", () => document.convertAndSaveImage("/a.png", "/b.webp", "webp", 90, true, false), "convert_and_save_image", { sourcePath: "/a.png", targetPath: "/b.webp", format: "webp", rotation: 90, flipH: true, flipV: false }],
  ["burnAnnotationsToPdf", () => document.burnAnnotationsToPdf("sid", 1, "[]", "/b.pdf"), "burn_annotations_to_pdf", { sessionId: "sid", expectedContentGeneration: 1, annotationsJson: "[]", targetPath: "/b.pdf" }],

  ["mergePdfs", () => manipulation.mergePdfs(["/a.pdf", "/b.pdf"], "/out.pdf"), "merge_pdfs", { filePaths: ["/a.pdf", "/b.pdf"], outputPath: "/out.pdf" }],
  ["mergeSessionPdfs", () => manipulation.mergeSessionPdfs("sid", 1, ["/a.pdf", "/b.pdf"], "/out.pdf"), "merge_session_pdfs", { sessionId: "sid", expectedContentGeneration: 1, additionalPaths: ["/a.pdf", "/b.pdf"], targetPath: "/out.pdf" }],
  ["splitSessionPdf", () => manipulation.splitSessionPdf("sid", 1, [[0, 2], [3, 4]], "/tmp"), "split_session_pdf", { sessionId: "sid", expectedContentGeneration: 1, ranges: [[0, 2], [3, 4]], outputDir: "/tmp" }],
  ["detectScannedPage", () => ocr.detectScannedPage("sid", 1, 1), "detect_scanned_page", { sessionId: "sid", expectedContentGeneration: 1, pageIndex: 1 }],
  ["getAvailableOcrLanguages", () => ocr.getAvailableOcrLanguages(), "get_available_ocr_languages"],
  ["runOcrOnPage", () => ocr.runOcrOnPage("sid", 1, 1, "tur"), "run_ocr_on_page", { sessionId: "sid", expectedContentGeneration: 1, pageIndex: 1, lang: "tur" }],
  ["exportAsSearchablePdf", () => ocr.exportAsSearchablePdf("sid", 1, "/b.pdf", "tur"), "export_as_searchable_pdf", { sessionId: "sid", expectedContentGeneration: 1, outputPath: "/b.pdf", lang: "tur" }],

  ["buildSemanticIndex", () => ai.buildSemanticIndex("sid", 1, 500, 50, "http://localhost", "embed"), "build_semantic_index", { sessionId: "sid", expectedContentGeneration: 1, chunkSize: 500, chunkOverlap: 50, ollamaUrl: "http://localhost", embedModel: "embed" }],
  ["semanticSearch", () => ai.semanticSearch("sid", 1, "query", 3, "http://localhost", "embed"), "semantic_search", { sessionId: "sid", expectedContentGeneration: 1, query: "query", topK: 3, ollamaUrl: "http://localhost", embedModel: "embed" }],
  ["queryDocumentRag", () => ai.queryDocumentRag("sid", 1, "prompt", history, "http://localhost", "llm", "embed"), "query_document_rag", { sessionId: "sid", expectedContentGeneration: 1, prompt: "prompt", history, ollamaUrl: "http://localhost", llmModel: "llm", embedModel: "embed" }],
  ["summarizeSelectionOrPage", () => ai.summarizeSelectionOrPage("text", "short", "http://localhost", "llm"), "summarize_selection_or_page", { textContent: "text", targetLength: "short", ollamaUrl: "http://localhost", llmModel: "llm" }],

  ["getActivePlugins", () => plugins.getActivePlugins(), "get_active_plugins"],
  ["loadPlugin", () => plugins.loadPlugin("/manifest.json"), "load_plugin", { manifestPath: "/manifest.json" }],
  ["unloadPlugin", () => plugins.unloadPlugin("plugin-1"), "unload_plugin", { pluginId: "plugin-1" }],
  ["triggerPluginHook", () => plugins.triggerPluginHook("plugin-1", "open", "{}"), "trigger_plugin_hook", { pluginId: "plugin-1", hookName: "open", payload: "{}" }],
  ["runScriptDirect", () => plugins.runScriptDirect("40 + 2"), "run_script_direct", { scriptContent: "40 + 2" }],
  ["addBatchJob", () => plugins.addBatchJob("ocr", "/a.pdf", "{}"), "add_batch_job", { jobType: "ocr", filePath: "/a.pdf", parameters: "{}" }],

  ["listSystemCertificates", () => signatures.listSystemCertificates(), "list_system_certificates"],
  ["signPdfPades", () => signatures.signPdfPades("sid", 1, "/b.pdf", "cert", "1234", signatureOptions), "sign_pdf_pades", { sessionId: "sid", expectedContentGeneration: 1, outputPath: "/b.pdf", certId: "cert", pin: "1234", options: signatureOptions }],
  ["verifyPdfSignatures", () => signatures.verifyPdfSignatures("sid", 1), "verify_pdf_signatures", { sessionId: "sid", expectedContentGeneration: 1 }],
  ["enableLtv", () => signatures.enableLtv("sid", 1, "sig-1"), "enable_ltv", { sessionId: "sid", expectedContentGeneration: 1, signatureId: "sig-1" }],

  ["applyRedactions", () => redaction.applyRedactions("sid", 1, "/b.pdf", redactions), "apply_redactions", { sessionId: "sid", expectedContentGeneration: 1, outputPath: "/b.pdf", redactions }],
  ["sanitizePdfDocument", () => redaction.sanitizePdfDocument("sid", 1, "/b.pdf", sanitization), "sanitize_pdf_document", { sessionId: "sid", expectedContentGeneration: 1, outputPath: "/b.pdf", options: sanitization }],
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
        value:
          command === "scan_sensitive_data"
            ? { schema_version: 2, items: [], warnings: [], truncated: false }
            : "result",
        cancelRequested: false,
        cancellationReason: null,
        cleanup: { status: "cleaned" },
      };
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
    const expected =
      command === "scan_sensitive_data"
        ? { schemaVersion: 2, items: [], warnings: [], truncated: false }
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
