import { callBackend } from "./client.js";
import { runSessionJob } from "./sessionJob.js";
import {
  normalizeSecureRedactionPolicy,
  normalizeRedactionAcknowledgements,
  normalizeRedactionRegionInput,
  validateRedactionRegionInputs,
  validateSha256Digest,
  normalizeAndVerifySecureRedactionPlan,
  normalizeSecureRedactionExecutionReport,
} from "../../features/redaction/redactionSchemas.js";

const ALLOWED_REPORT_KEYS = new Set([
  "schema_version",
  "items",
  "warnings",
  "truncated",
]);

const ALLOWED_ITEM_KEYS = new Set([
  "page_index",
  "pattern_index",
  "text_preview",
  "geometry_status",
  "page_geometry",
  "source_rects_pdf_points",
  "warnings",
]);

const ALLOWED_GEOMETRY_KEYS = new Set([
  "coordinate_space",
  "unit",
  "origin",
  "media_box",
  "crop_box",
  "effective_box",
  "intrinsic_rotation",
]);

const ALLOWED_BOX_KEYS = new Set(["left", "bottom", "width", "height"]);
const ALLOWED_RECT_KEYS = new Set(["x", "y", "width", "height"]);
const ALLOWED_WARNING_KEYS = new Set(["code", "message"]);
const SUPPORTED_ROTATIONS = new Set([0, 90, 180, 270]);

export const validateSensitiveScanReport = (data) => {
  if (!data || typeof data !== "object" || Array.isArray(data)) {
    throw new Error("Invalid sensitive scan report: must be an object");
  }

  for (const key of Object.keys(data)) {
    if (!ALLOWED_REPORT_KEYS.has(key)) {
      throw new Error(`Invalid sensitive scan report: unknown field '${key}'`);
    }
  }

  if (data.schema_version !== 2) {
    throw new Error(
      `Invalid sensitive scan report: schema_version must be 2, got ${data.schema_version}`,
    );
  }

  if (typeof data.truncated !== "boolean") {
    throw new Error("Invalid sensitive scan report: truncated must be a boolean");
  }

  if (!Array.isArray(data.warnings)) {
    throw new Error("Invalid sensitive scan report: warnings must be an array");
  }

  const seenWarningCodes = new Set();
  for (const warning of data.warnings) {
    validateWarningObject(warning, "report warning");
    if (seenWarningCodes.has(warning.code)) {
      throw new Error(`Invalid sensitive scan report: duplicate report warning code '${warning.code}'`);
    }
    seenWarningCodes.add(warning.code);
  }

  if (!Array.isArray(data.items)) {
    throw new Error("Invalid sensitive scan report: items must be an array");
  }

  if (data.items.length > 10000) {
    throw new Error("Invalid sensitive scan report: items length cannot exceed 10000");
  }

  for (const item of data.items) {
    validateReportItem(item);
  }

  return true;
};

function validateWarningObject(warning, context) {
  if (!warning || typeof warning !== "object" || Array.isArray(warning)) {
    throw new Error(`Invalid sensitive scan report: ${context} must be an object`);
  }
  for (const key of Object.keys(warning)) {
    if (!ALLOWED_WARNING_KEYS.has(key)) {
      throw new Error(`Invalid sensitive scan report: unknown field '${key}' in ${context}`);
    }
  }
  if (typeof warning.code !== "string" || !warning.code.trim()) {
    throw new Error(`Invalid sensitive scan report: ${context} code must be a non-empty string`);
  }
  if (typeof warning.message !== "string" || !warning.message.trim()) {
    throw new Error(`Invalid sensitive scan report: ${context} message must be a non-empty string`);
  }
}

