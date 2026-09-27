import { describe, expect, it } from "vitest";
import {
  SANITIZATION_CONTROL_DESCRIPTORS,
  SANITIZATION_INSPECTION_VERSION,
  SANITIZATION_POLICY_SCHEMA_VERSION,
  SANITIZATION_CONTROL_REGISTRY_VERSION,
  SANITIZATION_REPORT_SCHEMA_VERSION,
  SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION,
  SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION,
  SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION,
  SANITIZATION_VERIFICATION_VERSION,
  SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION,
  normalizeSanitizationInspectionReport,
  normalizeSanitizationExecutionReport,
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
  controlRegistryVersion: SANITIZATION_CONTROL_REGISTRY_VERSION,
  inputDigest: "a".repeat(64),
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
    ["controlRegistryVersion", "future"],
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

  it("enforces blocking finding identity and evidence cross-field invariants", () => {
    const descriptor = SANITIZATION_CONTROL_DESCRIPTORS.find(
      (item) => item.blockingForMinimumProfile,
    );
    const blockingControl = control(descriptor, {
      present: true,
      findingCount: 3,
      blockingReason: "blocked",
    });
    const blockingFinding = {
      code: "SANITIZATION_TEST_BLOCKER",
      controlId: descriptor.controlId,
      category: descriptor.category,
      safeMessage: "blocked",
      evidenceCount: 1,
    };
    const valid = report({
      controls: SANITIZATION_CONTROL_DESCRIPTORS.map((item) =>
        item.controlId === descriptor.controlId ? blockingControl : control(item)),
      blockingFindings: [blockingFinding],
      limits: { visitedObjects: 10, decodedBytes: 0, findings: 3, truncated: false },
    });
    expect(() => normalizeSanitizationInspectionReport(valid)).not.toThrow();

    const duplicate = structuredClone(valid);
    duplicate.blockingFindings.push({ ...blockingFinding });
    expect(() => normalizeSanitizationInspectionReport(duplicate)).toThrow(/duplicate/);

    const excessiveEvidence = structuredClone(valid);
    excessiveEvidence.controls.find((item) => item.controlId === descriptor.controlId).findingCount = 1;
    excessiveEvidence.limits.findings = 1;
    excessiveEvidence.blockingFindings[0].evidenceCount = 2;
    expect(() => normalizeSanitizationInspectionReport(excessiveEvidence)).toThrow(/evidence\/control/);

    const absent = structuredClone(valid);
    Object.assign(absent.controls.find((item) => item.controlId === descriptor.controlId), {
      present: false,
      findingCount: 0,
      blockingReason: null,
    });
    absent.limits.findings = 0;
    expect(() => normalizeSanitizationInspectionReport(absent)).toThrow(/blocking\/control/);

    for (const [present, findingCount] of [[true, 0], [false, 1]]) {
      const contradiction = report();
      Object.assign(
        contradiction.controls.find((item) => item.controlId === descriptor.controlId),
        { present, findingCount, blockingReason: present ? "blocked" : null },
      );
      contradiction.limits.findings = findingCount;
      if (present) contradiction.blockingFindings = [{ ...blockingFinding }];
      expect(() => normalizeSanitizationInspectionReport(contradiction)).toThrow(/invariant/);
    }
  });
});

