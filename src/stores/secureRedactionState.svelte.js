import {
  REDACTION_SCHEMA_VERSION,
  normalizeRedactionAcknowledgements,
  normalizeSecureRedactionPolicy,
  validateRedactionRegionInputs,
} from "../features/redaction/redactionSchemas.js";

export const DEFAULT_SECURE_REDACTION_POLICY = Object.freeze({
  schemaVersion: REDACTION_SCHEMA_VERSION,
  profile: "secure_rasterized_pages_v1",
  strategy: "rasterized_pages",
  rasterDpi: 300,
  appearance: Object.freeze({ kind: "solid", color: "black", opacity: 1 }),
  safetyMarginPoints: 2,
  interactiveContentPolicy: "block",
  signaturePolicy: "block",
  encryptionPolicy: "block",
});

export const DEFAULT_REDACTION_ACKNOWLEDGEMENTS = Object.freeze({
  schemaVersion: REDACTION_SCHEMA_VERSION,
  rasterizedPagesAccepted: false,
  textSearchLossAccepted: false,
  accessibilityLossAccepted: false,
  annotationLossAccepted: false,
  formLossAccepted: false,
  signatureInvalidationAccepted: false,
  encryptionChangeAccepted: false,
});

const clone = (value) => value == null ? value : JSON.parse(JSON.stringify(value));

export function isSecureRedactionPlanReady(plan) {
  return !!(
    plan?.digestVerified === true &&
    plan?.executable === true &&
    plan?.readyForExecution === true &&
    Array.isArray(plan?.blockers) &&
    plan.blockers.length === 0
  );
}