function validatePdfBox(box, name) {
  if (!box || typeof box !== "object" || Array.isArray(box)) {
    throw new Error(`Invalid sensitive scan report: ${name} must be an object`);
  }
  for (const key of Object.keys(box)) {
    if (!ALLOWED_BOX_KEYS.has(key)) {
      throw new Error(`Invalid sensitive scan report: unknown field '${key}' in ${name}`);
    }
  }
  for (const field of ["left", "bottom", "width", "height"]) {
    if (typeof box[field] !== "number" || !Number.isFinite(box[field])) {
      throw new Error(`Invalid sensitive scan report: ${name} ${field} must be a finite number`);
    }
  }
  if (box.width <= 0 || box.height <= 0) {
    throw new Error(`Invalid sensitive scan report: ${name} width and height must be positive`);
  }
}

function validatePageGeometry(geom) {
  if (!geom || typeof geom !== "object" || Array.isArray(geom)) {
    throw new Error("Invalid sensitive scan report: page_geometry must be an object");
  }
  for (const key of Object.keys(geom)) {
    if (!ALLOWED_GEOMETRY_KEYS.has(key)) {
      throw new Error(`Invalid sensitive scan report: unknown field '${key}' in page_geometry`);
    }
  }
  if (geom.coordinate_space !== "pdf_user_space") {
    throw new Error("Invalid sensitive scan report: page_geometry coordinate_space must be 'pdf_user_space'");
  }
  if (geom.unit !== "pdf_point") {
    throw new Error("Invalid sensitive scan report: page_geometry unit must be 'pdf_point'");
  }
  if (geom.origin !== "bottom_left") {
    throw new Error("Invalid sensitive scan report: page_geometry origin must be 'bottom_left'");
  }
  validatePdfBox(geom.media_box, "media_box");
  if (geom.crop_box !== null && geom.crop_box !== undefined) {
    validatePdfBox(geom.crop_box, "crop_box");
  }
  validatePdfBox(geom.effective_box, "effective_box");
  if (
    typeof geom.intrinsic_rotation !== "number" ||
    !SUPPORTED_ROTATIONS.has(geom.intrinsic_rotation)
  ) {
    throw new Error(
      "Invalid sensitive scan report: page_geometry intrinsic_rotation must be numeric 0, 90, 180, or 270",
    );
  }
}

function validateReportItem(item) {
  if (!item || typeof item !== "object" || Array.isArray(item)) {
    throw new Error("Invalid sensitive scan report: item must be an object");
  }

  for (const key of Object.keys(item)) {
    if (!ALLOWED_ITEM_KEYS.has(key)) {
      throw new Error(`Invalid sensitive scan report: unknown field '${key}' in item`);
    }
  }

  if (
    !Number.isInteger(item.page_index) ||
    item.page_index < 0 ||
    item.page_index >= 10000 ||
    !Number.isSafeInteger(item.page_index)
  ) {
    throw new Error("Invalid sensitive scan report: page_index must be a non-negative integer < 10000");
  }

  if (
    !Number.isInteger(item.pattern_index) ||
    item.pattern_index < 0 ||
    item.pattern_index >= 64 ||
    !Number.isSafeInteger(item.pattern_index)
  ) {
    throw new Error("Invalid sensitive scan report: pattern_index must be a non-negative integer < 64");
  }

  if (typeof item.text_preview !== "string" || Array.from(item.text_preview).length > 256) {
    throw new Error("Invalid sensitive scan report: text_preview must be a string up to 256 characters");
  }

  if (item.geometry_status !== "resolved" && item.geometry_status !== "unresolved") {
    throw new Error("Invalid sensitive scan report: geometry_status must be 'resolved' or 'unresolved'");
  }

  if (!Array.isArray(item.source_rects_pdf_points)) {
    throw new Error("Invalid sensitive scan report: source_rects_pdf_points must be an array");
  }

  if (item.source_rects_pdf_points.length > 64) {
    throw new Error("Invalid sensitive scan report: source_rects_pdf_points length cannot exceed 64");
  }

  if (item.geometry_status === "resolved") {
    if (!item.page_geometry) {
      throw new Error("Invalid sensitive scan report: resolved item must have page_geometry");
    }
    if (item.source_rects_pdf_points.length === 0) {
      throw new Error("Invalid sensitive scan report: resolved item must have source_rects_pdf_points");
    }
  }

  if (item.geometry_status === "unresolved" && item.source_rects_pdf_points.length !== 0) {
    throw new Error("Invalid sensitive scan report: unresolved item must have empty source_rects_pdf_points");
  }

  if (item.page_geometry !== null && item.page_geometry !== undefined) {
    validatePageGeometry(item.page_geometry);
  }

  for (const rect of item.source_rects_pdf_points) {
    validatePdfRect(rect);
  }

  if (!Array.isArray(item.warnings)) {
    throw new Error("Invalid sensitive scan report: item warnings must be an array");
  }

  for (const warning of item.warnings) {
    validateWarningObject(warning, "item warning");
  }
}

