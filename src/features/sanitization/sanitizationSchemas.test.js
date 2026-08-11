import { describe, expect, it } from "vitest";
import {
  SANITIZATION_CONTROL_DESCRIPTORS,
  SANITIZATION_INSPECTION_VERSION,
  SANITIZATION_POLICY_SCHEMA_VERSION,
  SANITIZATION_REPORT_SCHEMA_VERSION,
  normalizeSanitizationInspectionReport,
} from "./sanitizationSchemas.js";

const control = (descriptor, overrides = {}) => ({
  controlId: descriptor.controlId,
  category: descriptor.category,
  inspectionSupport: descriptor.inspectionSupport,
  futureRemovalSupport: descriptor.futureRemovalSupport,
  present: false,
  findingCount: 0,
  expectedVisualImpact: descriptor.expectedVisualImpact,
  expectedFunctionalImpact: descriptor.expectedFunctionalImpact,
  blockingReason: null,
  representativeEvidence: [],
  ...overrides,
});

const report = (overrides = {}) => ({
  schemaVersion: SANITIZATION_REPORT_SCHEMA_VERSION,
  inspectionVersion: SANITIZATION_INSPECTION_VERSION,
  policySchemaVersion: SANITIZATION_POLICY_SCHEMA_VERSION,
  documentFacts: {
    encrypted: false, signed: false, certificationSignatureDetected: false,
    fieldMdpDetected: false, incrementalRevisionsDetected: false, xfaDetected: false,
    optionalContentDetected: false, richMediaDetected: false, acroFormDetected: false,
    calculationOrderDetected: false, documentIdentifierPresent: false,
    objectStreamsDetected: false, xrefStreamsDetected: false, pageCount: 1,
  },
  controls: SANITIZATION_CONTROL_DESCRIPTORS.map((descriptor) => control(descriptor)),
  blockingFindings: [],
  warnings: [],
  limits: { visitedObjects: 10, decodedBytes: 0, findings: 0, truncated: false },
  ...overrides,
});

describe("sanitization inspection schema", () => {
  it("accepts a strict valid report and retains detection/removal separation", () => {
    const value = normalizeSanitizationInspectionReport(report());
    const xfa = value.controls.find((item) => item.controlId === "xfa");
    expect(xfa.inspectionSupport).toBe("supported_for_inspection");
    expect(xfa.futureRemovalSupport).toBe("unsupported");
    expect(Object.isFrozen(value)).toBe(true);
  });

  it.each([
    ["schemaVersion", "future"],
    ["inspectionVersion", "future"],
    ["policySchemaVersion", "future"],
  ])("rejects unknown %s", (field, value) => {
    expect(() => normalizeSanitizationInspectionReport(report({ [field]: value }))).toThrow(/unknown/);
  });

  it("rejects duplicate and unknown controls", () => {
    const duplicate = report();
    duplicate.controls[1] = { ...duplicate.controls[0] };
    expect(() => normalizeSanitizationInspectionReport(duplicate)).toThrow(/duplicate|missing/);
    const unknown = report();
    unknown.controls[0] = { ...unknown.controls[0], controlId: "unknown_control" };
    expect(() => normalizeSanitizationInspectionReport(unknown)).toThrow(/unknown control/);
  });

  it("rejects support drift, malformed/non-finite counts, and contradictory presence", () => {
    const support = report();
    support.controls[0] = { ...support.controls[0], inspectionSupport: "unknown" };
    expect(() => normalizeSanitizationInspectionReport(support)).toThrow(/support/);
    const nan = report();
    nan.controls[0] = { ...nan.controls[0], findingCount: Number.NaN };
    expect(() => normalizeSanitizationInspectionReport(nan)).toThrow(/integer/);
    const contradictory = report();
    contradictory.controls[0] = { ...contradictory.controls[0], present: true };
    expect(() => normalizeSanitizationInspectionReport(contradictory)).toThrow(/invariant/);
  });

  it("rejects malformed blockers, oversized collections, unknown fields and partial reports", () => {
    const blocker = report({ blockingFindings: [{ code: "X", controlId: "document_info", category: "metadata", safeMessage: "x", evidenceCount: 1 }] });
    expect(() => normalizeSanitizationInspectionReport(blocker)).toThrow(/blocking control/);
    const warnings = report({ warnings: Array.from({ length: 257 }, () => ({ code: "W", safeMessage: "warning" })) });
    expect(() => normalizeSanitizationInspectionReport(warnings)).toThrow(/collection/);
    const unknown = report({ payload: "forbidden" });
    expect(() => normalizeSanitizationInspectionReport(unknown)).toThrow(/unknown field/);
    const partial = report({ limits: { visitedObjects: 10, decodedBytes: 0, findings: 0, truncated: true } });
    expect(() => normalizeSanitizationInspectionReport(partial)).toThrow(/partial/);
  });
});

export { report as makeSanitizationReport };
