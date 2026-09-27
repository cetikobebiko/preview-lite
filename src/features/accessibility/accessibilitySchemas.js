export const ACCESSIBILITY_AUDIT_SCHEMA_VERSION = "accessibility-audit/2";
export const ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION = "accessibility-structure/1";

export const ACCESSIBILITY_LIMITS = Object.freeze({
  reportBytes: 1024 * 1024,
  structureBytes: 4 * 1024 * 1024,
  checks: 128,
  evidencePerCheck: 32,
  humanReview: 128,
  warnings: 256,
  nodes: 10_000,
  depth: 128,
  stringBytes: 2048,
});

const encoder = new TextEncoder();
const profiles = new Set(["pdf_ua1_reference", "pdf_ua2_reference", "undetermined", "unsupported_pdf_version"]);
const summaries = new Set(["findings_detected", "no_findings_in_implemented_checks", "audit_incomplete", "audit_failed"]);
const results = new Set(["pass", "fail", "warning", "not_applicable", "not_checked", "indeterminate", "human_review_required"]);
const severities = new Set(["info", "minor", "major", "critical"]);
const automations = new Set(["machine", "mixed", "human"]);
const textStates = new Set(["missing", "empty", "whitespace_only", "present", "invalid"]);
const nodeKinds = new Set(["document_root", "structure_element", "marked_content_reference", "object_reference", "mcid", "unknown"]);

function fail(path, message) { throw new TypeError(`${path}: ${message}`); }
function object(value, path, keys) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(path, "expected object");
  const allowed = new Set(keys);
  for (const key of Object.keys(value)) if (!allowed.has(key)) fail(`${path}.${key}`, "unknown field");
  for (const key of keys) if (!Object.prototype.hasOwnProperty.call(value, key)) fail(`${path}.${key}`, "missing field");
  return value;
}
function string(value, path, nullable = false) {
  if (nullable && value === null) return null;
  if (typeof value !== "string" || encoder.encode(value).length > ACCESSIBILITY_LIMITS.stringBytes) fail(path, "expected bounded string");
  return value;
}
function bool(value, path) { if (typeof value !== "boolean") fail(path, "expected boolean"); return value; }
function integer(value, path, max = Number.MAX_SAFE_INTEGER) {
  if (!Number.isSafeInteger(value) || value < 0 || value > max) fail(path, "expected bounded non-negative integer");
  return value;
}
function enumValue(value, path, allowed) { if (!allowed.has(value)) fail(path, "unknown enum value"); return value; }
function array(value, path, limit) { if (!Array.isArray(value) || value.length > limit) fail(path, "invalid collection"); return value; }
function jsonBudget(value, path, limit) {
  let bytes;
  try { bytes = encoder.encode(JSON.stringify(value)).length; } catch (_) { fail(path, "not serializable"); }
  if (bytes > limit) fail(path, "serialized byte limit exceeded");
}

function normalizeEvidence(raw, path) {
  const value = object(raw, path, ["objectId", "pageIndex", "structurePath", "field", "message", "valuePreview"]);
  return Object.freeze({
    objectId: string(value.objectId, `${path}.objectId`, true),
    pageIndex: value.pageIndex === null ? null : integer(value.pageIndex, `${path}.pageIndex`, 4_294_967_295),
    structurePath: string(value.structurePath, `${path}.structurePath`, true),
    field: string(value.field, `${path}.field`, true),
    message: string(value.message, `${path}.message`),
    valuePreview: string(value.valuePreview, `${path}.valuePreview`, true),
  });
}

function normalizeReference(raw, path) {
  const value = object(raw, path, ["standard", "clause", "referenceType"]);
  return Object.freeze({ standard: string(value.standard, `${path}.standard`), clause: string(value.clause, `${path}.clause`), referenceType: string(value.referenceType, `${path}.referenceType`) });
}

function normalizeCheck(raw, index) {
  const path = `report.checks[${index}]`;
  const value = object(raw, path, ["id", "title", "category", "result", "severity", "automation", "evidence", "standardReferences", "conformanceClaim", "notes"]);
  if (value.conformanceClaim !== false) fail(`${path}.conformanceClaim`, "must be false");
  return Object.freeze({
    id: string(value.id, `${path}.id`), title: string(value.title, `${path}.title`), category: string(value.category, `${path}.category`),
    result: enumValue(value.result, `${path}.result`, results), severity: enumValue(value.severity, `${path}.severity`, severities),
    automation: enumValue(value.automation, `${path}.automation`, automations),
    evidence: Object.freeze(array(value.evidence, `${path}.evidence`, ACCESSIBILITY_LIMITS.evidencePerCheck).map((item, i) => normalizeEvidence(item, `${path}.evidence[${i}]`))),
    standardReferences: Object.freeze(array(value.standardReferences, `${path}.standardReferences`, 16).map((item, i) => normalizeReference(item, `${path}.standardReferences[${i}]`))),
    conformanceClaim: false,
    notes: Object.freeze(array(value.notes, `${path}.notes`, 32).map((item, i) => string(item, `${path}.notes[${i}]`))),
  });
}

