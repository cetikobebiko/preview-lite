import { describe, expect, it } from "vitest";
import { createSanitizationState } from "./sanitizationState.svelte.js";

describe("sanitizationState", () => {
  it("separates inspection and execution lifecycle state", () => {
    const state = createSanitizationState();
    expect(state.inspectionStatus).toBe("idle");
    const revision = state.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 3 });
    expect(state.inspectionStatus).toBe("inspecting");
    expect(state.applyInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 3, revision, inspection: { controls: [] } })).toBe(true);
    expect(state.inspectionStatus).toBe("review");
    expect(state.inspection).toEqual({ controls: [] });
  });

  it("derives blocked/review/ready and invalidates ready snapshots on policy changes", () => {
    const state = createSanitizationState();
    let revision = state.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1 });
    state.applyInspection({
      jobId: "job-1", sessionId: "session-1", contentGeneration: 1, revision,
      inspection: {
        inspectionVersion: "sanitization-inspection-v1", policySchemaVersion: "sanitization-policy-v1",
        controlRegistryVersion: "sanitization-controls/1", documentFacts: { signed: false }, blockingFindings: [],
      },
    });
    expect(state.workflowStatus).toBe("ready");
    const readyRevision = state.workflowRevision;
    state.updatePolicy({ profile: "custom_supported_v1", controls: ["incremental_revisions"] });
    expect(state.workflowRevision).toBeGreaterThan(readyRevision);
    expect(state.workflowStatus).toBe("ready");
    expect(state.executionResult).toBeNull();

    state.beginExecution("execute-1", { immutable: true });
    expect(state.workflowStatus).toBe("executing");
    expect(state.requestExecutionCancel("execute-1")).toBe(true);
    expect(state.requestExecutionCancel("execute-1")).toBe(false);
    state.completeExecution("execute-1", { publication: { published: true } });
    expect(state.workflowStatus).toBe("completed");
  });

  it("requires signature acknowledgement and never lets unsupported blockers become ready", () => {
    const signed = createSanitizationState();
    let revision = signed.beginInspection({ jobId: "signed", sessionId: "session", contentGeneration: 1 });
    signed.applyInspection({
      jobId: "signed", sessionId: "session", contentGeneration: 1, revision,
      inspection: {
        inspectionVersion: "sanitization-inspection-v1", policySchemaVersion: "sanitization-policy-v1",
        controlRegistryVersion: "sanitization-controls/1", documentFacts: { signed: true },
        blockingFindings: [{ controlId: "digital_signatures" }],
      },
    });
    expect(signed.workflowStatus).toBe("review");
    expect(signed.acknowledgements.signatureInvalidationAccepted).toBe(false);
    signed.setAcknowledgement("signatureInvalidationAccepted", true);
    expect(signed.workflowStatus).toBe("ready");

    const blocked = createSanitizationState();
    revision = blocked.beginInspection({ jobId: "blocked", sessionId: "session", contentGeneration: 1 });
    blocked.applyInspection({
      jobId: "blocked", sessionId: "session", contentGeneration: 1, revision,
      inspection: {
        inspectionVersion: "sanitization-inspection-v1", policySchemaVersion: "sanitization-policy-v1",
        controlRegistryVersion: "sanitization-controls/1", documentFacts: { signed: false },
        blockingFindings: [{ controlId: "encryption" }],
      },
    });
    expect(blocked.workflowStatus).toBe("blocked");
    expect(blocked.readyForExecution).toBe(false);
  });

  it("rejects stale job/session/generation/revision updates", () => {
    const state = createSanitizationState();
    const revision = state.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 3 });
    expect(state.applyInspection({ jobId: "job-2", sessionId: "session-1", contentGeneration: 3, revision, inspection: {} })).toBe(false);
    expect(state.applyInspection({ jobId: "job-1", sessionId: "session-2", contentGeneration: 3, revision, inspection: {} })).toBe(false);
    expect(state.applyInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 4, revision, inspection: {} })).toBe(false);
    expect(state.applyInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 3, revision: revision + 1, inspection: {} })).toBe(false);
    expect(state.inspectionStatus).toBe("inspecting");
  });

  it("separates failure, cancellation, and invalidation", () => {
    const state = createSanitizationState();
    let revision = state.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1 });
    state.failInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1, revision, error: new Error("failed") });
    expect(state.inspectionStatus).toBe("failed");
    revision = state.beginInspection({ jobId: "job-2", sessionId: "session-1", contentGeneration: 1 });
    state.failInspection({ jobId: "job-2", sessionId: "session-1", contentGeneration: 1, revision, error: null, cancelled: true });
    expect(state.inspectionStatus).toBe("cancelled");
    state.invalidate();
    expect(state.inspectionStatus).toBe("idle");
    expect(state.capturedSessionId).toBeNull();
  });
});
