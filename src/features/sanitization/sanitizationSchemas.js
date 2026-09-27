import controlManifest from "./sanitization-controls-v1.json";

export const SANITIZATION_REPORT_SCHEMA_VERSION = "1.1.0";
export const SANITIZATION_INSPECTION_VERSION = "sanitization-inspection-v1";
export const SANITIZATION_POLICY_SCHEMA_VERSION = "sanitization-policy-v1";
export const SANITIZATION_CONTROL_REGISTRY_VERSION = "sanitization-controls/1";
export const SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION = "sanitization-execution/1";
export const SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION = "sanitization-transform/1";
export const SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION = "sanitization-verification/1";
export const SANITIZATION_VERIFICATION_VERSION = "sanitization-verifier-v1";
export const SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION = "sanitization-publication/1";

export const SANITIZATION_PROFILES = Object.freeze(["verified_limited_v1", "custom_supported_v1"]);
export const SANITIZATION_ASSURANCES = Object.freeze(["verified_limited", "verified_custom"]);
export const SANITIZATION_ACKNOWLEDGEMENT_FIELDS = Object.freeze([
  "signatureInvalidationAccepted",
  "visualChangeAccepted",
  "formFunctionLossAccepted",
  "encryptionChangeAccepted",
  "standardConformanceLossAccepted",
]);

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
export const SANITIZATION_SUPPORTED_CONTROL_IDS = Object.freeze(
  SANITIZATION_CONTROL_DESCRIPTORS
    .filter((control) => ["supported", "supported_with_visual_change", "supported_with_functional_change"]
      .includes(control.futureRemovalSupport))
    .map((control) => control.controlId),
);
export const SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS = SANITIZATION_SUPPORTED_CONTROL_IDS;

const descriptorById = new Map(
  SANITIZATION_CONTROL_DESCRIPTORS.map((control) => [control.controlId, control]),
);
const inspectionSupport = new Set([
  "supported_for_inspection", "experimental", "unsupported", "not_applicable",
]);
const removalSupport = new Set([
  "supported", "supported_with_visual_change", "supported_with_functional_change",
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
    "schemaVersion", "inspectionVersion", "policySchemaVersion", "controlRegistryVersion", "inputDigest", "documentFacts",
    "controls", "blockingFindings", "warnings", "limits",
  ]);
  if (report.schemaVersion !== SANITIZATION_REPORT_SCHEMA_VERSION) fail("report.schemaVersion", "unknown schema version");
  if (report.inspectionVersion !== SANITIZATION_INSPECTION_VERSION) fail("report.inspectionVersion", "unknown inspection version");
  if (report.policySchemaVersion !== SANITIZATION_POLICY_SCHEMA_VERSION) fail("report.policySchemaVersion", "unknown policy schema version");
  if (report.controlRegistryVersion !== SANITIZATION_CONTROL_REGISTRY_VERSION) fail("report.controlRegistryVersion", "unknown control registry version");
  if (typeof report.inputDigest !== "string" || !/^[a-f0-9]{64}$/.test(report.inputDigest)) fail("report.inputDigest", "invalid SHA-256 digest");
  if (!Array.isArray(report.controls) || report.controls.length !== SANITIZATION_LIMITS.maxControls) fail("report.controls", "incomplete control set");
  if (!Array.isArray(report.blockingFindings) || report.blockingFindings.length > SANITIZATION_LIMITS.maxBlockingFindings) fail("report.blockingFindings", "invalid collection");
  if (!Array.isArray(report.warnings) || report.warnings.length > SANITIZATION_LIMITS.maxWarnings) fail("report.warnings", "invalid collection");
  const controls = report.controls.map(validateControl);
  const ids = new Set(controls.map((control) => control.controlId));
  if (ids.size !== controls.length || SANITIZATION_CONTROL_IDS.some((id) => !ids.has(id))) fail("report.controls", "duplicate or missing control");
  const blockingFindings = report.blockingFindings.map(validateBlockingFinding);
  const blockingIds = new Set(blockingFindings.map((finding) => finding.controlId));
  if (blockingIds.size !== blockingFindings.length) fail("report.blockingFindings", "duplicate blocking control");
  for (const control of controls) {
    const descriptor = descriptorById.get(control.controlId);
    if ((descriptor.blockingForMinimumProfile && control.present) !== blockingIds.has(control.controlId)) fail("report.blockingFindings", "blocking/control parity failed");
  }
  for (const finding of blockingFindings) {
    const control = controls.find((item) => item.controlId === finding.controlId);
    if (!control?.present || control.findingCount === 0 || finding.evidenceCount > control.findingCount) {
      fail("report.blockingFindings", "blocking evidence/control invariant failed");
    }
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
    controlRegistryVersion: report.controlRegistryVersion,
    inputDigest: report.inputDigest,
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

const sha256 = (value, path) => {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/.test(value)) fail(path, "invalid SHA-256 digest");
  return value;
};

