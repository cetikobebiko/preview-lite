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

const scanPatternKindType = Object.freeze({
  type: "string",
  enum: Object.freeze(["regex", "builtin"]),
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

// Sayfanın görüntü uzayında (page /Rotate ve CropBox uygulanmış, viewer rotation
// kaldırılmış) sol-üst kökenli normalize dikdörtgen. PDF user space'e dönüşüm job
// worker'da PDFium ile yapılır; bkz. `NormalizedPageRect` (src-tauri/src/annotations.rs).
const NORMALIZED_RECT_EPSILON = 1e-4;
const unitIntervalType = Object.freeze({ type: "number", min: 0, max: 1 });
const unitExtentType = Object.freeze({
  type: "number",
  min: Number.MIN_VALUE,
  max: 1,
});
const normalizedPageRect = objectOf(
  {
    x: unitIntervalType,
    y: unitIntervalType,
    width: unitExtentType,
    height: unitExtentType,
  },
  {},
  (rect) =>
    rect.x + rect.width > 1 + NORMALIZED_RECT_EPSILON ||
    rect.y + rect.height > 1 + NORMALIZED_RECT_EPSILON
      ? "normalized rect must stay inside the unit page square"
      : null,
);

const redactionArea = objectOf({
  page_index: u32Type,
  x: numberType,
  y: numberType,
  width: numberType,
  height: numberType,
});

// print_imposition::PrintLayoutRequest: paper by print_layout::PAPERS name, mode tagged by kind.
const enumOf = (...values) => Object.freeze({ type: "string", enum: Object.freeze(values) });
const printLayoutWire = objectOf({
  paper: stringType,
  orientation: enumOf("auto", "portrait", "landscape"),
  mode: objectOf(
    { kind: enumOf("fit", "actual", "shrink", "scale", "multiple", "booklet", "poster") },
    {
      percent: numberType,
      perSheet: u32Type,
      order: enumOf("horizontal", "horizontal_reversed", "vertical", "vertical_reversed", "cut_stack"),
      border: booleanType,
      sides: enumOf("both", "front", "back"),
      rightBinding: booleanType,
      overlapPoints: numberType,
      cutMarks: booleanType,
    },
  ),
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

// Form taslağı yazımı (Faz 3); bkz. `FormFieldEdit` (src-tauri/src/forms/fill.rs).
// Kurallar backend'de de uygulanır; burada yalnız biçim denetlenir.
const FORM_TEXT_MAX_LENGTH = 32768;
const formFieldEdit = objectOf(
  {
    pageIndex: u32Type,
    annotIndex: u32Type,
    fieldName: Object.freeze({ type: "string", minLength: 1, maxLength: FORM_TEXT_MAX_LENGTH }),
    kind: Object.freeze({ type: "string", enum: Object.freeze(["text", "checkbox", "radio", "choice"]) }),
  },
  {
    text: Object.freeze({ type: "string", maxLength: FORM_TEXT_MAX_LENGTH }),
    checked: booleanType,
    selected: Object.freeze({ type: "array", items: u32Type }),
  },
  (edit) => {
    const present = ["text", "checked", "selected"].filter((key) => edit[key] !== undefined);
    const expected = { text: ["text"], checkbox: ["checked"], radio: [], choice: ["selected"] }[edit.kind];
    return present.length === expected.length && present.every((key, index) => key === expected[index])
      ? null
      : `form edit of kind ${edit.kind} must carry exactly: ${expected.join(", ") || "no value"}`;
  },
);

// Form alanı tasarımı (form-authoring-plan.md); bkz. `FormAuthorOp`
// (src-tauri/src/forms/author.rs). Kurallar backend'de de uygulanır.
const formLabelType = Object.freeze({ type: "string", minLength: 1, maxLength: 1024 });
const formOptionsType = Object.freeze({ type: "array", items: formLabelType });
const newFormField = objectOf(
  { kind: Object.freeze({ type: "string", enum: Object.freeze(["text", "checkbox", "radio", "combo", "list"]) }) },
  {
    multiline: booleanType,
    group: formLabelType,
    export: formLabelType,
    options: formOptionsType,
    editable: booleanType,
    multi: booleanType,
  },
  (field) => {
    const allowed = {
      text: ["multiline"],
      checkbox: [],
      radio: ["group", "export"],
      combo: ["options", "editable"],
      list: ["options", "multi"],
    }[field.kind];
    const present = ["multiline", "group", "export", "options", "editable", "multi"].filter(
      (key) => field[key] !== undefined,
    );
    return present.length === allowed.length && allowed.every((key) => present.includes(key))
      ? null
      : `form field of kind ${field.kind} must carry exactly: ${allowed.join(", ") || "nothing else"}`;
  },
);
const formFieldPropsPatch = objectOf(
  {},
  {
    name: formLabelType,
    tooltip: Object.freeze({ type: "string", maxLength: 1024 }),
    required: booleanType,
    readOnly: booleanType,
    multiline: booleanType,
    maxLen: u32Type,
    options: formOptionsType,
  },
);
const formAuthorOp = objectOf(
  {
    op: Object.freeze({ type: "string", enum: Object.freeze(["add", "delete", "move", "set_props"]) }),
    pageIndex: u32Type,
  },
  {
    normalizedRect: normalizedPageRect,
    field: newFormField,
    name: Object.freeze({ type: "string", minLength: 1, maxLength: 255, nullable: true }),
    annotIndex: u32Type,
    fieldName: formLabelType,
    props: formFieldPropsPatch,
  },
  (op) => {
    const required = {
      add: ["normalizedRect", "field"],
      delete: ["annotIndex", "fieldName"],
      move: ["annotIndex", "fieldName", "normalizedRect"],
      set_props: ["annotIndex", "fieldName", "props"],
    }[op.op];
    const optional = op.op === "add" ? ["name"] : [];
    const present = ["normalizedRect", "field", "name", "annotIndex", "fieldName", "props"].filter(
      (key) => op[key] !== undefined,
    );
    const ok =
      required.every((key) => present.includes(key)) &&
      present.every((key) => required.includes(key) || optional.includes(key));
    return ok ? null : `form op ${op.op} must carry exactly: ${required.join(", ")}`;
  },
);

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
    // false: an existing file with a planned name fails the job with
    // SESSION_JOB_OUTPUT_CONFLICT and `conflictingNames`; nothing is written.
    overwrite: booleanType,
  }),
  exportSessionTextContent: command("export_session_text_content", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pages: arrayOf(u32Type),
    format: stringType,
  }),
  // Rust writes the file through document_io::write_atomically; the target must be .txt or .md.
  exportSessionTextToFile: command("export_session_text_to_file", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pages: arrayOf(u32Type),
    format: stringType,
    targetPath: stringType,
  }),
  // PdfCraft's writers in the job worker; the target is published atomically.
  exportSessionDocument: command("export_session_document", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pages: arrayOf(u32Type),
    format: Object.freeze({ type: "string", enum: Object.freeze(["docx", "pptx", "html", "rtf"]) }),
    targetPath: stringType,
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
      layout: nullable(printLayoutWire),
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
  undoSessionMutation: command("undo_session_mutation", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  redoSessionMutation: command("redo_session_mutation", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
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
  getSessionFormInfo: command("get_session_form_info", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
  }),
  getSessionPageFormFields: command("get_session_page_form_fields", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    pageIndex: u32Type,
  }),
  authorSessionFormFields: command("author_session_form_fields", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    ops: Object.freeze({ type: "array", items: formAuthorOp }),
  }),
  fillSessionFormFields: command("fill_session_form_fields", {
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    edits: Object.freeze({ type: "array", items: formFieldEdit }),
  }),
  addSessionAnnotation: command(
    "add_session_annotation",
    {
      sessionId: renderIdentifierType,
      expectedContentGeneration: safeIntegerType,
      pageIndex: u32Type,
      annotType: stringType,
      normalizedRect: normalizedPageRect,
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
  // Read-only: Rust copies otherPath into the job directory and compares text in the job worker.
  compareSessionPdf: command("compare_session_pdf", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    otherPath: stringType,
    currentIsOld: booleanType,
  }),
  splitSessionPdf: command("split_session_pdf", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    ranges: arrayOf(tupleOf(u32Type, u32Type)),
    outputDir: stringType,
    // See exportSessionPagesAsImages.
    overwrite: booleanType,
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
  // A pattern is a regex, or a built-in Rust matcher by id (redaction::patterns: card numbers
  // with a Luhn check, e-mail, phone, date, US SSN).
  scanSensitiveData: command("scan_sensitive_data", {
    jobId: renderIdentifierType,
    sessionId: renderIdentifierType,
    expectedContentGeneration: safeIntegerType,
    patterns: arrayOf(objectOf({ kind: scanPatternKindType, value: stringType })),
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
  // app_config.json in AppData; Rust validates a JSON object of at most 1 MiB and writes atomically.
  readAppConfig: commandWithoutPayload("read_app_config"),
  writeAppConfig: command("write_app_config", {
    contents: stringType,
  }),
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
  "compareSessionPdf",
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
