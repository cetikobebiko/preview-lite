import {
  normalizeSanitizationPolicy,
} from "../../features/sanitization/sanitizationSchemas.js";
import {
  startSanitizationExecution,
  startSanitizationInspection,
} from "../backend/sanitization.js";
import { save } from "../platform/dialogs.js";

const clone = (value) => value == null ? value : JSON.parse(JSON.stringify(value));
const isCancellation = (error) => error?.code === "SESSION_JOB_CANCELLED"
  || error?.jobOutcome?.status === "cancelled"
  || /iptal|cancel/i.test(String(error?.message || ""));

function validHandle(handle) {
  return !!(handle && typeof handle.jobId === "string" && handle.jobId.length > 0
    && typeof handle.promise?.then === "function" && typeof handle.cancel === "function");
}

function cancelHandle(handle) {
  if (!handle) return;
  try {
    const result = handle.cancel();
    if (typeof result?.catch === "function") result.catch(() => {});
  } catch (_) {
    // Best effort only. Request ownership remains the stale-result authority.
  }
}

function currentContext(getContext, captured) {
  const current = getContext?.() || {};
  return !!(current.isPdf && current.sessionId === captured.sessionId
    && current.contentGeneration === captured.contentGeneration);
}

function defaultTargetName(fileName = "document.pdf") {
  const base = String(fileName || "document.pdf").replace(/\.pdf$/i, "") || "document";
  return `${base}.sanitized-limited.pdf`;
}

export function mapSanitizationExecutionFailure(error) {
  if (isCancellation(error)) {
    return Object.freeze({ kind: "cancelled", code: "SESSION_JOB_CANCELLED", message: "Sanitization işlemi iptal edildi." });
  }
  const detail = error?.jobOutcome?.error || error?.cause || {};
  const code = detail.code || error?.code || "SANITIZATION_EXECUTION_FAILED";
  const commitState = detail.commitState ?? detail.commit_state ?? null;
  const recoveryPath = detail.recoveryPath ?? detail.recovery_path ?? null;
  const replacementDigest = detail.replacementDigest ?? detail.replacement_digest ?? null;
  let kind = "execution_failed";
  let message = detail.message || error?.message || "Sanitization işlemi tamamlanamadı.";
  if (commitState === "recovery_required") {
    kind = "recovery_required";
    message = "Dosya sistemi sonucu kurtarma gerektiriyor. Hedef ve kurtarma dosyasını dikkatle kontrol edin.";
  } else if (commitState === "indeterminate") {
    kind = "indeterminate";
    message = "Dosya sistemindeki nihai durum kesin olarak doğrulanamadı.";
  } else if (code === "SANITIZATION_TARGET_CONFLICT") {
    kind = "publication_conflict";
    message = "Hedef dosya işlem sırasında değişti. Mevcut dosyanın üzerine yazılmadı.";
  } else if (/VERIFICATION|VERIFY/.test(code)) {
    kind = "verification_failed";
    message = "Çıktı bağımsız doğrulamadan geçmedi ve kullanıcı hedefinde yayımlanmadı.";
  } else if (/STALE|CONTENT_GENERATION|INSPECTION.*MISMATCH/.test(code)) {
    kind = "inspection_stale";
    message = "Belge veya inspection authority değişti. Yeniden inceleme yapın.";
  } else if (/UNSUPPORTED|BLOCK/.test(code)) {
    kind = "blocked";
    message = detail.message || "Belgedeki desteklenmeyen içerik sanitization işlemini engelliyor.";
  }
  return Object.freeze({ kind, code, message, commitState, recoveryPath, replacementDigest });
}