const safeArray = (value, path, limit) => {
  if (!Array.isArray(value) || value.length > limit) fail(path, "invalid collection");
  return value;
};

const normalizeExecutionWarning = (raw, path) => {
  const value = object(raw, path, ["code", "safeMessage"]);
  return Object.freeze({
    code: string(value.code, `${path}.code`),
    safeMessage: string(value.safeMessage, `${path}.safeMessage`),
  });
};

function uniqueKnownControls(raw, path, { supportedOnly = false } = {}) {
  const controls = safeArray(raw, path, SANITIZATION_LIMITS.maxControls);
  if (controls.length === 0) fail(path, "at least one control is required");
  const seen = new Set();
  return controls.map((control, index) => {
    const id = string(control, `${path}[${index}]`);
    if (!descriptorById.has(id)) fail(`${path}[${index}]`, "unknown control ID");
    if (supportedOnly && !SANITIZATION_SUPPORTED_CONTROL_IDS.includes(id)) fail(`${path}[${index}]`, "unsupported control");
    if (seen.has(id)) fail(`${path}[${index}]`, "duplicate control");
    seen.add(id);
    return id;
  });
}

export function createSanitizationAcknowledgements(value = {}) {
  const result = {};
  for (const field of SANITIZATION_ACKNOWLEDGEMENT_FIELDS) result[field] = value[field] === true;
  return Object.freeze(result);
}

export function requiredSanitizationAcknowledgements(inspection) {
  if (!inspection) return Object.freeze([]);
  const signatureBlocker = inspection.blockingFindings
    .some((finding) => finding.controlId === "digital_signatures");
  return Object.freeze(
    inspection.documentFacts.signed || signatureBlocker ? ["signatureInvalidationAccepted"] : [],
  );
}

export function blockingSanitizationFindings(inspection) {
  if (!inspection) return Object.freeze([]);
  return Object.freeze(
    inspection.blockingFindings.filter((finding) => finding.controlId !== "digital_signatures"),
  );
}

export function normalizeSanitizationPolicy({ profile, controls, acknowledgements, inspection }) {
  if (!SANITIZATION_PROFILES.includes(profile)) fail("policy.profile", "unknown profile");
  if (!inspection || inspection.inspectionVersion !== SANITIZATION_INSPECTION_VERSION
    || inspection.controlRegistryVersion !== SANITIZATION_CONTROL_REGISTRY_VERSION
    || inspection.policySchemaVersion !== SANITIZATION_POLICY_SCHEMA_VERSION) {
    fail("policy.inspection", "stale inspection authority");
  }
  const normalizedControls = uniqueKnownControls(controls, "policy.controls", { supportedOnly: true });
  const controlSet = new Set(normalizedControls);
  if (!controlSet.has("incremental_revisions")) fail("policy.controls", "full rewrite control is mandatory");
  if (controlSet.has("object_metadata") && (!controlSet.has("xmp_metadata") || !controlSet.has("page_metadata"))) {
    fail("policy.controls", "object metadata requires XMP and page metadata controls");
  }
  if (profile === "verified_limited_v1") {
    const minimum = new Set(SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS);
    if (controlSet.size !== minimum.size || [...minimum].some((id) => !controlSet.has(id))) {
      fail("policy.controls", "verified limited profile requires the canonical minimum control set");
    }
  }
  if (blockingSanitizationFindings(inspection).length > 0) {
    fail("policy.inspection", "blocking unsupported content is present");
  }
  const accepted = createSanitizationAcknowledgements(acknowledgements);
  for (const field of requiredSanitizationAcknowledgements(inspection)) {
    if (!accepted[field]) fail(`policy.acknowledgements.${field}`, "required acknowledgement is missing");
  }
  return Object.freeze({
    schemaVersion: SANITIZATION_POLICY_SCHEMA_VERSION,
    profile,
    controls: Object.freeze([...normalizedControls]),
    acknowledgements: Object.freeze({
      acknowledgeSignatureInvalidation: accepted.signatureInvalidationAccepted,
    }),
    expectedInspectionVersion: inspection.inspectionVersion,
  });
}

