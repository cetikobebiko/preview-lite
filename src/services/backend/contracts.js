import { decodeRenderFrame } from "./renderFrame.js";

const stringType = Object.freeze({ type: "string" });
const booleanType = Object.freeze({ type: "boolean" });
const numberType = Object.freeze({ type: "number" });
const u32Type = Object.freeze({
  type: "number",
  integer: true,
  min: 0,
  max: 4_294_967_295,
});
const usizeType = Object.freeze({
  type: "number",
  integer: true,
  min: 0,
  max: Number.MAX_SAFE_INTEGER,
});
const i32Type = Object.freeze({
  type: "number",
  integer: true,
  min: -2_147_483_648,
  max: 2_147_483_647,
});
const safeIntegerType = Object.freeze({
  type: "number",
  integer: true,
  min: 0,
  max: Number.MAX_SAFE_INTEGER,
});
const renderIdentifierType = Object.freeze({
  type: "string",
  minLength: 1,
  maxLength: 128,
});
const transitionKindType = Object.freeze({
  type: "string",
  enum: Object.freeze(["document_switch", "document_close", "window_close"]),
});
const renderScopeType = Object.freeze({
  type: "string",
  enum: Object.freeze([
    "main_page",
    "sidebar_thumbnail",
    "organizer_thumbnail",
  ]),
});
// Diagnostic only; mirrors `RenderCancelReason` in render_service.rs.
const renderCancelReasonType = Object.freeze({
  type: "string",
  enum: Object.freeze([
    "superseded",
    "viewport_exit",
    "zoom",
    "rerender",
    "timeout",
    "destroy",
    "unspecified",
  ]),
});
const renderScaleType = Object.freeze({
  type: "number",
  min: Number.MIN_VALUE,
  max: 16,
});
const renderTargetWidthType = Object.freeze({
  type: "number",
  integer: true,
  min: 1,
  max: 16_384,
});

const nullable = (schema) => Object.freeze({ ...schema, nullable: true });
const arrayOf = (items) => Object.freeze({ type: "array", items });
const tupleOf = (...items) => Object.freeze({ type: "tuple", items });
const objectOf = (required = {}, optional = {}, validate = null) =>
  Object.freeze({
    type: "object",
    required: Object.freeze(required),
    optional: Object.freeze(optional),
    unknownKeys: "reject",
    validate,
  });

const pdfMetadata = objectOf({
  title: stringType,
  author: stringType,
  subject: stringType,
  keywords: stringType,
  creator: stringType,
});

const chatMessage = objectOf({
  role: stringType,
  content: stringType,
});

const annotationRect = objectOf({
  x: numberType,
  y: numberType,
  width: numberType,
  height: numberType,
});

const redactionArea = objectOf({
  page_index: u32Type,
  x: numberType,
  y: numberType,
  width: numberType,
  height: numberType,
});

const sanitizationAcknowledgementsWire = objectOf({
  acknowledgeSignatureInvalidation: booleanType,
});

const sanitizationPolicyWire = objectOf({
  schemaVersion: stringType,
  profile: stringType,
  controls: arrayOf(stringType),
  acknowledgements: sanitizationAcknowledgementsWire,
  expectedInspectionVersion: stringType,
});

const redactionAppearancePolicyWire = objectOf({
  kind: stringType,
  color: stringType,
  opacity: numberType,
});

const secureRedactionPolicyWire = objectOf({
  schema_version: stringType,
  profile: stringType,
  strategy: stringType,
  raster_dpi: u32Type,
  appearance: redactionAppearancePolicyWire,
  safety_margin_points: numberType,
  interactive_content_policy: stringType,
  signature_policy: stringType,
  encryption_policy: stringType,
});

const redactionAcknowledgementsWire = objectOf({
  schema_version: stringType,
  rasterized_pages_accepted: booleanType,
  text_search_loss_accepted: booleanType,
  accessibility_loss_accepted: booleanType,
  annotation_loss_accepted: booleanType,
  form_loss_accepted: booleanType,
  signature_invalidation_accepted: booleanType,
  encryption_change_accepted: booleanType,
});