export function createSecureRedactionState() {
  const state = $state({
    pendingRegions: [],
    policy: clone(DEFAULT_SECURE_REDACTION_POLICY),
    acknowledgements: clone(DEFAULT_REDACTION_ACKNOWLEDGEMENTS),
    plan: null,
    reviewPlan: null,
    planning: false,
    planningError: null,
    execution: null,
    executing: false,
    executionError: null,
    activePlanJobId: null,
    activeExecutionJobId: null,
    selectionMode: false,
    capturedSessionId: null,
    capturedContentGeneration: null,
    result: null,
    workflowRevision: 0,
    cancelRequested: false,
    saveDialogOpen: false,
    cancelled: false,
  });

  function invalidateInputs({ preserveReview = false } = {}) {
    state.workflowRevision += 1;
    state.plan = null;
    if (!preserveReview) state.reviewPlan = null;
    state.result = null;
    state.execution = null;
    state.planning = false;
    state.activePlanJobId = null;
    state.planningError = null;
    state.executionError = null;
    state.cancelled = false;
  }

  function assertEditable() {
    if (state.executing) throw new Error("Execution sürerken redaksiyon girdileri değiştirilemez.");
  }

  const api = {
    addRegion(region) {
      assertEditable();
      const next = validateRedactionRegionInputs([...state.pendingRegions, region]);
      state.pendingRegions = next;
      invalidateInputs();
      return next[next.length - 1];
    },
    addRegions(regions) {
      assertEditable();
      if (!Array.isArray(regions) || regions.length === 0) return [];
      const existingCount = state.pendingRegions.length;
      const next = validateRedactionRegionInputs([...state.pendingRegions, ...regions]);
      state.pendingRegions = next;
      invalidateInputs();
      return next.slice(existingCount);
    },
    replaceRegion(regionId, region) {
      assertEditable();
      const index = state.pendingRegions.findIndex((item) => item.id === regionId);
      if (index < 0) throw new Error(`Redaksiyon adayı bulunamadı: ${regionId}`);
      const next = [...state.pendingRegions];
      next[index] = region;
      state.pendingRegions = validateRedactionRegionInputs(next);
      invalidateInputs();
    },
    removeRegion(regionId) {
      assertEditable();
      const next = state.pendingRegions.filter((item) => item.id !== regionId);
      if (next.length === state.pendingRegions.length) return false;
      state.pendingRegions = validateRedactionRegionInputs(next);
      invalidateInputs();
      return true;
    },
    clearRegions() {
      assertEditable();
      if (state.pendingRegions.length === 0) return;
      state.pendingRegions = [];
      invalidateInputs();
    },
    setPolicy(policy) {
      assertEditable();
      state.policy = normalizeSecureRedactionPolicy(policy);
      invalidateInputs();
    },
    updatePolicy(patch) {
      api.setPolicy({ ...clone(state.policy), ...patch });
    },
    setAcknowledgement(name, accepted) {
      assertEditable();
      if (!(name in DEFAULT_REDACTION_ACKNOWLEDGEMENTS) || name === "schemaVersion") {
        throw new Error(`Bilinmeyen acknowledgement alanı: ${name}`);
      }
      state.acknowledgements = normalizeRedactionAcknowledgements({
        ...clone(state.acknowledgements),
        [name]: accepted,
      });
      invalidateInputs({ preserveReview: true });
    },
    setSelectionMode(value) {
      if (state.executing) return;
      state.selectionMode = !!value;
    },
    captureContext(sessionId, contentGeneration) {
      state.capturedSessionId = sessionId;
      state.capturedContentGeneration = contentGeneration;
    },
    beginPlanning(jobId, sessionId, contentGeneration) {
      state.planning = true;
      state.planningError = null;
      state.plan = null;
      state.result = null;
      state.cancelled = false;
      state.activePlanJobId = jobId;
      state.capturedSessionId = sessionId;
      state.capturedContentGeneration = contentGeneration;
    },
    completePlanning(jobId, plan) {
      if (state.activePlanJobId !== jobId) return false;
      state.planning = false;
      state.activePlanJobId = null;
      state.plan = plan;
      state.reviewPlan = plan;
      state.planningError = null;
      return true;
    },
    failPlanning(jobId, error) {
      if (state.activePlanJobId !== jobId) return false;
      state.planning = false;
      state.activePlanJobId = null;
      state.plan = null;
      state.planningError = error?.kind === "cancelled" ? null : error;
      state.cancelled = error?.kind === "cancelled";
      return true;
    },
    cancelPlanning(jobId = state.activePlanJobId) {
      if (jobId && state.activePlanJobId !== jobId) return false;
      state.planning = false;
      state.activePlanJobId = null;
      state.cancelled = true;
      return true;
    },
    setSaveDialogOpen(value) { state.saveDialogOpen = !!value; },
    beginExecution(jobId, snapshot) {
      state.executing = true;
      state.execution = snapshot;
      state.executionError = null;
      state.result = null;
      state.cancelRequested = false;
      state.cancelled = false;
      state.selectionMode = false;
      state.activeExecutionJobId = jobId;
    },
    requestExecutionCancel(jobId) {
      if (!jobId || state.activeExecutionJobId !== jobId) return false;
      state.cancelRequested = true;
      return true;
    },
    completeExecution(jobId, report) {
      if (state.activeExecutionJobId !== jobId) return false;
      const cancelArrivedLate = state.cancelRequested && report?.published === true;
      state.executing = false;
      state.activeExecutionJobId = null;
      state.execution = null;
      state.executionError = null;
      state.result = { report, cancelArrivedLate };
      state.cancelRequested = false;
      state.cancelled = false;
      return true;
    },
    failExecution(jobId, error) {
      if (state.activeExecutionJobId !== jobId) return false;
      state.executing = false;
      state.activeExecutionJobId = null;
      state.execution = null;
      state.result = null;
      state.executionError = error?.kind === "cancelled" ? null : error;
      state.cancelRequested = false;
      state.cancelled = error?.kind === "cancelled";
      return true;
    },
    clearTransient({ preservePending = true } = {}) {
      state.plan = null;
      state.reviewPlan = null;
      state.planning = false;
      state.planningError = null;
      state.execution = null;
      state.executing = false;
      state.executionError = null;
      state.activePlanJobId = null;
      state.activeExecutionJobId = null;
      state.selectionMode = false;
      state.result = null;
      state.cancelRequested = false;
      state.saveDialogOpen = false;
      state.cancelled = false;
      if (!preservePending) state.pendingRegions = [];
      state.workflowRevision += 1;
    },
    reset() {
      state.pendingRegions = [];
      state.policy = clone(DEFAULT_SECURE_REDACTION_POLICY);
      state.acknowledgements = clone(DEFAULT_REDACTION_ACKNOWLEDGEMENTS);
      api.clearTransient({ preservePending: true });
      state.capturedSessionId = null;
      state.capturedContentGeneration = null;
    },
    newRedaction() { api.reset(); },
    get pendingRegions() { return state.pendingRegions; },
    get policy() { return state.policy; },
    get acknowledgements() { return state.acknowledgements; },
    get plan() { return state.plan; },
    get reviewPlan() { return state.reviewPlan; },
    get planning() { return state.planning; },
    get planningError() { return state.planningError; },
    get execution() { return state.execution; },
    get executing() { return state.executing; },
    get executionError() { return state.executionError; },
    get activePlanJobId() { return state.activePlanJobId; },
    get activeExecutionJobId() { return state.activeExecutionJobId; },
    get selectionMode() { return state.selectionMode; },
    get capturedSessionId() { return state.capturedSessionId; },
    get capturedContentGeneration() { return state.capturedContentGeneration; },
    get result() { return state.result; },
    get workflowRevision() { return state.workflowRevision; },
    get cancelRequested() { return state.cancelRequested; },
    get saveDialogOpen() { return state.saveDialogOpen; },
    get readyForExecution() { return isSecureRedactionPlanReady(state.plan); },
    get status() {
      if (state.executing) return "executing";
      if (state.planning) return "planning";
      if (state.result) return "completed";
      if (state.executionError || state.planningError) return "failed";
      if (state.cancelled) return "cancelled";
      if (state.plan) return api.readyForExecution ? "ready" : "review_required";
      if (state.pendingRegions.length > 0) return "editing";
      return "idle";
    },
  };
  return api;
}