function normalizeTransformControl(raw, index) {
  const path = `report.transform.controls[${index}]`;
  const value = object(raw, path, [
    "controlId", "requested", "detectedBefore", "removedCount", "transformStatus", "evidence", "warnings",
  ]);
  if (!SANITIZATION_SUPPORTED_CONTROL_IDS.includes(value.controlId)) fail(`${path}.controlId`, "unsupported control");
  if (value.requested !== true) fail(`${path}.requested`, "must be true");
  bool(value.detectedBefore, `${path}.detectedBefore`);
  integer(value.removedCount, `${path}.removedCount`, SANITIZATION_LIMITS.maxFindings);
  if (!["removed_candidate", "not_found"].includes(value.transformStatus)) fail(`${path}.transformStatus`, "unknown status");
  if ((value.removedCount > 0) !== (value.transformStatus === "removed_candidate")
    || value.detectedBefore !== (value.removedCount > 0)) fail(path, "status invariant failed");
  const evidence = safeArray(value.evidence, `${path}.evidence`, SANITIZATION_LIMITS.maxEvidencePerControl)
    .map((rawEvidence, evidenceIndex) => {
      const evidencePath = `${path}.evidence[${evidenceIndex}]`;
      const entry = object(rawEvidence, evidencePath, ["logicalPath", "objectId", "payloadSha256"]);
      string(entry.logicalPath, `${evidencePath}.logicalPath`);
      nullableString(entry.objectId, `${evidencePath}.objectId`);
      if (entry.payloadSha256 !== null) sha256(entry.payloadSha256, `${evidencePath}.payloadSha256`);
      return Object.freeze({ ...entry });
    });
  const warnings = safeArray(value.warnings, `${path}.warnings`, SANITIZATION_LIMITS.maxWarnings)
    .map((item, warningIndex) => normalizeExecutionWarning(item, `${path}.warnings[${warningIndex}]`));
  return Object.freeze({ ...value, evidence: Object.freeze(evidence), warnings: Object.freeze(warnings) });
}

