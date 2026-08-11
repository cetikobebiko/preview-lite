import { startSanitizationInspection } from "../backend/sanitization.js";

const isCancellation = (error) =>
  error?.code === "SESSION_JOB_CANCELLED"
  || error?.jobOutcome?.status === "cancelled"
  || /iptal|cancel/i.test(String(error?.message || ""));

function validHandle(handle) {
  return !!(handle
    && typeof handle.jobId === "string"
    && typeof handle.promise?.then === "function"
    && typeof handle.cancel === "function");
}

function cancelHandle(handle) {
  if (!handle) return;
  try {
    const result = handle.cancel();
    if (typeof result?.catch === "function") result.catch(() => {});
  } catch (_) {
    // Cancellation is best effort; request identity remains the authority.
  }
}

export function createSanitizationInspectionController({
  state,
  getContext,
  startInspection = startSanitizationInspection,
}) {
  let mounted = false;
  let requestToken = 0;
  let activeHandle = null;

  function start() { mounted = true; }

  function context() {
    const current = getContext?.() || {};
    if (!mounted || !current.isPdf || !current.sessionId) {
      throw new Error("Sanitization inspection için etkin bir PDF session gereklidir.");
    }
    if (!Number.isSafeInteger(current.contentGeneration) || current.contentGeneration < 0) {
      throw new Error("Sanitization inspection content generation bilgisi geçersiz.");
    }
    return current;
  }

  function stillOwns(captured, token, jobId) {
    const current = getContext?.() || {};
    return mounted
      && requestToken === token
      && activeHandle?.jobId === jobId
      && current.isPdf
      && current.sessionId === captured.sessionId
      && current.contentGeneration === captured.contentGeneration;
  }

  function cancel({ updateState = true } = {}) {
    requestToken += 1;
    const handle = activeHandle;
    activeHandle = null;
    cancelHandle(handle);
    if (updateState && handle) state.invalidate({ status: "cancelled" });
    return !!handle;
  }

  async function inspect() {
    const current = context();
    cancel({ updateState: false });
    const captured = Object.freeze({
      sessionId: current.sessionId,
      contentGeneration: current.contentGeneration,
    });
    const token = ++requestToken;
    const handle = startInspection(captured.sessionId, captured.contentGeneration);
    if (!validHandle(handle)) throw new Error("Sanitization inspection backend handle geçersiz.");
    activeHandle = handle;
    const revision = state.beginInspection({
      jobId: handle.jobId,
      sessionId: captured.sessionId,
      contentGeneration: captured.contentGeneration,
    });
    try {
      const report = await handle.promise;
      if (!stillOwns(captured, token, handle.jobId)) {
        if (activeHandle?.jobId === handle.jobId) {
          activeHandle = null;
          state.invalidate();
        }
        return null;
      }
      activeHandle = null;
      state.applyInspection({
        jobId: handle.jobId,
        sessionId: captured.sessionId,
        contentGeneration: captured.contentGeneration,
        revision,
        inspection: report,
      });
      return report;
    } catch (error) {
      if (!stillOwns(captured, token, handle.jobId)) {
        if (activeHandle?.jobId === handle.jobId) {
          activeHandle = null;
          state.invalidate();
        }
        return null;
      }
      activeHandle = null;
      state.failInspection({
        jobId: handle.jobId,
        sessionId: captured.sessionId,
        contentGeneration: captured.contentGeneration,
        revision,
        error: isCancellation(error) ? null : error,
        cancelled: isCancellation(error),
      });
      return null;
    }
  }

  function reconcileContext() {
    if (!activeHandle && state.inspectionStatus === "idle") return;
    const current = getContext?.() || {};
    const matches = mounted
      && current.isPdf
      && current.sessionId === state.capturedSessionId
      && current.contentGeneration === state.capturedContentGeneration;
    if (!matches) {
      cancel({ updateState: false });
      state.invalidate();
    }
  }

  function destroy() {
    mounted = false;
    cancel({ updateState: false });
    state.invalidate();
  }

  return Object.freeze({ start, inspect, cancel, reconcileContext, destroy });
}