const normalizedTopLeftRectWire = objectOf({
  x: numberType,
  y: numberType,
  width: numberType,
  height: numberType,
});

const redactionRegionInputWire = objectOf({
  id: stringType,
  page_index: u32Type,
  normalized_top_left_rect: normalizedTopLeftRectWire,
  source: stringType,
});

const command = (name, required = {}, optional = {}, validate = null) =>
  Object.freeze({
    command: name,
    payload: objectOf(required, optional, validate),
  });

// Raw `tauri::ipc::Response` bodies (ArrayBuffer); errors remain JSON.
const binaryRenderFrameResponse = Object.freeze({
  type: "binary",
  format: "render-frame/1",
  decode: decodeRenderFrame,
});
// `read_image_file` reuses render-frame/1; its page/generation/request ID
// fields are fixed placeholders, so only the image dimensions and pixels
// are exposed.
const binaryImageFrameResponse = Object.freeze({
  type: "binary",
  format: "render-frame/1",
  decode: (raw) => {
    const { width, height, data } = decodeRenderFrame(raw);
    return { width, height, data };
  },
});
const withResponse = (contract, response) =>
  Object.freeze({ ...contract, response });

const commandWithoutPayload = (name) =>
  Object.freeze({
    command: name,
    payload: null,
  });