const passed = () => ({ status: "passed", message: null });
const executionReport = (overrides = {}) => {
  const controls = overrides.controls ?? ["incremental_revisions"];
  const transformControls = controls.map((controlId) => ({
    controlId, requested: true, detectedBefore: false, removedCount: 0,
    transformStatus: "not_found", evidence: [], warnings: [],
  }));
  const verificationControls = controls.map((controlId) => ({
    controlId, requested: true, detectedBefore: false, forbiddenFindingsAfter: 0,
    payloadFingerprintsChecked: 0, payloadFingerprintsRemaining: 0, status: "passed", warnings: [],
  }));
  return {
    schemaVersion: SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION,
    policySchemaVersion: SANITIZATION_POLICY_SCHEMA_VERSION,
    profile: "custom_supported_v1",
    assurance: "verified_custom",
    transform: {
      schemaVersion: SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION,
      policySchemaVersion: SANITIZATION_POLICY_SCHEMA_VERSION,
      controlRegistryVersion: SANITIZATION_CONTROL_REGISTRY_VERSION,
      profile: "custom_supported_v1",
      inputDigest: "a".repeat(64), outputDigest: "b".repeat(64), outputSizeBytes: 128,
      outputRelativeName: "sanitized-transform.pdf", inspectionVersion: SANITIZATION_INSPECTION_VERSION,
      inspectionDigest: "c".repeat(64), contentGeneration: 1, controls: transformControls,
      expectedDocumentInvariants: {
        pageCount: 1,
        pages: [{ pageIndex: 0, widthPoints: 612, heightPoints: 792, rotationDegrees: 0, contentSha256: "d".repeat(64) }],
        namedDestinationCount: 0, localGotoActionCount: 0, formFieldCount: 0, outlineRootPresent: false,
      },
      fullRewriteEvidence: {
        sourceBytesCopied: false, incrementalSaveUsed: false, activePrevPresent: false,
        activeXrefStmPresent: false, parserReopenPassed: true, unreachableObjectsPruned: 0, selfCheckPassed: true,
      },
      warnings: [], signaturesInvalidated: false, certificationLost: false, status: "transformed_unverified",
    },
    verification: {
      schemaVersion: SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION,
      policySchemaVersion: SANITIZATION_POLICY_SCHEMA_VERSION,
      controlRegistryVersion: SANITIZATION_CONTROL_REGISTRY_VERSION,
      verificationVersion: SANITIZATION_VERIFICATION_VERSION,
      inspectionVersion: SANITIZATION_INSPECTION_VERSION,
      inputDigest: "a".repeat(64), transformDigest: "b".repeat(64), profile: "custom_supported_v1",
      controls: verificationControls,
      fullRewrite: passed(), incrementalHistoryRemoved: passed(), objectGraphScan: passed(),
      removedPayloadScan: passed(), parserReopen: passed(), pdfiumReopen: passed(), pageTreeValid: passed(),
      pageCountValid: passed(), pageDimensionsValid: passed(), preservationChecks: passed(),
      renderSmoke: passed(), artifactUnchanged: passed(), unresolvedFindings: [], blockers: [], warnings: [], isVerified: true,
    },
    publication: {
      schemaVersion: SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION,
      published: true, targetPath: "/exports/result.pdf", publishedDigest: "b".repeat(64),
      commitState: "new_committed", warnings: [],
    },
    controls,
    warnings: [],
    ...overrides,
  };
};

describe("sanitization execution schema", () => {
  it("accepts a strict verified custom report", () => {
    const value = normalizeSanitizationExecutionReport(executionReport());
    expect(value.assurance).toBe("verified_custom");
    expect(value.publication.published).toBe(true);
    expect(Object.isFrozen(value)).toBe(true);
  });

  it.each([
    ["schemaVersion", "future"],
    ["profile", "future_profile"],
    ["assurance", "future_assurance"],
  ])("rejects unknown %s", (field, value) => {
    expect(() => normalizeSanitizationExecutionReport(executionReport({ [field]: value }))).toThrow(/unknown/);
  });

  it("rejects unknown and duplicate controls", () => {
    expect(() => normalizeSanitizationExecutionReport(executionReport({ controls: ["future_control"] }))).toThrow(/unknown/);
    expect(() => normalizeSanitizationExecutionReport(executionReport({ controls: ["incremental_revisions", "incremental_revisions"] }))).toThrow(/duplicate/);
  });

  it("rejects malformed commit state and invalid published invariant", () => {
    const commit = executionReport();
    commit.publication.commitState = "recovery_required";
    expect(() => normalizeSanitizationExecutionReport(commit)).toThrow(/commit state/);
    const unpublished = executionReport();
    unpublished.publication.published = false;
    expect(() => normalizeSanitizationExecutionReport(unpublished)).toThrow(/published/);
  });

  it("rejects invalid verification results and oversized collections", () => {
    const verification = executionReport();
    verification.verification.renderSmoke = { status: "skipped", message: "not run" };
    expect(() => normalizeSanitizationExecutionReport(verification)).toThrow(/verification result/);
    const oversized = executionReport({
      warnings: Array.from({ length: 257 }, (_, index) => ({ code: `W${index}`, safeMessage: "warning" })),
    });
    expect(() => normalizeSanitizationExecutionReport(oversized)).toThrow(/collection/);
  });
});

export { report as makeSanitizationReport };