export function createSanitizationController({
  state,
  getContext,
  startInspection = startSanitizationInspection,
  startExecution = startSanitizationExecution,
  saveFile = save,
}) {
  let mounted = false;
  let inspectionToken = 0;
  let executionToken = 0;
  let activeInspectionHandle = null;
  let activeExecutionHandle = null;

  function start() { mounted = true; }

  function validateContext() {
    const context = getContext?.() || {};
    if (!mounted || !context.isPdf || !context.sessionId) {
      throw new Error("Sanitization için açık ve etkin bir PDF session gereklidir.");
    }
    if (!Number.isSafeInteger(context.contentGeneration) || context.contentGeneration < 0) {
      throw new Error("Sanitization content generation bilgisi geçersiz.");
    }
    return context;
  }

  function cancelInspection({ updateState = true } = {}) {
    inspectionToken += 1;
    const handle = activeInspectionHandle;
    activeInspectionHandle = null;
    cancelHandle(handle);
    if (updateState && handle) state.invalidate({ status: "cancelled" });
    return !!handle;
  }

  function cancelExecution() {
    const handle = activeExecutionHandle;
    if (!handle || state.activeExecutionJobId !== handle.jobId) return false;
    if (!state.requestExecutionCancel(handle.jobId)) return false;
    cancelHandle(handle);
    return true;
  }

  async function inspect() {
    if (state.inspectionStatus === "inspecting") return activeInspectionHandle;
    const context = validateContext();
    cancelInspection({ updateState: false });
    const captured = Object.freeze({
      sessionId: context.sessionId,
      contentGeneration: context.contentGeneration,
    });
    const token = ++inspectionToken;
    let handle;
    try {
      handle = startInspection(captured.sessionId, captured.contentGeneration);
      if (!validHandle(handle)) throw new Error("Sanitization inspection backend handle geçersiz.");
    } catch (error) {
      state.invalidate();
      throw error;
    }
    activeInspectionHandle = handle;
    const revision = state.beginInspection({
      jobId: handle.jobId,
      sessionId: captured.sessionId,
      contentGeneration: captured.contentGeneration,
    });
    handle.promise.then((inspection) => {
      if (!mounted || token !== inspectionToken || activeInspectionHandle?.jobId !== handle.jobId
        || !currentContext(getContext, captured)) return;
      state.applyInspection({ ...captured, jobId: handle.jobId, revision, inspection });
    }).catch((error) => {
      if (!mounted || token !== inspectionToken || activeInspectionHandle?.jobId !== handle.jobId
        || !currentContext(getContext, captured)) return;
      state.failInspection({
        ...captured,
        jobId: handle.jobId,
        revision,
        error: isCancellation(error) ? null : error,
        cancelled: isCancellation(error),
      });
    }).finally(() => {
      if (token === inspectionToken && activeInspectionHandle?.jobId === handle.jobId) activeInspectionHandle = null;
    });
    return handle;
  }

  function updatePolicy(patch) {
    if (state.executionStatus === "executing") throw new Error("Execution sürerken policy değiştirilemez.");
    state.updatePolicy(patch);
  }

  function setAcknowledgement(name, value) {
    if (state.executionStatus === "executing") throw new Error("Execution sürerken acknowledgement değiştirilemez.");
    state.setAcknowledgement(name, value);
  }

  async function execute() {
    if (state.executionStatus === "executing" || state.saveDialogOpen) return null;
    if (state.executionResult) throw new Error("Tamamlanan policy tekrar çalıştırılamaz; Yeni sanitization başlatın.");
    const context = validateContext();
    if (state.capturedSessionId !== context.sessionId
      || state.capturedContentGeneration !== context.contentGeneration) {
      throw new Error("Inspection etkin belge authority'siyle eşleşmiyor.");
    }
    const policy = normalizeSanitizationPolicy({
      profile: state.selectedProfile,
      controls: clone(state.selectedControls),
      acknowledgements: clone(state.acknowledgements),
      inspection: state.inspection,
    });
    if (!state.readyForExecution) throw new Error("Authoritative inspection ve policy execution için hazır değil.");
    const request = Object.freeze({
      sessionId: context.sessionId,
      contentGeneration: context.contentGeneration,
      inspectionVersion: state.inspection.inspectionVersion,
      profile: policy.profile,
      controls: policy.controls,
      acknowledgements: Object.freeze(clone(state.acknowledgements)),
      policy,
      workflowRevision: state.workflowRevision,
    });

    state.setSaveDialogOpen(true);
    let targetPath;
    try {
      targetPath = await saveFile({
        title: "Doğrulanmış sanitization çıktısını kaydet",
        defaultPath: defaultTargetName(context.fileName),
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
    } finally {
      state.setSaveDialogOpen(false);
    }
    if (!targetPath) return null;
    if (!mounted || state.workflowRevision !== request.workflowRevision || !state.readyForExecution
      || !currentContext(getContext, request) || state.inspection?.inspectionVersion !== request.inspectionVersion) {
      throw new Error("Belge veya sanitization policy değişti; yeniden gözden geçirin.");
    }

    const token = ++executionToken;
    let handle;
    try {
      handle = startExecution(
        request.sessionId,
        request.contentGeneration,
        request.policy,
        targetPath,
      );
      if (!validHandle(handle)) throw new Error("Sanitization execution backend handle geçersiz.");
    } catch (error) {
      state.failExecution(null, mapSanitizationExecutionFailure(error));
      throw error;
    }
    activeExecutionHandle = handle;
    state.beginExecution(handle.jobId, Object.freeze({ ...request, targetPath }));
    handle.promise.then((report) => {
      if (!mounted || token !== executionToken || activeExecutionHandle?.jobId !== handle.jobId
        || state.workflowRevision !== request.workflowRevision || !currentContext(getContext, request)) return;
      state.completeExecution(handle.jobId, report);
    }).catch((error) => {
      if (!mounted || token !== executionToken || activeExecutionHandle?.jobId !== handle.jobId
        || state.workflowRevision !== request.workflowRevision || !currentContext(getContext, request)) return;
      const failure = mapSanitizationExecutionFailure(error);
      state.failExecution(handle.jobId, failure, failure.kind === "cancelled");
    }).finally(() => {
      if (token === executionToken && activeExecutionHandle?.jobId === handle.jobId) activeExecutionHandle = null;
    });
    return handle;
  }

  function reconcileContext() {
    if (!mounted) return;
    const context = getContext?.() || {};
    const captured = state.capturedSessionId !== null;
    const changed = captured && (!context.isPdf || !context.sessionId
      || context.sessionId !== state.capturedSessionId
      || context.contentGeneration !== state.capturedContentGeneration);
    if (changed) {
      cancelInspection({ updateState: false });
      executionToken += 1;
      cancelHandle(activeExecutionHandle);
      activeExecutionHandle = null;
      state.invalidate();
    }
  }

  function reset() {
    cancelInspection({ updateState: false });
    executionToken += 1;
    cancelHandle(activeExecutionHandle);
    activeExecutionHandle = null;
    state.reset();
  }

  function destroy() {
    mounted = false;
    reset();
  }

  return Object.freeze({
    start,
    inspect,
    updatePolicy,
    setAcknowledgement,
    execute,
    cancelInspection,
    cancelExecution,
    cancel: () => state.executionStatus === "executing" ? cancelExecution() : cancelInspection(),
    reconcileContext,
    reset,
    destroy,
  });
}

export { defaultTargetName as sanitizationDefaultTargetName };
