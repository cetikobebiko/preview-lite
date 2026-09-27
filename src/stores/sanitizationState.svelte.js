import {
  SANITIZATION_ACKNOWLEDGEMENT_FIELDS,
  SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS,
  blockingSanitizationFindings,
  createSanitizationAcknowledgements,
  normalizeSanitizationPolicy,
  requiredSanitizationAcknowledgements,
} from "../features/sanitization/sanitizationSchemas.js";

export const SANITIZATION_INSPECTION_STATUSES = Object.freeze([
  "idle", "inspecting", "review", "failed", "cancelled",
]);
export const SANITIZATION_EXECUTION_STATUSES = Object.freeze([
  "idle", "executing", "completed", "failed", "cancelled",
]);

const initialAcknowledgements = () => createSanitizationAcknowledgements();

export function createSanitizationState() {
  const state = $state({
    inspection: null,
    inspectionStatus: "idle",
    inspectionError: null,

    selectedProfile: "verified_limited_v1",
    selectedControls: [...SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS],
    acknowledgements: { ...initialAcknowledgements() },

    executionStatus: "idle",
    executionError: null,
    executionResult: null,
    executionRequest: null,
    executionCancelRequested: false,
    saveDialogOpen: false,

    activeInspectionJobId: null,
    activeExecutionJobId: null,
    capturedSessionId: null,
    capturedContentGeneration: null,
    workflowRevision: 0,
  });

  function ownsInspection(jobId, sessionId, contentGeneration, revision) {
    return state.activeInspectionJobId === jobId
      && state.capturedSessionId === sessionId
      && state.capturedContentGeneration === contentGeneration
      && state.workflowRevision === revision;
  }

  function clearExecution() {
    state.executionStatus = "idle";
    state.executionError = null;
    state.executionResult = null;
    state.executionRequest = null;
    state.executionCancelRequested = false;
    state.activeExecutionJobId = null;
    state.saveDialogOpen = false;
  }

  function invalidatePolicy() {
    state.workflowRevision += 1;
    clearExecution();
  }

  function policyOrNull() {
    try {
      return normalizeSanitizationPolicy({
        profile: state.selectedProfile,
        controls: state.selectedControls,
        acknowledgements: state.acknowledgements,
        inspection: state.inspection,
      });
    } catch (_) {
      return null;
    }
  }

  return {
    get inspection() { return state.inspection; },
    get inspectionStatus() { return state.inspectionStatus; },
    get inspectionError() { return state.inspectionError; },
    get selectedProfile() { return state.selectedProfile; },
    get selectedControls() { return state.selectedControls; },
    get acknowledgements() { return state.acknowledgements; },
    get executionStatus() { return state.executionStatus; },
    get executionError() { return state.executionError; },
    get executionResult() { return state.executionResult; },
    get executionRequest() { return state.executionRequest; },
    get executionCancelRequested() { return state.executionCancelRequested; },
    get saveDialogOpen() { return state.saveDialogOpen; },
    get activeInspectionJobId() { return state.activeInspectionJobId; },
    get activeExecutionJobId() { return state.activeExecutionJobId; },
    get capturedSessionId() { return state.capturedSessionId; },
    get capturedContentGeneration() { return state.capturedContentGeneration; },
    get workflowRevision() { return state.workflowRevision; },
    get inspectionRevision() { return state.workflowRevision; },
    get requiredAcknowledgements() { return requiredSanitizationAcknowledgements(state.inspection); },
    get blockingFindings() { return blockingSanitizationFindings(state.inspection); },
    get policy() { return policyOrNull(); },
    get readyForExecution() {
      return state.inspectionStatus === "review" && state.executionStatus === "idle"
        && !state.saveDialogOpen && !state.executionResult && policyOrNull() !== null;
    },
    get workflowStatus() {
      if (state.executionStatus === "executing") return "executing";
      if (state.executionStatus === "completed") return "completed";
      if (state.executionStatus === "failed") return "failed";
      if (state.executionStatus === "cancelled") return "cancelled";
      if (state.inspectionStatus === "inspecting") return "inspecting";
      if (state.inspectionStatus === "failed") return "failed";
      if (state.inspectionStatus === "cancelled") return "cancelled";
      if (state.inspectionStatus === "review") {
        if (blockingSanitizationFindings(state.inspection).length > 0) return "blocked";
        return policyOrNull() ? "ready" : "review";
      }
      return "idle";
    },

    beginInspection({ jobId, sessionId, contentGeneration }) {
      state.workflowRevision += 1;
      state.inspection = null;
      state.inspectionStatus = "inspecting";
      state.inspectionError = null;
      state.activeInspectionJobId = jobId;
      state.capturedSessionId = sessionId;
      state.capturedContentGeneration = contentGeneration;
      clearExecution();
      return state.workflowRevision;
    },
    applyInspection({ jobId, sessionId, contentGeneration, revision, inspection }) {
      if (!ownsInspection(jobId, sessionId, contentGeneration, revision)) return false;
      state.inspection = inspection;
      state.inspectionStatus = "review";
      state.inspectionError = null;
      state.activeInspectionJobId = null;
      state.selectedProfile = "verified_limited_v1";
      state.selectedControls = [...SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS];
      state.acknowledgements = { ...initialAcknowledgements() };
      clearExecution();
      return true;
    },
    failInspection({ jobId, sessionId, contentGeneration, revision, error, cancelled = false }) {
      if (!ownsInspection(jobId, sessionId, contentGeneration, revision)) return false;
      state.inspection = null;
      state.inspectionStatus = cancelled ? "cancelled" : "failed";
      state.inspectionError = cancelled ? null : error;
      state.activeInspectionJobId = null;
      return true;
    },
    updatePolicy({ profile = state.selectedProfile, controls = state.selectedControls } = {}) {
      if (state.executionStatus === "executing") throw new Error("Execution sürerken policy değiştirilemez.");
      if (!Array.isArray(controls)) throw new TypeError("Sanitization controls bir dizi olmalıdır.");
      state.selectedProfile = profile;
      state.selectedControls = [...controls];
      invalidatePolicy();
    },
    setAcknowledgement(name, value) {
      if (!SANITIZATION_ACKNOWLEDGEMENT_FIELDS.includes(name)) throw new TypeError("Bilinmeyen acknowledgement alanı.");
      if (state.executionStatus === "executing") throw new Error("Execution sürerken acknowledgement değiştirilemez.");
      state.acknowledgements = { ...state.acknowledgements, [name]: value === true };
      invalidatePolicy();
    },
    setSaveDialogOpen(value) { state.saveDialogOpen = value === true; },
    beginExecution(jobId, request) {
      if (!request || !policyOrNull()) return false;
      state.activeExecutionJobId = jobId;
      state.executionStatus = "executing";
      state.executionError = null;
      state.executionResult = null;
      state.executionRequest = request;
      state.executionCancelRequested = false;
      return true;
    },
    requestExecutionCancel(jobId) {
      if (state.executionStatus !== "executing" || state.activeExecutionJobId !== jobId
        || state.executionCancelRequested) return false;
      state.executionCancelRequested = true;
      return true;
    },
    completeExecution(jobId, result) {
      if (state.activeExecutionJobId !== jobId || state.executionStatus !== "executing") return false;
      state.activeExecutionJobId = null;
      state.executionStatus = "completed";
      state.executionError = null;
      state.executionResult = result;
      return true;
    },
    failExecution(jobId, error, cancelled = false) {
      if (jobId !== null && state.activeExecutionJobId !== jobId) return false;
      state.activeExecutionJobId = null;
      state.executionStatus = cancelled ? "cancelled" : "failed";
      state.executionError = cancelled ? null : error;
      state.executionResult = null;
      return true;
    },
    invalidate({ status = "idle" } = {}) {
      state.workflowRevision += 1;
      state.inspection = null;
      state.inspectionStatus = status;
      state.inspectionError = null;
      state.activeInspectionJobId = null;
      state.capturedSessionId = null;
      state.capturedContentGeneration = null;
      clearExecution();
    },
    reset() {
      state.workflowRevision += 1;
      state.inspection = null;
      state.inspectionStatus = "idle";
      state.inspectionError = null;
      state.selectedProfile = "verified_limited_v1";
      state.selectedControls = [...SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS];
      state.acknowledgements = { ...initialAcknowledgements() };
      state.activeInspectionJobId = null;
      state.capturedSessionId = null;
      state.capturedContentGeneration = null;
      clearExecution();
    },
  };
}