function normalizeTransform(raw, policyControls, profile) {
  const value = object(raw, "report.transform", [
    "schemaVersion", "policySchemaVersion", "controlRegistryVersion", "profile", "inputDigest", "outputDigest",
    "outputSizeBytes", "outputRelativeName", "inspectionVersion", "inspectionDigest", "contentGeneration", "controls",
    "expectedDocumentInvariants", "fullRewriteEvidence", "warnings", "signaturesInvalidated", "certificationLost", "status",
  ]);
  if (value.schemaVersion !== SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION
    || value.policySchemaVersion !== SANITIZATION_POLICY_SCHEMA_VERSION
    || value.controlRegistryVersion !== SANITIZATION_CONTROL_REGISTRY_VERSION
    || value.inspectionVersion !== SANITIZATION_INSPECTION_VERSION || value.profile !== profile
    || value.outputRelativeName !== "sanitized-transform.pdf" || value.status !== "transformed_unverified") {
    fail("report.transform", "version or authority invariant failed");
  }
  sha256(value.inputDigest, "report.transform.inputDigest");
  sha256(value.outputDigest, "report.transform.outputDigest");
  sha256(value.inspectionDigest, "report.transform.inspectionDigest");
  integer(value.outputSizeBytes, "report.transform.outputSizeBytes", 512 * 1024 * 1024);
  if (value.outputSizeBytes === 0) fail("report.transform.outputSizeBytes", "must be positive");
  integer(value.contentGeneration, "report.transform.contentGeneration");
  bool(value.signaturesInvalidated, "report.transform.signaturesInvalidated");
  bool(value.certificationLost, "report.transform.certificationLost");
  if (value.certificationLost && !value.signaturesInvalidated) fail("report.transform.certificationLost", "signature invariant failed");
  const controls = safeArray(value.controls, "report.transform.controls", SANITIZATION_LIMITS.maxControls)
    .map(normalizeTransformControl);
  const ids = controls.map((control) => control.controlId);
  if (new Set(ids).size !== ids.length || ids.length !== policyControls.length
    || policyControls.some((id) => !ids.includes(id))) fail("report.transform.controls", "request correlation failed");
  const invariants = object(value.expectedDocumentInvariants, "report.transform.expectedDocumentInvariants", [
    "pageCount", "pages", "namedDestinationCount", "localGotoActionCount", "formFieldCount", "outlineRootPresent",
  ]);
  integer(invariants.pageCount, "report.transform.expectedDocumentInvariants.pageCount", 10_000);
  if (invariants.pageCount === 0) fail("report.transform.expectedDocumentInvariants.pageCount", "must be positive");
  for (const key of ["namedDestinationCount", "localGotoActionCount", "formFieldCount"]) {
    integer(invariants[key], `report.transform.expectedDocumentInvariants.${key}`);
  }
  bool(invariants.outlineRootPresent, "report.transform.expectedDocumentInvariants.outlineRootPresent");
  const pages = safeArray(invariants.pages, "report.transform.expectedDocumentInvariants.pages", 10_000);
  if (pages.length !== invariants.pageCount) fail("report.transform.expectedDocumentInvariants.pages", "page count mismatch");
  pages.forEach((rawPage, index) => {
    const path = `report.transform.expectedDocumentInvariants.pages[${index}]`;
    const page = object(rawPage, path, ["pageIndex", "widthPoints", "heightPoints", "rotationDegrees", "contentSha256"]);
    if (page.pageIndex !== index || !Number.isFinite(page.widthPoints) || page.widthPoints <= 0
      || !Number.isFinite(page.heightPoints) || page.heightPoints <= 0
      || ![0, 90, 180, 270].includes(page.rotationDegrees)) fail(path, "invalid page invariant");
    sha256(page.contentSha256, `${path}.contentSha256`);
  });
  const rewrite = object(value.fullRewriteEvidence, "report.transform.fullRewriteEvidence", [
    "sourceBytesCopied", "incrementalSaveUsed", "activePrevPresent", "activeXrefStmPresent",
    "parserReopenPassed", "unreachableObjectsPruned", "selfCheckPassed",
  ]);
  for (const key of ["sourceBytesCopied", "incrementalSaveUsed", "activePrevPresent", "activeXrefStmPresent", "parserReopenPassed", "selfCheckPassed"]) {
    bool(rewrite[key], `report.transform.fullRewriteEvidence.${key}`);
  }
  integer(rewrite.unreachableObjectsPruned, "report.transform.fullRewriteEvidence.unreachableObjectsPruned");
  if (rewrite.sourceBytesCopied || rewrite.incrementalSaveUsed || rewrite.activePrevPresent
    || rewrite.activeXrefStmPresent || !rewrite.parserReopenPassed || !rewrite.selfCheckPassed) {
    fail("report.transform.fullRewriteEvidence", "full rewrite invariant failed");
  }
  safeArray(value.warnings, "report.transform.warnings", SANITIZATION_LIMITS.maxWarnings)
    .forEach((item, index) => normalizeExecutionWarning(item, `report.transform.warnings[${index}]`));
  return Object.freeze({ ...value, controls: Object.freeze(controls) });
}

const VERIFICATION_CHECKS = Object.freeze([
  "fullRewrite", "incrementalHistoryRemoved", "objectGraphScan", "removedPayloadScan", "parserReopen",
  "pdfiumReopen", "pageTreeValid", "pageCountValid", "pageDimensionsValid", "preservationChecks",
  "renderSmoke", "artifactUnchanged",
]);

