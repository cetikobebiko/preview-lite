import { describe, expect, it } from "vitest";
import { normalizeAccessibilityAuditReport, normalizeAccessibilityStructureInspection } from "./accessibilitySchemas.js";

const audit = () => ({
  schemaVersion: "accessibility-audit/2", auditVersion: "basic-accessibility-audit/1",
  referenceProfile: "pdf_ua1_reference", summaryStatus: "findings_detected",
  documentFacts: {
    headerVersion: "1.7", catalogVersion: null, effectiveVersion: "1.7",
    pdfuaDeclaration: { part: null, revision: null, state: "missing" }, pageCount: 1,
    encrypted: false, hasStructureTree: false, hasMarkInfo: false, hasMarkedEntry: false, marked: false, displayDocumentTitle: false,
    documentLanguage: "missing", xmpTitle: "missing", infoTitle: "missing",
  },
  checks: [{ id: "tagging.structure-tree-root", title: "Structure", category: "structure", result: "fail", severity: "critical", automation: "machine", evidence: [], standardReferences: [], conformanceClaim: false, notes: [] }],
  coverage: { implementedCheckIds: ["tagging.structure-tree-root"], notImplementedCheckIds: [], incompleteCheckIds: [], machineCheckCount: 1, humanReviewCount: 0, categories: [], limitations: [] },
  humanReview: [], warnings: [], truncated: false,
});

describe("accessibility report trust boundary", () => {
  it("accepts the pinned audit and structure schemas", () => {
    expect(normalizeAccessibilityAuditReport(audit()).schemaVersion).toBe("accessibility-audit/2");
    expect(normalizeAccessibilityStructureInspection({ schemaVersion: "accessibility-structure/1", inspectionVersion: "bounded-structure-inspection/1", root: null, findings: [], warnings: [], truncated: false, limitations: [] }).root).toBeNull();
  });

  it("rejects unknown fields, conformance claims, and unknown check ids", () => {
    expect(() => normalizeAccessibilityAuditReport({ ...audit(), score: 100 })).toThrow(/unknown field/);
    const claimed = audit(); claimed.checks[0].conformanceClaim = true;
    expect(() => normalizeAccessibilityAuditReport(claimed)).toThrow(/must be false/);
    const unknown = audit(); unknown.coverage.incompleteCheckIds = ["unknown"];
    expect(() => normalizeAccessibilityAuditReport(unknown)).toThrow(/unknown check id/);
  });

  it("rejects unknown schema versions and malformed mandatory report enums", () => {
    const unknownSchema = audit(); unknownSchema.schemaVersion = "accessibility-audit/999";
    expect(() => normalizeAccessibilityAuditReport(unknownSchema)).toThrow(/unknown schema version/);
    const badSummary = audit(); badSummary.summaryStatus = "passed";
    expect(() => normalizeAccessibilityAuditReport(badSummary)).toThrow(/unknown enum value/);
    const badResult = audit(); badResult.checks[0].result = "success";
    expect(() => normalizeAccessibilityAuditReport(badResult)).toThrow(/unknown enum value/);
    const missingCoverage = audit(); delete missingCoverage.coverage;
    expect(() => normalizeAccessibilityAuditReport(missingCoverage)).toThrow(/missing field/);
    const malformedCoverage = audit(); malformedCoverage.coverage = [];
    expect(() => normalizeAccessibilityAuditReport(malformedCoverage)).toThrow(/expected object/);
  });

  it("requires the three distinct MarkInfo facts", () => {
    for (const field of ["hasMarkInfo", "hasMarkedEntry", "marked"]) {
      const value = audit(); delete value.documentFacts[field];
      expect(() => normalizeAccessibilityAuditReport(value)).toThrow(/missing field/);
    }
    const value = audit(); value.documentFacts.hasMarkInfo = "true";
    expect(() => normalizeAccessibilityAuditReport(value)).toThrow(/expected boolean/);
  });

  it("rejects malformed structures and unknown structure enums", () => {
    const base = { schemaVersion: "accessibility-structure/1", inspectionVersion: "bounded-structure-inspection/1", root: null, findings: [], warnings: [], truncated: false, limitations: [] };
    expect(() => normalizeAccessibilityStructureInspection({ ...base, schemaVersion: "accessibility-structure/2" })).toThrow(/unknown schema version/);
    expect(() => normalizeAccessibilityStructureInspection({ ...base, root: [] })).toThrow(/expected object/);
    const root = { id: "node", kind: "mystery", rawType: null, mappedType: null, objectId: null, pageIndex: null, mcid: null, altTextState: "missing", altTextPreview: null, actualTextState: "missing", actualTextPreview: null, languageState: "missing", languagePreview: null, associationStatus: "unresolved", children: [], findings: [], truncated: false };
    expect(() => normalizeAccessibilityStructureInspection({ ...base, root })).toThrow(/unknown enum value/);
  });

  it("rejects structures deeper than the frontend budget", () => {
    let root = { id: "leaf", kind: "structure_element", rawType: null, mappedType: null, objectId: null, pageIndex: null, mcid: null, altTextState: "missing", altTextPreview: null, actualTextState: "missing", actualTextPreview: null, languageState: "missing", languagePreview: null, associationStatus: "not_applicable", children: [], findings: [], truncated: false };
    for (let i = 0; i < 130; i += 1) root = { ...root, id: `node-${i}`, children: [root] };
    expect(() => normalizeAccessibilityStructureInspection({ schemaVersion: "accessibility-structure/1", inspectionVersion: "bounded-structure-inspection/1", root, findings: [], warnings: [], truncated: false, limitations: [] })).toThrow(/structure budget/);
  });
});
