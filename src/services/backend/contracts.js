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

const sanitizationOptions = objectOf({
  remove_metadata: booleanType,
  remove_annotations: booleanType,
  remove_javascript: booleanType,
  remove_embedded_files: booleanType,
  remove_hidden_layers: booleanType,
});

const visualSignatureOptions = objectOf(
  {
    page_index: u32Type,
    x: numberType,
    y: numberType,
    width: numberType,
    height: numberType,
  },
  {
    signature_text: nullable(stringType),
    signature_image_base64: nullable(stringType),
  },
);

const signatureOptions = objectOf(
  {
    reason: stringType,
    location: stringType,
  },
  {
    contact_info: nullable(stringType),
    visual_signature: nullable(visualSignatureOptions),
  },
);

const command = (name, required = {}, optional = {}, validate = null) =>
  Object.freeze({
    command: name,
    payload: objectOf(required, optional, validate),
  });

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
  analyzeSessionColorSpaces: command("analyze_session_color_spaces", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  calculateSessionInkCoverage: command("calculate_session_ink_coverage", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
    maxLimit: numberType,
    scale: numberType,
  }),
  renderSessionSeparationChannels: command("render_session_separation_channels", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
    scale: numberType,
  }),
  exportSessionPdfX: command("export_session_pages_as_pdf_x", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    standard: stringType,
    profileName: stringType,
    targetPath: stringType,
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
  validateSessionPdfUa: command("validate_session_pdf_ua", {
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
  renderSessionPage: command(
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
  ),
  renderSessionThumbnail: command(
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
  ),
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
  updateSessionAltText: command(
    "update_session_alt_text",
    {
      sessionId: renderIdentifierType,
      expectedContentGeneration: safeIntegerType,
      objectId: stringType,
    },
    {
      altText: nullable(stringType),
      language: nullable(stringType),
    },
  ),
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
  readImageFile: command("read_image_file", {
    path: stringType,
    rotation: i32Type,
    flipH: booleanType,
    flipV: booleanType,
  }),
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
  detectScannedPage: command("detect_scanned_page", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
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

  buildSemanticIndex: command("build_semantic_index", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    chunkSize: usizeType,
    chunkOverlap: usizeType,
    ollamaUrl: stringType,
    embedModel: stringType,
  }),
  semanticSearch: command("semantic_search", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    query: stringType,
    topK: usizeType,
    ollamaUrl: stringType,
    embedModel: stringType,
  }),
  queryDocumentRag: command("query_document_rag", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    prompt: stringType,
    history: arrayOf(chatMessage),
    ollamaUrl: stringType,
    llmModel: stringType,
    embedModel: stringType,
  }),
  summarizeSelectionOrPage: command("summarize_selection_or_page", {
    textContent: stringType,
    targetLength: stringType,
    ollamaUrl: stringType,
    llmModel: stringType,
  }),

  getActivePlugins: commandWithoutPayload("get_active_plugins"),
  loadPlugin: command("load_plugin", {
    manifestPath: stringType,
  }),
  unloadPlugin: command("unload_plugin", {
    pluginId: stringType,
  }),
  triggerPluginHook: command("trigger_plugin_hook", {
    pluginId: stringType,
    hookName: stringType,
    payload: stringType,
  }),
  runScriptDirect: command("run_script_direct", {
    scriptContent: stringType,
  }),
  addBatchJob: command("add_batch_job", {
    jobType: stringType,
    filePath: stringType,
    parameters: stringType,
  }),

  listSystemCertificates: commandWithoutPayload("list_system_certificates"),
  signPdfPades: command("sign_pdf_pades", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    outputPath: stringType,
    certId: stringType,
    pin: nullable(stringType),
    options: signatureOptions,
  }),
  verifyPdfSignatures: command("verify_pdf_signatures", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  enableLtv: command("enable_ltv", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    signatureId: stringType,
  }),

  applyRedactions: command("apply_redactions", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    outputPath: stringType,
    redactions: arrayOf(redactionArea),
  }),
  sanitizePdfDocument: command("sanitize_pdf_document", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    outputPath: stringType,
    options: sanitizationOptions,
  }),
  scanSensitiveData: command("scan_sensitive_data", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    patterns: arrayOf(stringType),
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
  "sanitizePdfDocument",
  "scanSensitiveData",
  "listSystemCertificates",
  "signPdfPades",
  "verifyPdfSignatures",
  "enableLtv",
  "exportPdfX",
  "getActivePlugins",
  "loadPlugin",
  "unloadPlugin",
  "triggerPluginHook",
  "runScriptDirect",
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