function normalizeFacts(raw) {
  const path = "report.documentFacts";
  const value = object(raw, path, ["headerVersion", "catalogVersion", "effectiveVersion", "pdfuaDeclaration", "pageCount", "encrypted", "hasStructureTree", "hasMarkInfo", "hasMarkedEntry", "marked", "displayDocumentTitle", "documentLanguage", "xmpTitle", "infoTitle"]);
  const declaration = object(value.pdfuaDeclaration, `${path}.pdfuaDeclaration`, ["part", "revision", "state"]);
  return Object.freeze({
    headerVersion: string(value.headerVersion, `${path}.headerVersion`, true), catalogVersion: string(value.catalogVersion, `${path}.catalogVersion`, true),
    effectiveVersion: string(value.effectiveVersion, `${path}.effectiveVersion`, true),
    pdfuaDeclaration: Object.freeze({ part: string(declaration.part, `${path}.pdfuaDeclaration.part`, true), revision: string(declaration.revision, `${path}.pdfuaDeclaration.revision`, true), state: enumValue(declaration.state, `${path}.pdfuaDeclaration.state`, textStates) }),
    pageCount: integer(value.pageCount, `${path}.pageCount`, 4_294_967_295), encrypted: bool(value.encrypted, `${path}.encrypted`),
    hasStructureTree: bool(value.hasStructureTree, `${path}.hasStructureTree`), hasMarkInfo: bool(value.hasMarkInfo, `${path}.hasMarkInfo`),
    hasMarkedEntry: bool(value.hasMarkedEntry, `${path}.hasMarkedEntry`), marked: bool(value.marked, `${path}.marked`),
    displayDocumentTitle: bool(value.displayDocumentTitle, `${path}.displayDocumentTitle`),
    documentLanguage: enumValue(value.documentLanguage, `${path}.documentLanguage`, textStates), xmpTitle: enumValue(value.xmpTitle, `${path}.xmpTitle`, textStates), infoTitle: enumValue(value.infoTitle, `${path}.infoTitle`, textStates),
  });
}

export function normalizeAccessibilityAuditReport(raw) {
  jsonBudget(raw, "report", ACCESSIBILITY_LIMITS.reportBytes);
  const report = object(raw, "report", ["schemaVersion", "auditVersion", "referenceProfile", "summaryStatus", "documentFacts", "checks", "coverage", "humanReview", "warnings", "truncated"]);
  if (report.schemaVersion !== ACCESSIBILITY_AUDIT_SCHEMA_VERSION) fail("report.schemaVersion", "unknown schema version");
  const checks = array(report.checks, "report.checks", ACCESSIBILITY_LIMITS.checks).map(normalizeCheck);
  const ids = new Set(checks.map((item) => item.id));
  if (ids.size !== checks.length) fail("report.checks", "duplicate check id");
  const coverage = object(report.coverage, "report.coverage", ["implementedCheckIds", "notImplementedCheckIds", "incompleteCheckIds", "machineCheckCount", "humanReviewCount", "categories", "limitations"]);
  for (const [key, limit] of [["implementedCheckIds", 128], ["notImplementedCheckIds", 128], ["incompleteCheckIds", 128]]) {
    array(coverage[key], `report.coverage.${key}`, limit).forEach((id, i) => { string(id, `report.coverage.${key}[${i}]`); if (!ids.has(id)) fail(`report.coverage.${key}[${i}]`, "unknown check id"); });
  }
  const categories = array(coverage.categories, "report.coverage.categories", 32).map((rawCategory, index) => {
    const path = `report.coverage.categories[${index}]`;
    const category = object(rawCategory, path, ["category", "status", "limitations"]);
    return Object.freeze({
      category: string(category.category, `${path}.category`),
      status: string(category.status, `${path}.status`),
      limitations: Object.freeze(array(category.limitations, `${path}.limitations`, 32).map((item, i) => string(item, `${path}.limitations[${i}]`))),
    });
  });
  const coverageLimitations = Object.freeze(array(coverage.limitations, "report.coverage.limitations", 64).map((item, i) => string(item, `report.coverage.limitations[${i}]`)));
  integer(coverage.machineCheckCount, "report.coverage.machineCheckCount", 128);
  integer(coverage.humanReviewCount, "report.coverage.humanReviewCount", ACCESSIBILITY_LIMITS.humanReview);
  if (coverage.machineCheckCount !== coverage.implementedCheckIds.length) fail("report.coverage.machineCheckCount", "implemented count mismatch");
  const humanReview = array(report.humanReview, "report.humanReview", ACCESSIBILITY_LIMITS.humanReview).map((rawItem, index) => {
    const path = `report.humanReview[${index}]`;
    const item = object(rawItem, path, ["id", "title", "category", "result", "applies", "notes"]);
    if (item.result !== "human_review_required") fail(`${path}.result`, "must require human review");
    return Object.freeze({ id: string(item.id, `${path}.id`), title: string(item.title, `${path}.title`), category: string(item.category, `${path}.category`), result: item.result, applies: bool(item.applies, `${path}.applies`), notes: Object.freeze(array(item.notes, `${path}.notes`, 32).map((note, i) => string(note, `${path}.notes[${i}]`))) });
  });
  return Object.freeze({
    schemaVersion: report.schemaVersion, auditVersion: string(report.auditVersion, "report.auditVersion"),
    referenceProfile: enumValue(report.referenceProfile, "report.referenceProfile", profiles), summaryStatus: enumValue(report.summaryStatus, "report.summaryStatus", summaries),
    documentFacts: normalizeFacts(report.documentFacts), checks: Object.freeze(checks),
    coverage: Object.freeze({
      implementedCheckIds: Object.freeze([...coverage.implementedCheckIds]),
      notImplementedCheckIds: Object.freeze([...coverage.notImplementedCheckIds]),
      incompleteCheckIds: Object.freeze([...coverage.incompleteCheckIds]),
      machineCheckCount: coverage.machineCheckCount, humanReviewCount: coverage.humanReviewCount,
      categories: Object.freeze(categories), limitations: coverageLimitations,
    }), humanReview: Object.freeze(humanReview),
    warnings: Object.freeze(array(report.warnings, "report.warnings", ACCESSIBILITY_LIMITS.warnings).map((item, i) => string(item, `report.warnings[${i}]`))),
    truncated: bool(report.truncated, "report.truncated"),
  });
}

