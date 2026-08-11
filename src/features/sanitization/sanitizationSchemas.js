import controlManifest from "./sanitization-controls-v1.json";

export const SANITIZATION_REPORT_SCHEMA_VERSION = "1.0.0";
export const SANITIZATION_INSPECTION_VERSION = "sanitization-inspection-v1";
export const SANITIZATION_POLICY_SCHEMA_VERSION = "sanitization-policy-v1";

export const SANITIZATION_LIMITS = Object.freeze({
  maxControls: controlManifest.controls.length,
  maxFindings: 10_000,
  maxEvidencePerControl: 8,
  maxBlockingFindings: 128,
  maxWarnings: 256,
  maxReportBytes: 1024 * 1024,
  maxSafeStringBytes: 512,
  maxVisitedObjects: 100_000,
  maxDecodedBytes: 128 * 1024 * 1024,
});

export const SANITIZATION_CONTROL_DESCRIPTORS = Object.freeze(
  controlManifest.controls.map((control) => Object.freeze({ ...control })),
);
export const SANITIZATION_CONTROL_IDS = Object.freeze(
  SANITIZATION_CONTROL_DESCRIPTORS.map((control) => control.controlId),
);

const descriptorById = new Map(
  SANITIZATION_CONTROL_DESCRIPTORS.map((control) => [control.controlId, control]),
);
const inspectionSupport = new Set([
  "supported_for_inspection", "experimental", "unsupported", "not_applicable",
]);
const removalSupport = new Set([
  "candidate", "candidate_with_visual_change", "candidate_with_functional_change",
  "experimental", "unsupported", "unverified", "not_applicable",
]);
const encoder = new TextEncoder();

function fail(path, message) {
  throw new TypeError(`${path}: ${message}`);
}

function object(value, path, keys) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(path, "expected object");
  const allowed = new Set(keys);
  for (const key of Object.keys(value)) if (!allowed.has(key)) fail(`${path}.${key}`, "unknown field");
  for (const key of keys) if (!Object.prototype.hasOwnProperty.call(value, key)) fail(`${path}.${key}`, "missing field");
  return value;
}

function string(value, path, { nullable = false } = {}) {
  if (value === null && nullable) return null;
  if (typeof value !== "string" || value.length === 0 || encoder.encode(value).length > SANITIZATION_LIMITS.maxSafeStringBytes) {
    fail(path, "expected bounded non-empty string");
  }
  return value;
}

function bool(value, path) {
  if (typeof value !== "boolean") fail(path, "expected boolean");
  return value;
}

function integer(value, path, max = Number.MAX_SAFE_INTEGER) {
  if (!Number.isSafeInteger(value) || value < 0 || value > max) fail(path, "expected bounded non-negative safe integer");
  return value;
}

function nullableString(value, path) {
  return value === null ? null : string(value, path);
}

function validateDocumentFacts(raw) {
  const keys = [
    "encrypted", "signed", "certificationSignatureDetected", "fieldMdpDetected",
    "incrementalRevisionsDetected", "xfaDetected", "optionalContentDetected",
    "richMediaDetected", "acroFormDetected", "calculationOrderDetected",
    "documentIdentifierPresent", "objectStreamsDetected", "xrefStreamsDetected", "pageCount",
  ];
  const value = object(raw, "report.documentFacts", keys);
  for (const key of keys.slice(0, -1)) bool(value[key], `report.documentFacts.${key}`);
  integer(value.pageCount, "report.documentFacts.pageCount", 4_294_967_295);
  return { ...value };
}

function validateEvidence(raw, path) {
  const value = object(raw, path, ["category", "objectId", "logicalPath", "eventOrActionType"]);
  string(value.category, `${path}.category`);
  nullableString(value.objectId, `${path}.objectId`);
  string(value.logicalPath, `${path}.logicalPath`);
  nullableString(value.eventOrActionType, `${path}.eventOrActionType`);
  return { ...value };
}

function validateControl(raw, index) {
  const path = `report.controls[${index}]`;
  const value = object(raw, path, [
    "controlId", "category", "inspectionSupport", "futureRemovalSupport", "present",
    "findingCount", "expectedVisualImpact", "expectedFunctionalImpact", "blockingReason",
    "representativeEvidence",
  ]);
  const id = string(value.controlId, `${path}.controlId`);
  const descriptor = descriptorById.get(id);
  if (!descriptor) fail(`${path}.controlId`, "unknown control ID");
  string(value.category, `${path}.category`);
  if (!inspectionSupport.has(value.inspectionSupport)) fail(`${path}.inspectionSupport`, "unknown inspection support");
  if (!removalSupport.has(value.futureRemovalSupport)) fail(`${path}.futureRemovalSupport`, "unknown future removal support");
  bool(value.present, `${path}.present`);
  integer(value.findingCount, `${path}.findingCount`, SANITIZATION_LIMITS.maxFindings);
  string(value.expectedVisualImpact, `${path}.expectedVisualImpact`);
  string(value.expectedFunctionalImpact, `${path}.expectedFunctionalImpact`);
  nullableString(value.blockingReason, `${path}.blockingReason`);
  if (!Array.isArray(value.representativeEvidence) || value.representativeEvidence.length > SANITIZATION_LIMITS.maxEvidencePerControl) {
    fail(`${path}.representativeEvidence`, "invalid evidence collection");
  }
  if (value.present !== (value.findingCount > 0)) fail(path, "present/findingCount invariant failed");
  const shouldBlock = descriptor.blockingForMinimumProfile && value.present;
  if (shouldBlock !== (value.blockingReason !== null)) fail(path, "blockingReason invariant failed");
  for (const key of ["category", "inspectionSupport", "futureRemovalSupport", "expectedVisualImpact", "expectedFunctionalImpact"]) {
    if (value[key] !== descriptor[key]) fail(`${path}.${key}`, "control manifest parity failed");
  }
  return {
    ...value,
    representativeEvidence: value.representativeEvidence.map((item, evidenceIndex) =>
      validateEvidence(item, `${path}.representativeEvidence[${evidenceIndex}]`)),
  };
}