function normalizeVerificationCheck(raw, path) {
  const value = object(raw, path, ["status", "message"]);
  if (!["passed", "failed", "skipped_with_reason"].includes(value.status)) fail(`${path}.status`, "invalid verification result");
  if (value.status === "passed" && value.message !== null) fail(`${path}.message`, "passed check cannot carry a message");
  if (value.status !== "passed") string(value.message, `${path}.message`);
  return Object.freeze({ ...value });
}

function normalizeVerification(raw, transform, policyControls, profile) {
  const value = object(raw, "report.verification", [
    "schemaVersion", "policySchemaVersion", "controlRegistryVersion", "verificationVersion", "inspectionVersion",
    "inputDigest", "transformDigest", "profile", "controls", ...VERIFICATION_CHECKS,
    "unresolvedFindings", "blockers", "warnings", "isVerified",
  ]);
  if (value.schemaVersion !== SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION
    || value.policySchemaVersion !== SANITIZATION_POLICY_SCHEMA_VERSION
    || value.controlRegistryVersion !== SANITIZATION_CONTROL_REGISTRY_VERSION
    || value.verificationVersion !== SANITIZATION_VERIFICATION_VERSION
    || value.inspectionVersion !== SANITIZATION_INSPECTION_VERSION || value.profile !== profile
    || value.inputDigest !== transform.inputDigest || value.transformDigest !== transform.outputDigest) {
    fail("report.verification", "version or digest authority invariant failed");
  }
  const checks = VERIFICATION_CHECKS.map((key) => normalizeVerificationCheck(value[key], `report.verification.${key}`));
  const controls = safeArray(value.controls, "report.verification.controls", SANITIZATION_LIMITS.maxControls)
    .map((rawControl, index) => {
      const path = `report.verification.controls[${index}]`;
      const control = object(rawControl, path, [
        "controlId", "requested", "detectedBefore", "forbiddenFindingsAfter", "payloadFingerprintsChecked",
        "payloadFingerprintsRemaining", "status", "warnings",
      ]);
      if (!policyControls.includes(control.controlId) || control.requested !== true) fail(path, "request correlation failed");
      bool(control.detectedBefore, `${path}.detectedBefore`);
      for (const key of ["forbiddenFindingsAfter", "payloadFingerprintsChecked", "payloadFingerprintsRemaining"]) {
        integer(control[key], `${path}.${key}`, SANITIZATION_LIMITS.maxFindings);
      }
      if (!["passed", "failed", "skipped_with_reason"].includes(control.status)) fail(`${path}.status`, "invalid verification result");
      if (control.status === "passed" && (control.forbiddenFindingsAfter !== 0 || control.payloadFingerprintsRemaining !== 0)) {
        fail(path, "passed control invariant failed");
      }
      safeArray(control.warnings, `${path}.warnings`, SANITIZATION_LIMITS.maxWarnings)
        .forEach((item, warningIndex) => normalizeExecutionWarning(item, `${path}.warnings[${warningIndex}]`));
      return Object.freeze({ ...control });
    });
  const ids = controls.map((control) => control.controlId);
  if (new Set(ids).size !== ids.length || ids.length !== policyControls.length) {
    fail("report.verification.controls", "duplicate or missing control");
  }
  const unresolved = safeArray(value.unresolvedFindings, "report.verification.unresolvedFindings", SANITIZATION_LIMITS.maxFindings);
  unresolved.forEach((rawFinding, index) => {
    const path = `report.verification.unresolvedFindings[${index}]`;
    const finding = object(rawFinding, path, ["code", "controlId", "objectId", "logicalPath", "mandatory"]);
    string(finding.code, `${path}.code`);
    nullableString(finding.controlId, `${path}.controlId`);
    if (finding.controlId !== null && !descriptorById.has(finding.controlId)) fail(`${path}.controlId`, "unknown control");
    nullableString(finding.objectId, `${path}.objectId`);
    nullableString(finding.logicalPath, `${path}.logicalPath`);
    bool(finding.mandatory, `${path}.mandatory`);
  });
  const blockers = safeArray(value.blockers, "report.verification.blockers", SANITIZATION_LIMITS.maxBlockingFindings);
  const warnings = safeArray(value.warnings, "report.verification.warnings", SANITIZATION_LIMITS.maxWarnings);
  blockers.forEach((item, index) => normalizeExecutionWarning(item, `report.verification.blockers[${index}]`));
  warnings.forEach((item, index) => normalizeExecutionWarning(item, `report.verification.warnings[${index}]`));
  bool(value.isVerified, "report.verification.isVerified");
  const computedVerified = checks.every((check) => check.status === "passed") && blockers.length === 0
    && !unresolved.some((finding) => finding.mandatory)
    && controls.every((control) => control.status === "passed");
  if (value.isVerified !== computedVerified) fail("report.verification.isVerified", "mandatory verification invariant failed");
  return Object.freeze({ ...value, controls: Object.freeze(controls) });
}