function normalizeNode(raw, path, depth, state) {
  if (depth > ACCESSIBILITY_LIMITS.depth || ++state.nodes > ACCESSIBILITY_LIMITS.nodes) fail(path, "structure budget exceeded");
  const value = object(raw, path, ["id", "kind", "rawType", "mappedType", "objectId", "pageIndex", "mcid", "altTextState", "altTextPreview", "actualTextState", "actualTextPreview", "languageState", "languagePreview", "associationStatus", "children", "findings", "truncated"]);
  return Object.freeze({
    id: string(value.id, `${path}.id`), kind: enumValue(value.kind, `${path}.kind`, nodeKinds),
    rawType: string(value.rawType, `${path}.rawType`, true), mappedType: string(value.mappedType, `${path}.mappedType`, true), objectId: string(value.objectId, `${path}.objectId`, true),
    pageIndex: value.pageIndex === null ? null : integer(value.pageIndex, `${path}.pageIndex`, 4_294_967_295),
    mcid: value.mcid === null ? null : integer(value.mcid, `${path}.mcid`),
    altTextState: enumValue(value.altTextState, `${path}.altTextState`, textStates), altTextPreview: string(value.altTextPreview, `${path}.altTextPreview`, true),
    actualTextState: enumValue(value.actualTextState, `${path}.actualTextState`, textStates), actualTextPreview: string(value.actualTextPreview, `${path}.actualTextPreview`, true),
    languageState: enumValue(value.languageState, `${path}.languageState`, textStates), languagePreview: string(value.languagePreview, `${path}.languagePreview`, true),
    associationStatus: string(value.associationStatus, `${path}.associationStatus`),
    children: Object.freeze(array(value.children, `${path}.children`, 4096).map((item, i) => normalizeNode(item, `${path}.children[${i}]`, depth + 1, state))),
    findings: Object.freeze(array(value.findings, `${path}.findings`, 32).map((item, i) => string(item, `${path}.findings[${i}]`))),
    truncated: bool(value.truncated, `${path}.truncated`),
  });
}

export function normalizeAccessibilityStructureInspection(raw) {
  jsonBudget(raw, "inspection", ACCESSIBILITY_LIMITS.structureBytes);
  const inspection = object(raw, "inspection", ["schemaVersion", "inspectionVersion", "root", "findings", "warnings", "truncated", "limitations"]);
  if (inspection.schemaVersion !== ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION) fail("inspection.schemaVersion", "unknown schema version");
  return Object.freeze({
    schemaVersion: inspection.schemaVersion, inspectionVersion: string(inspection.inspectionVersion, "inspection.inspectionVersion"),
    root: inspection.root === null ? null : normalizeNode(inspection.root, "inspection.root", 0, { nodes: 0 }),
    findings: Object.freeze(array(inspection.findings, "inspection.findings", 512).map((item, i) => string(item, `inspection.findings[${i}]`))),
    warnings: Object.freeze(array(inspection.warnings, "inspection.warnings", ACCESSIBILITY_LIMITS.warnings).map((item, i) => string(item, `inspection.warnings[${i}]`))),
    truncated: bool(inspection.truncated, "inspection.truncated"),
    limitations: Object.freeze(array(inspection.limitations, "inspection.limitations", 64).map((item, i) => string(item, `inspection.limitations[${i}]`))),
  });
}