export const CONTRACTS = Object.freeze({
  openDocumentSession: command("open_document_session", {
    path: stringType,
  }),
  authenticateDocumentSession: command("authenticate_document_session", {
    sessionId: renderIdentifierType,
    password: stringType,
  }),
  getDocumentSessionSnapshot: command("get_document_session_snapshot", {
    sessionId: renderIdentifierType,
  }),
  getActiveDocumentSessionSnapshot: commandWithoutPayload(
    "get_active_document_session_snapshot",
  ),
  probeCandidateRender: command("probe_candidate_render", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  recoverDocumentSession: command("recover_document_session", {
    sessionId: renderIdentifierType,
  }),
  cancelSessionJob: command("cancel_session_job", {
    sessionId: renderIdentifierType,
    jobId: renderIdentifierType,
  }),
  exportSessionPageToImage: command("export_session_page_to_image", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
    targetPath: stringType,
    format: stringType,
  }),
  exportSessionPagesAsImages: command("export_session_pages_as_images", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pages: arrayOf(u32Type),
    outputDir: stringType,
    format: stringType,
    dpi: numberType,
  }),
  exportSessionTextContent: command("export_session_text_content", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pages: arrayOf(u32Type),
    format: stringType,
  }),
  printSessionPdf: command(
    "print_session_pdf",
    {
      jobId: renderIdentifierType,
      sessionId: renderIdentifierType,
      expectedContentGeneration: safeIntegerType,
    },
    {
      pageRange: nullable(arrayOf(u32Type)),
    },
  ),
  inspectSessionPrintResources: command("inspect_session_print_resources", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    physicalPageIndex: u32Type,
    requestId: renderIdentifierType,
  }),
  generateSessionRgbDerivedCoveragePreview: command("generate_session_rgb_derived_coverage_preview", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    physicalPageIndex: u32Type,
    requestId: renderIdentifierType,
    thresholdPercent: numberType,
    scale: numberType,
  }),
  generateSessionRgbDerivedProcessPreview: command("generate_session_rgb_derived_process_preview", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    physicalPageIndex: u32Type,
    requestId: renderIdentifierType,
    scale: numberType,
  }),
  extractSessionPages: command("extract_session_pages", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndices: arrayOf(u32Type),
    targetPath: stringType,
  }),
  discardCandidateSession: command("discard_candidate_session", {
    sessionId: renderIdentifierType,
  }),
  getSessionPageCount: command("get_session_page_count", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  getSessionPageSizes: command("get_session_page_sizes", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  getSessionPdfOutline: command("get_session_pdf_outline", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  getSessionPdfMetadata: command("get_session_pdf_metadata", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  getSessionStructureTree: command("get_session_structure_tree", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  auditSessionAccessibility: command("audit_session_accessibility", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  searchSessionPdfText: command("search_session_pdf_text", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    query: stringType,
  }),
  getSessionPageTextLayout: command("get_session_page_text_layout", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
  }),
  retrySessionPage: command("retry_session_page", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
  }),
  renderSessionPage: withResponse(command(
    "render_session_page",
    {
      sessionId: renderIdentifierType,
      pageIndex: u32Type,
      scale: renderScaleType,
      rotation: i32Type,
      contentGeneration: safeIntegerType,
      requestId: renderIdentifierType,
    },
    {},
    (value) =>
      [0, 90, 180, 270].includes(value.rotation)
        ? null
        : "rotation must be 0, 90, 180, or 270",
  ), binaryRenderFrameResponse),
  renderSessionThumbnail: withResponse(command(
    "render_session_thumbnail",
    {
      sessionId: renderIdentifierType,
      pageIndex: u32Type,
      targetWidth: renderTargetWidthType,
      contentGeneration: safeIntegerType,
      requestId: renderIdentifierType,
      scope: renderScopeType,
    },
    {},
    (value) =>
      value.scope === "sidebar_thumbnail" ||
      value.scope === "organizer_thumbnail"
        ? null
        : "thumbnail consumer scope must be sidebar_thumbnail or organizer_thumbnail",
  ), binaryRenderFrameResponse),
  setActiveSessionRenderDocument: command(
    "set_active_session_render_document",
    {
      sessionId: nullable(renderIdentifierType),
      contentGeneration: safeIntegerType,
    },
  ),
  cancelSessionRenderRequest: command("cancel_session_render_request", {
    sessionId: renderIdentifierType,
    pageIndex: u32Type,
    contentGeneration: safeIntegerType,
    requestId: renderIdentifierType,
    scope: renderScopeType,
  }, {
    reason: renderCancelReasonType,
  }),
  rotateSessionPage: command("rotate_session_page", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
    degrees: i32Type,
  }),
  deleteSessionPages: command("delete_session_pages", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndices: arrayOf(u32Type),
  }),
  reorderSessionPages: command("reorder_session_pages", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    newOrder: arrayOf(u32Type),
  }),
  insertSessionBlankPage: command("insert_session_blank_page", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    atIndex: u32Type,
    width: numberType,
    height: numberType,
  }),
  getSessionPageAnnotations: command("get_session_page_annotations", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
  }),
  addSessionAnnotation: command(
    "add_session_annotation",
    {
      sessionId: renderIdentifierType,
      expectedContentGeneration: safeIntegerType,
      pageIndex: u32Type,
      annotType: stringType,
      rect: annotationRect,
      color: stringType,
    },
    {
      contents: nullable(stringType),
    },
  ),
  deleteSessionAnnotation: command("delete_session_annotation", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
    annotId: stringType,
  }),
  updateSessionMetadata: command("update_session_metadata", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    metadata: pdfMetadata,
  }),
  saveDocumentSession: command("save_document_session", {
    sessionId: renderIdentifierType,
    expectedStateRevision: safeIntegerType,
  }),
  saveDocumentSessionAs: command("save_document_session_as", {
    sessionId: renderIdentifierType,
    targetPath: stringType,
    expectedStateRevision: safeIntegerType,
  }),
  beginSessionTransition: command("begin_session_transition", {
    sessionId: renderIdentifierType,
    kind: transitionKindType,
  }),
  saveTransition: command("save_transition", {
    token: renderIdentifierType,
    targetPath: nullable(stringType),
  }),
  discardTransition: command("discard_transition", {
    token: renderIdentifierType,
  }),
  cancelTransition: command("cancel_transition", {
    token: renderIdentifierType,
  }),
  prepareCandidateActivation: command("prepare_candidate_activation", {
    candidateSessionId: renderIdentifierType,
    previousSessionId: nullable(renderIdentifierType),
    previousTransitionToken: nullable(renderIdentifierType),
  }, {}, (value) =>
    (value.previousSessionId === null) !== (value.previousTransitionToken === null)
      ? "previousSessionId and previousTransitionToken must both be null or both be strings"
      : null),
  commitCandidateActivation: command("commit_candidate_activation", {
    activationToken: renderIdentifierType,
  }),
  rollbackCandidateActivation: command("rollback_candidate_activation", {
    activationToken: renderIdentifierType,
  }),
  closeAllDocumentSessions: commandWithoutPayload("close_all_document_sessions"),
  readImageFile: withResponse(command("read_image_file", {
    path: stringType,
    rotation: i32Type,
    flipH: booleanType,
    flipV: booleanType,
  }), binaryImageFrameResponse),
  convertAndSaveImage: command("convert_and_save_image", {
    sourcePath: stringType,
    targetPath: stringType,
    format: stringType,
    rotation: i32Type,
    flipH: booleanType,
    flipV: booleanType,
  }),
  burnAnnotationsToPdf: command("burn_annotations_to_pdf", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    annotationsJson: stringType,
    targetPath: stringType,
  }),
  mergePdfs: command("merge_pdfs", {
    filePaths: arrayOf(stringType),
    outputPath: stringType,
  }),
  mergeSessionPdfs: command("merge_session_pdfs", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    additionalPaths: arrayOf(stringType),
    targetPath: stringType,
  }),
  splitSessionPdf: command("split_session_pdf", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    ranges: arrayOf(tupleOf(u32Type, u32Type)),
    outputDir: stringType,
  }),
  detectScannedPages: command("detect_scanned_pages", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  getAvailableOcrLanguages: commandWithoutPayload(
    "get_available_ocr_languages",
  ),
  runOcrOnPage: command("run_ocr_on_page", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
    lang: stringType,
  }),
  exportAsSearchablePdf: command("export_as_searchable_pdf", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    outputPath: stringType,
    lang: stringType,
  }),

  configureAiEndpoint: command("configure_ai_endpoint", {
    url: stringType,
    remoteApproved: booleanType,
    privateNetworkApproved: booleanType,
  }),
  previewAiEndpoint: command("preview_ai_endpoint", {
    url: stringType,
  }),
  getAiEndpointStatus: commandWithoutPayload("get_ai_endpoint_status"),
  clearAiEndpointConfig: commandWithoutPayload("clear_ai_endpoint_config"),
  cancelAiRequest: command(
    "cancel_ai_request",
    { requestId: stringType },
    {
      sessionId: nullable(renderIdentifierType),
      expectedContentGeneration: nullable(safeIntegerType),
      callerAttemptId: nullable(stringType),
    },
  ),

  buildSemanticIndex: command("build_semantic_index", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    chunkSize: usizeType,
    chunkOverlap: usizeType,
    embedModel: stringType,
    requestId: stringType,
  }),
  semanticSearch: command("semantic_search", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    query: stringType,
    topK: usizeType,
    embedModel: stringType,
    expectedChunkSize: usizeType,
    expectedChunkOverlap: usizeType,
    requestId: stringType,
  }),
  queryDocumentRag: command("query_document_rag", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    prompt: stringType,
    history: arrayOf(chatMessage),
    llmModel: stringType,
    embedModel: stringType,
    expectedChunkSize: usizeType,
    expectedChunkOverlap: usizeType,
    requestId: stringType,
  }),
  summarizeSelectionOrPage: command("summarize_selection_or_page", {
    textContent: stringType,
    targetLength: stringType,
    llmModel: stringType,
    requestId: stringType,
    callerAttemptId: stringType,
  }),

  inspectPluginPackage: command("inspect_plugin_package", {
    packagePath: stringType,
  }),
  installPluginPackage: command("install_plugin_package", {
    packagePath: stringType,
    expectedDigest: stringType,
  }),
  grantPluginConsent: command("grant_plugin_consent", {
    pluginId: stringType,
    expectedPackageDigest: stringType,
    expectedPermissions: arrayOf(stringType),
  }),
  uninstallPlugin: command("uninstall_plugin", {
    pluginId: stringType,
  }),
  setPluginEnabled: command("set_plugin_enabled", {
    pluginId: stringType,
    enabled: booleanType,
  }),
  listInstalledPlugins: commandWithoutPayload("list_installed_plugins"),
  startPluginExecution: command("start_plugin_execution", {
    pluginId: stringType,
    hookName: stringType,
    payloadJson: stringType,
    executionId: stringType,
  }),
  cancelPluginExecution: command("cancel_plugin_execution", {
    executionId: stringType,
  }),
  getPluginExecutionStatus: command("get_plugin_execution_status", {
    executionId: stringType,
  }),
  addBatchJob: command("add_batch_job", {
    jobType: stringType,
    filePath: stringType,
    parameters: stringType,
  }),

  applyRedactions: command("apply_redactions", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    outputPath: stringType,
    redactions: arrayOf(redactionArea),
  }),
  inspectDocumentSanitization: command("inspect_document_sanitization", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    policySchemaVersion: stringType,
  }),
  executeDocumentSanitization: command("execute_document_sanitization", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    policy: sanitizationPolicyWire,
    targetPath: stringType,
  }),
  scanSensitiveData: command("scan_sensitive_data", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    patterns: arrayOf(stringType),
  }),
  planSecureRedaction: command("plan_secure_redaction", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    policy: secureRedactionPolicyWire,
    acknowledgements: redactionAcknowledgementsWire,
    regions: arrayOf(redactionRegionInputWire),
  }),
  executeSecureRedaction: command("execute_secure_redaction", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    policy: secureRedactionPolicyWire,
    acknowledgements: redactionAcknowledgementsWire,
    regions: arrayOf(redactionRegionInputWire),
    expectedPlanDigest: stringType,
    targetPath: stringType,
  }),

  speakText: command(
    "speak_text",
    {
      text: stringType,
    },
    {
      voice: nullable(stringType),
    },
  ),
  stopSpeaking: commandWithoutPayload("stop_speaking"),
  shareFileMacos: command("share_file_macos", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  shareExternalFileMacos: command("share_external_file_macos", {
    path: stringType,
  }),
  performOcrOnPage: command("perform_ocr_on_page", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
  }),
});