export function normalizeSanitizationExecutionReport(raw) {
  let encoded;
  try { encoded = encoder.encode(JSON.stringify(raw)); } catch (_) { fail("report", "not serializable"); }
  if (encoded.length > SANITIZATION_LIMITS.maxReportBytes) fail("report", "report byte limit exceeded");
  const report = object(raw, "report", [
    "schemaVersion", "policySchemaVersion", "profile", "assurance", "transform", "verification",
    "publication", "controls", "warnings",
  ]);
  if (report.schemaVersion !== SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION) fail("report.schemaVersion", "unknown schema version");
  if (report.policySchemaVersion !== SANITIZATION_POLICY_SCHEMA_VERSION) fail("report.policySchemaVersion", "unknown policy schema version");
  if (!SANITIZATION_PROFILES.includes(report.profile)) fail("report.profile", "unknown profile");
  if (!SANITIZATION_ASSURANCES.includes(report.assurance)) fail("report.assurance", "unknown assurance");
  const controls = uniqueKnownControls(report.controls, "report.controls", { supportedOnly: true });
  const transform = normalizeTransform(report.transform, controls, report.profile);
  const verification = normalizeVerification(report.verification, transform, controls, report.profile);
  const publication = object(report.publication, "report.publication", [
    "schemaVersion", "published", "targetPath", "publishedDigest", "commitState", "warnings",
  ]);
  if (publication.schemaVersion !== SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION) fail("report.publication.schemaVersion", "unknown schema version");
  if (publication.published !== true) fail("report.publication.published", "verified execution must be published");
  string(publication.targetPath, "report.publication.targetPath");
  if (encoder.encode(publication.targetPath).length > 4096 || publication.targetPath.includes("\0")) {
    fail("report.publication.targetPath", "invalid target path");
  }
  sha256(publication.publishedDigest, "report.publication.publishedDigest");
  if (!["new_committed", "new_committed_durability_unconfirmed"].includes(publication.commitState)) {
    fail("report.publication.commitState", "malformed commit state");
  }
  safeArray(publication.warnings, "report.publication.warnings", SANITIZATION_LIMITS.maxWarnings)
    .forEach((item, index) => normalizeExecutionWarning(item, `report.publication.warnings[${index}]`));
  if (!verification.isVerified || publication.publishedDigest !== transform.outputDigest
    || publication.publishedDigest !== verification.transformDigest) {
    fail("report.publication", "published digest invariant failed");
  }
  const expectedAssurance = report.profile === "verified_limited_v1" ? "verified_limited" : "verified_custom";
  if (report.assurance !== expectedAssurance) fail("report.assurance", "profile assurance mismatch");
  const warnings = safeArray(report.warnings, "report.warnings", SANITIZATION_LIMITS.maxWarnings)
    .map((item, index) => normalizeExecutionWarning(item, `report.warnings[${index}]`));
  return Object.freeze({
    schemaVersion: report.schemaVersion,
    policySchemaVersion: report.policySchemaVersion,
    profile: report.profile,
    assurance: report.assurance,
    controls: Object.freeze([...controls]),
    transform,
    verification,
    publication: Object.freeze({ ...publication }),
    warnings: Object.freeze(warnings),
  });
}

export const validateSanitizationExecutionReport = (raw) => {
  normalizeSanitizationExecutionReport(raw);
  return true;
};
