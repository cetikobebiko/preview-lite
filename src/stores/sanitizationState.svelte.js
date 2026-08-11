export const SANITIZATION_INSPECTION_STATUSES = Object.freeze([
  "idle", "inspecting", "review", "failed", "cancelled",
]);

export function createSanitizationState() {
  const state = $state({
    inspection: null,
    inspectionStatus: "idle",
    inspectionError: null,
    activeInspectionJobId: null,
    capturedSessionId: null,
    capturedContentGeneration: null,
    inspectionRevision: 0,
  });

  function owns(jobId, sessionId, contentGeneration, revision) {
    return state.activeInspectionJobId === jobId
      && state.capturedSessionId === sessionId
      && state.capturedContentGeneration === contentGeneration
      && state.inspectionRevision === revision;
  }

  return {
    get inspection() { return state.inspection; },
    get inspectionStatus() { return state.inspectionStatus; },
    get inspectionError() { return state.inspectionError; },
    get activeInspectionJobId() { return state.activeInspectionJobId; },
    get capturedSessionId() { return state.capturedSessionId; },
    get capturedContentGeneration() { return state.capturedContentGeneration; },
    get inspectionRevision() { return state.inspectionRevision; },

    beginInspection({ jobId, sessionId, contentGeneration }) {
      state.inspectionRevision += 1;
      state.inspection = null;
      state.inspectionStatus = "inspecting";
      state.inspectionError = null;
      state.activeInspectionJobId = jobId;
      state.capturedSessionId = sessionId;
      state.capturedContentGeneration = contentGeneration;
      return state.inspectionRevision;
    },
    applyInspection({ jobId, sessionId, contentGeneration, revision, inspection }) {
      if (!owns(jobId, sessionId, contentGeneration, revision)) return false;
      state.inspection = inspection;
      state.inspectionStatus = "review";
      state.inspectionError = null;
      state.activeInspectionJobId = null;
      return true;
    },
    failInspection({ jobId, sessionId, contentGeneration, revision, error, cancelled = false }) {
      if (!owns(jobId, sessionId, contentGeneration, revision)) return false;
      state.inspection = null;
      state.inspectionStatus = cancelled ? "cancelled" : "failed";
      state.inspectionError = cancelled ? null : error;
      state.activeInspectionJobId = null;
      return true;
    },
    invalidate({ status = "idle" } = {}) {
      state.inspectionRevision += 1;
      state.inspection = null;
      state.inspectionStatus = status;
      state.inspectionError = null;
      state.activeInspectionJobId = null;
      state.capturedSessionId = null;
      state.capturedContentGeneration = null;
    },
  };
}
