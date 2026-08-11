import { describe, expect, it } from "vitest";
import { createSanitizationState } from "./sanitizationState.svelte.js";

describe("sanitizationState", () => {
  it("models only idle, inspecting, review, failed, and cancelled", () => {
    const state = createSanitizationState();
    expect(state.inspectionStatus).toBe("idle");
    const revision = state.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 3 });
    expect(state.inspectionStatus).toBe("inspecting");
    expect(state.applyInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 3, revision, inspection: { controls: [] } })).toBe(true);
    expect(state.inspectionStatus).toBe("review");
    expect(state.inspection).toEqual({ controls: [] });
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