function validatePdfRect(rect) {
  if (!rect || typeof rect !== "object" || Array.isArray(rect)) {
    throw new Error("Invalid sensitive scan report: rectangle must be an object");
  }
  for (const key of Object.keys(rect)) {
    if (!ALLOWED_RECT_KEYS.has(key)) {
      throw new Error(`Invalid sensitive scan report: unknown field '${key}' in rectangle`);
    }
  }
  for (const field of ["x", "y", "width", "height"]) {
    if (typeof rect[field] !== "number" || !Number.isFinite(rect[field])) {
      throw new Error(`Invalid sensitive scan report: rectangle ${field} must be a finite number`);
    }
  }
  if (rect.width <= 0 || rect.height <= 0) {
    throw new Error("Invalid sensitive scan report: rectangle width and height must be positive");
  }
}

export const normalizeSensitiveScanReport = (data) => {
  validateSensitiveScanReport(data);

  return {
    schemaVersion: data.schema_version,
    items: data.items.map((item) => ({
      pageIndex: item.page_index,
      patternIndex: item.pattern_index,
      textPreview: item.text_preview,
      geometryStatus: item.geometry_status,
      pageGeometry: item.page_geometry
        ? {
            coordinateSpace: item.page_geometry.coordinate_space,
            unit: item.page_geometry.unit,
            origin: item.page_geometry.origin,
            mediaBox: { ...item.page_geometry.media_box },
            cropBox: item.page_geometry.crop_box ? { ...item.page_geometry.crop_box } : null,
            effectiveBox: { ...item.page_geometry.effective_box },
            intrinsicRotation: item.page_geometry.intrinsic_rotation,
          }
        : null,
      sourceRectsPdfPoints: item.source_rects_pdf_points.map((r) => ({
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
      })),
      warnings: item.warnings.map((w) => ({
        code: w.code,
        message: w.message,
      })),
    })),
    warnings: data.warnings.map((w) => ({
      code: w.code,
      message: w.message,
    })),
    truncated: data.truncated,
  };
};

export const applyRedactions = (sessionId, expectedContentGeneration, outputPath, redactions) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("applyRedactions", { jobId, sessionId, expectedContentGeneration, outputPath, redactions }),
  ).promise;

export const sanitizePdfDocument = (sessionId, expectedContentGeneration, outputPath, options) =>
  runSessionJob(sessionId, (jobId) =>
    callBackend("sanitizePdfDocument", { jobId, sessionId, expectedContentGeneration, outputPath, options }),
  ).promise;

export const startSensitiveDataScan = (sessionId, expectedContentGeneration, patterns) => {
  const handle = runSessionJob(sessionId, (jobId) =>
    callBackend("scanSensitiveData", { jobId, sessionId, expectedContentGeneration, patterns }),
  );
  return {
    jobId: handle.jobId,
    promise: handle.promise.then(normalizeSensitiveScanReport),
    cancel: handle.cancel,
  };
};

export const scanSensitiveData = (sessionId, expectedContentGeneration, patterns) =>
  startSensitiveDataScan(sessionId, expectedContentGeneration, patterns).promise;

export function toSecureRedactionPolicyWire(policy) {
  const norm = normalizeSecureRedactionPolicy(policy);
  return {
    schema_version: norm.schemaVersion,
    profile: norm.profile,
    strategy: norm.strategy,
    raster_dpi: norm.rasterDpi,
    appearance: {
      kind: norm.appearance.kind,
      color: norm.appearance.color,
      opacity: norm.appearance.opacity,
    },
    safety_margin_points: norm.safetyMarginPoints,
    interactive_content_policy: norm.interactiveContentPolicy,
    signature_policy: norm.signaturePolicy,
    encryption_policy: norm.encryptionPolicy,
  };
}