export const PROTECTED_CONTRACT_IDS = Object.freeze([
  "burnAnnotationsToPdf",
  "insertBlankPage",
  "getFormFields",
  "updateFormField",
  "exportAsSearchablePdf",
  "getPageAnnotations",
  "addAnnotation",
  "deleteAnnotation",
  "savePdfWithAnnotations",
  "applyRedactions",
  "planSecureRedaction",
  "executeSecureRedaction",
  "inspectDocumentSanitization",
  "executeDocumentSanitization",
  "scanSensitiveData",
  "inspectPluginPackage",
  "installPluginPackage",
  "grantPluginConsent",
  "uninstallPlugin",
  "setPluginEnabled",
  "listInstalledPlugins",
  "startPluginExecution",
  "cancelPluginExecution",
  "getPluginExecutionStatus",
  "addBatchJob",
  "speakText",
  "stopSpeaking",
  "shareFileMacos",
  "shareExternalFileMacos",
  "performOcrOnPage",
  "mergeSessionPdfs",
  "splitSessionPdf",
]);

const contractEntries = Object.entries(CONTRACTS);

export function getContract(contractId) {
  return CONTRACTS[contractId] ?? null;
}

export function getContractByCommand(commandName) {
  return (
    contractEntries.find(([, contract]) => contract.command === commandName) ??
    null
  );
}
