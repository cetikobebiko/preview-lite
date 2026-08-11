import { describe, expect, it, vi } from "vitest";
import { SANITIZATION_CONTROL_DESCRIPTORS } from "../../features/sanitization/sanitizationSchemas.js";

const callBackend = vi.hoisted(() => vi.fn());
const cancel = vi.hoisted(() => vi.fn());
const runSessionJob = vi.hoisted(() => vi.fn((_sessionId, invokeOperation) => ({
  jobId: "job-1",
  promise: invokeOperation("job-1"),
  cancel,
})));

vi.mock("./client.js", () => ({ callBackend }));
vi.mock("./sessionJob.js", () => ({ runSessionJob }));

import { startSanitizationInspection } from "./sanitization.js";

const validReport = {
  schemaVersion: "1.0.0",
  inspectionVersion: "sanitization-inspection-v1",
  policySchemaVersion: "sanitization-policy-v1",
  documentFacts: {
    encrypted: false, signed: false, certificationSignatureDetected: false,
    fieldMdpDetected: false, incrementalRevisionsDetected: false, xfaDetected: false,
    optionalContentDetected: false, richMediaDetected: false, acroFormDetected: false,
    calculationOrderDetected: false, documentIdentifierPresent: false,
    objectStreamsDetected: false, xrefStreamsDetected: false, pageCount: 1,
  },
  controls: SANITIZATION_CONTROL_DESCRIPTORS.map((descriptor) => ({
    controlId: descriptor.controlId,
    category: descriptor.category,
    inspectionSupport: descriptor.inspectionSupport,
    futureRemovalSupport: descriptor.futureRemovalSupport,
    expectedVisualImpact: descriptor.expectedVisualImpact,
    expectedFunctionalImpact: descriptor.expectedFunctionalImpact,
    present: false,
    findingCount: 0,
    blockingReason: null,
    representativeEvidence: [],
  })),
  blockingFindings: [], warnings: [],
  limits: { visitedObjects: 1, decodedBytes: 0, findings: 0, truncated: false },
};

describe("sanitization backend wrapper", () => {
  it("uses the canonical session job identity without any path or output payload", async () => {
    callBackend.mockResolvedValueOnce(validReport);
    const handle = startSanitizationInspection("session-1", 7);
    expect(runSessionJob).toHaveBeenCalledWith("session-1", expect.any(Function));
    expect(callBackend).toHaveBeenCalledWith("inspectDocumentSanitization", {
      jobId: "job-1",
      sessionId: "session-1",
      expectedContentGeneration: 7,
      policySchemaVersion: "sanitization-policy-v1",
    });
    expect(await handle.promise).toMatchObject({ inspectionVersion: "sanitization-inspection-v1" });
    expect(handle.cancel).toBe(cancel);
  });
});