export function toRedactionAcknowledgementsWire(acknowledgements) {
  const norm = normalizeRedactionAcknowledgements(acknowledgements);
  return {
    schema_version: norm.schemaVersion,
    rasterized_pages_accepted: norm.rasterizedPagesAccepted,
    text_search_loss_accepted: norm.textSearchLossAccepted,
    accessibility_loss_accepted: norm.accessibilityLossAccepted,
    annotation_loss_accepted: norm.annotationLossAccepted,
    form_loss_accepted: norm.formLossAccepted,
    signature_invalidation_accepted: norm.signatureInvalidationAccepted,
    encryption_change_accepted: norm.encryptionChangeAccepted,
  };
}

export function toRedactionRegionInputWire(region) {
  const norm = normalizeRedactionRegionInput(region);
  return {
    id: norm.id,
    page_index: norm.pageIndex,
    normalized_top_left_rect: {
      x: norm.normalizedTopLeftRect.x,
      y: norm.normalizedTopLeftRect.y,
      width: norm.normalizedTopLeftRect.width,
      height: norm.normalizedTopLeftRect.height,
    },
    source: norm.source,
  };
}

export const startSecureRedactionPlan = (
  sessionId,
  expectedContentGeneration,
  policy,
  acknowledgements,
  regions,
) => {
  const wirePolicy = toSecureRedactionPolicyWire(policy);
  const wireAcks = toRedactionAcknowledgementsWire(acknowledgements);
  const canonicalRegions = validateRedactionRegionInputs(regions);
  const wireRegions = canonicalRegions.map(toRedactionRegionInputWire);

  const handle = runSessionJob(sessionId, (jobId) =>
    callBackend("planSecureRedaction", {
      jobId,
      sessionId,
      expectedContentGeneration,
      policy: wirePolicy,
      acknowledgements: wireAcks,
      regions: wireRegions,
    }),
  );

  return {
    jobId: handle.jobId,
    promise: handle.promise.then(async (rawPlan) => {
      const verifiedPlan = await normalizeAndVerifySecureRedactionPlan(rawPlan);
      if (!verifiedPlan || verifiedPlan.digestVerified !== true) {
        throw new Error("Plan verification failed: digestVerified must be true");
      }
      return verifiedPlan;
    }),
    cancel: handle.cancel,
  };
};

/**
 * Starts the secure export pipeline for the canonical secureRedactionController.
 * Target filesystem authority remains exclusively in Rust.
 */
export const startSecureRedactionExecution = (
  sessionId,
  expectedContentGeneration,
  policy,
  acknowledgements,
  regions,
  expectedPlanDigest,
  targetPath,
) => {
  const wirePolicy = toSecureRedactionPolicyWire(policy);
  const wireAcks = toRedactionAcknowledgementsWire(acknowledgements);
  const canonicalRegions = validateRedactionRegionInputs(regions);
  const wireRegions = canonicalRegions.map(toRedactionRegionInputWire);
  validateSha256Digest(expectedPlanDigest, "expectedPlanDigest");
  if (typeof targetPath !== "string" || targetPath.trim().length === 0 || targetPath.length > 4096) {
    throw new Error("targetPath must be a non-empty contract-level path string up to 4096 characters");
  }

  const handle = runSessionJob(sessionId, (jobId) =>
    callBackend("executeSecureRedaction", {
      jobId,
      sessionId,
      expectedContentGeneration,
      policy: wirePolicy,
      acknowledgements: wireAcks,
      regions: wireRegions,
      expectedPlanDigest,
      targetPath,
    }),
  );
  return {
    jobId: handle.jobId,
    promise: handle.promise.then(normalizeSecureRedactionExecutionReport),
    cancel: handle.cancel,
  };
};