function validateBlockingFinding(raw, index) {
  const path = `report.blockingFindings[${index}]`;
  const value = object(raw, path, ["code", "controlId", "category", "safeMessage", "evidenceCount"]);
  string(value.code, `${path}.code`);
  const id = string(value.controlId, `${path}.controlId`);
  const descriptor = descriptorById.get(id);
  if (!descriptor?.blockingForMinimumProfile) fail(`${path}.controlId`, "not a blocking control");
  if (value.category !== descriptor.category) fail(`${path}.category`, "manifest parity failed");
  string(value.safeMessage, `${path}.safeMessage`);
  integer(value.evidenceCount, `${path}.evidenceCount`, SANITIZATION_LIMITS.maxFindings);
  if (value.evidenceCount === 0) fail(`${path}.evidenceCount`, "must be positive");
  return { ...value };
}

function validateWarning(raw, index) {
  const path = `report.warnings[${index}]`;
  const value = object(raw, path, ["code", "safeMessage"]);
  string(value.code, `${path}.code`);
  string(value.safeMessage, `${path}.safeMessage`);
  return { ...value };
}

/**
 * Strictly validates and returns a detached canonical inspection report.
 * Sensitive PDF payloads are not part of this schema.
 * @param {unknown} raw
 * @returns {object}
 */
export function normalizeSanitizationInspectionReport(raw) {
  let encoded;
  try { encoded = encoder.encode(JSON.stringify(raw)); } catch (_) { fail("report", "not serializable"); }
  if (encoded.length > SANITIZATION_LIMITS.maxReportBytes) fail("report", "report byte limit exceeded");
  const report = object(raw, "report", [
    "schemaVersion", "inspectionVersion", "policySchemaVersion", "documentFacts",
    "controls", "blockingFindings", "warnings", "limits",
  ]);
  if (report.schemaVersion !== SANITIZATION_REPORT_SCHEMA_VERSION) fail("report.schemaVersion", "unknown schema version");
  if (report.inspectionVersion !== SANITIZATION_INSPECTION_VERSION) fail("report.inspectionVersion", "unknown inspection version");
  if (report.policySchemaVersion !== SANITIZATION_POLICY_SCHEMA_VERSION) fail("report.policySchemaVersion", "unknown policy schema version");
  if (!Array.isArray(report.controls) || report.controls.length !== SANITIZATION_LIMITS.maxControls) fail("report.controls", "incomplete control set");
  if (!Array.isArray(report.blockingFindings) || report.blockingFindings.length > SANITIZATION_LIMITS.maxBlockingFindings) fail("report.blockingFindings", "invalid collection");
  if (!Array.isArray(report.warnings) || report.warnings.length > SANITIZATION_LIMITS.maxWarnings) fail("report.warnings", "invalid collection");
  const controls = report.controls.map(validateControl);
  const ids = new Set(controls.map((control) => control.controlId));
  if (ids.size !== controls.length || SANITIZATION_CONTROL_IDS.some((id) => !ids.has(id))) fail("report.controls", "duplicate or missing control");
  const blockingFindings = report.blockingFindings.map(validateBlockingFinding);
  const blockingIds = new Set(blockingFindings.map((finding) => finding.controlId));
  for (const control of controls) {
    const descriptor = descriptorById.get(control.controlId);
    if ((descriptor.blockingForMinimumProfile && control.present) !== blockingIds.has(control.controlId)) fail("report.blockingFindings", "blocking/control parity failed");
  }
  const limits = object(report.limits, "report.limits", ["visitedObjects", "decodedBytes", "findings", "truncated"]);
  integer(limits.visitedObjects, "report.limits.visitedObjects", SANITIZATION_LIMITS.maxVisitedObjects);
  integer(limits.decodedBytes, "report.limits.decodedBytes", SANITIZATION_LIMITS.maxDecodedBytes);
  integer(limits.findings, "report.limits.findings", SANITIZATION_LIMITS.maxFindings);
  bool(limits.truncated, "report.limits.truncated");
  if (limits.truncated) fail("report.limits.truncated", "partial inspection cannot be complete");
  const findingTotal = controls.reduce((sum, control) => sum + control.findingCount, 0);
  if (findingTotal !== limits.findings) fail("report.limits.findings", "finding total mismatch");
  return Object.freeze({
    schemaVersion: report.schemaVersion,
    inspectionVersion: report.inspectionVersion,
    policySchemaVersion: report.policySchemaVersion,
    documentFacts: Object.freeze(validateDocumentFacts(report.documentFacts)),
    controls: Object.freeze(controls.map(Object.freeze)),
    blockingFindings: Object.freeze(blockingFindings.map(Object.freeze)),
    warnings: Object.freeze(report.warnings.map(validateWarning).map(Object.freeze)),
    limits: Object.freeze({ ...limits }),
  });
}

export const validateSanitizationInspectionReport = (raw) => {
  normalizeSanitizationInspectionReport(raw);
  return true;
};
