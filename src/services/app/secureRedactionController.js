import {
  normalizeRedactionAcknowledgements,
  normalizeSecureRedactionPolicy,
  validateRedactionRegionInputs,
} from "../../features/redaction/redactionSchemas.js";
import { pdfPointRectToNormalizedTopLeft } from "../../utils/pdfGeometry.js";
import {
  startSecureRedactionExecution,
  startSecureRedactionPlan,
} from "../backend/redaction.js";
import { save } from "../platform/dialogs.js";

const clone = (value) => value == null ? value : JSON.parse(JSON.stringify(value));
const isCancellation = (error) =>
  error?.code === "SESSION_JOB_CANCELLED" || /iptal|cancel/i.test(String(error?.message || ""));

function validHandle(handle) {
  return !!(
    handle && typeof handle.jobId === "string" && handle.jobId.length > 0 &&
    typeof handle.promise?.then === "function" && typeof handle.cancel === "function"
  );
}

function cancelHandle(handle) {
  if (typeof handle?.cancel !== "function") return;
  try {
    const result = handle.cancel();
    if (typeof result?.catch === "function") result.catch(() => {});
  } catch (_) {
    // Cancellation is best effort; the eventual job outcome remains authoritative.
  }
}

function contextMatches(getContext, captured, requirePanel = true) {
  const latest = getContext?.() || {};
  return !!(
    (!requirePanel || latest.panelOpen) && latest.isPdf && latest.sessionId &&
    latest.sessionId === captured.sessionId &&
    latest.contentGeneration === captured.contentGeneration
  );
}

function executionFailure(error) {
  if (isCancellation(error)) {
    return { kind: "cancelled", message: "Güvenli redaksiyon işlemi iptal edildi." };
  }
  const detail = error?.jobOutcome?.error || {};
  const commitState = detail.commitState ?? detail.commit_state ?? null;
  const recoveryPath = detail.recoveryPath ?? detail.recovery_path ?? null;
  const replacementDigest = detail.replacementDigest ?? detail.replacement_digest ?? null;
  const code = detail.code || error?.code || "REDACTION_EXECUTION_FAILED";

  let kind = "execution_failed";
  let message = detail.message || error?.message || "Güvenli redaksiyon tamamlanamadı.";
  if (commitState === "recovery_required") {
    kind = "recovery_required";
    message = "Dosya sistemi sonucu kurtarma gerektiriyor. Hedef ve kurtarma dosyasını kontrol edin.";
  } else if (commitState === "indeterminate") {
    kind = "indeterminate";
    message = "Dosyanın son disk durumu belirlenemedi. Hedefi kullanmadan önce doğrulayın.";
  } else if (code === "REDACTION_TARGET_CONFLICT") {
    kind = "publication_conflict";
    message = "Hedef dosya işlem sırasında değişti; üzerine yazılmadı.";
  } else if (/VERIFY|VERIFICATION/.test(code)) {
    kind = "verification_failed";
    message = "Çıktı güvenli biçimde doğrulanamadı ve yayımlanmadı.";
  } else if (/PLAN.*DIGEST|STALE|CONTENT_GENERATION/.test(code)) {
    kind = "plan_stale";
    message = "Belge veya redaksiyon girdileri değişti; yeniden planlayın.";
  }
  return { kind, code, message, commitState, recoveryPath, replacementDigest };
}

function planningFailure(error) {
  if (isCancellation(error)) return { kind: "cancelled", message: "Planlama iptal edildi." };
  const code = error?.code || error?.jobOutcome?.error?.code || "REDACTION_PLAN_FAILED";
  return {
    kind: /STALE|CONTENT_GENERATION/.test(code) ? "plan_stale" : "planning_failed",
    code,
    message: error?.jobOutcome?.error?.message || error?.message || "Redaksiyon planı hazırlanamadı.",
  };
}

function defaultTargetName(fileName = "document.pdf") {
  const base = String(fileName || "document.pdf").replace(/\.pdf$/i, "") || "document";
  return `${base}-secure-redacted.pdf`;
}

export function createSecureRedactionController({
  state,
  getContext,
  startPlan = startSecureRedactionPlan,
  startExecution = startSecureRedactionExecution,
  saveFile = save,
}) {
  let mounted = false;
  let planToken = 0;
  let executionToken = 0;
  let activePlanHandle = null;
  let activeExecutionHandle = null;

  function start() { mounted = true; }

  function cancelPlanning({ updateState = true } = {}) {
    planToken += 1;
    const handle = activePlanHandle;
    activePlanHandle = null;
    cancelHandle(handle);
    if (updateState && handle) state.cancelPlanning(handle.jobId);
  }

  function cancelExecution() {
    const handle = activeExecutionHandle;
    if (!handle || state.activeExecutionJobId !== handle.jobId) return false;
    if (!state.requestExecutionCancel(handle.jobId)) return false;
    cancelHandle(handle);
    return true;
  }

  function validateContext() {
    const context = getContext?.() || {};
    if (!mounted || !context.panelOpen || !context.isPdf || !context.sessionId) {
      throw new Error("Güvenli redaksiyon için açık ve etkin bir PDF oturumu gereklidir.");
    }
    if (!Number.isSafeInteger(context.contentGeneration) || context.contentGeneration < 0) {
      throw new Error("Belge content generation bilgisi geçersiz.");
    }
    return context;
  }

  function captureInputs() {
    const regions = validateRedactionRegionInputs(clone(state.pendingRegions));
    if (regions.length === 0) throw new Error("En az bir geçerli redaksiyon adayı ekleyin.");
    return Object.freeze({
      regions: Object.freeze(regions.map((region) => Object.freeze(clone(region)))),
      policy: Object.freeze(normalizeSecureRedactionPolicy(clone(state.policy))),
      acknowledgements: Object.freeze(normalizeRedactionAcknowledgements(clone(state.acknowledgements))),
      workflowRevision: state.workflowRevision,
    });
  }

  function plan() {
    let context;
    let inputs;
    try {
      context = validateContext();
      inputs = captureInputs();
    } catch (error) {
      state.failPlanning(state.activePlanJobId, planningFailure(error));
      throw error;
    }

    cancelPlanning({ updateState: false });
    const token = ++planToken;
    const captured = {
      sessionId: context.sessionId,
      contentGeneration: context.contentGeneration,
      workflowRevision: inputs.workflowRevision,
    };
    let handle;
    try {
      handle = startPlan(
        captured.sessionId,
        captured.contentGeneration,
        inputs.policy,
        inputs.acknowledgements,
        inputs.regions,
      );
      if (!validHandle(handle)) throw new Error("Plan job başlatılamadı.");
    } catch (error) {
      state.failPlanning(null, planningFailure(error));
      throw error;
    }
    activePlanHandle = handle;
    state.beginPlanning(handle.jobId, captured.sessionId, captured.contentGeneration);

    handle.promise.then((authoritativePlan) => {
      if (
        !mounted || token !== planToken || activePlanHandle?.jobId !== handle.jobId ||
        state.workflowRevision !== captured.workflowRevision ||
        !contextMatches(getContext, captured)
      ) return;
      state.completePlanning(handle.jobId, authoritativePlan);
    }).catch((error) => {
      if (
        !mounted || token !== planToken || activePlanHandle?.jobId !== handle.jobId ||
        state.workflowRevision !== captured.workflowRevision ||
        !contextMatches(getContext, captured)
      ) return;
      state.failPlanning(handle.jobId, planningFailure(error));
    }).finally(() => {
      if (token === planToken && activePlanHandle?.jobId === handle.jobId) activePlanHandle = null;
    });
    return handle;
  }

  async function execute() {
    if (state.executing || state.saveDialogOpen) return null;
    if (state.result) throw new Error("Tamamlanan plan tekrar çalıştırılamaz; Yeni redaksiyon ile sıfırlayın.");
    const context = validateContext();
    if (!state.readyForExecution) throw new Error("En son authoritative plan execution için hazır değil.");
    const inputs = captureInputs();
    const planSnapshot = clone(state.plan);
    if (planSnapshot.contentGeneration !== context.contentGeneration) {
      throw new Error("Plan belge generation bilgisiyle eşleşmiyor; yeniden planlayın.");
    }
    const request = Object.freeze({
      sessionId: context.sessionId,
      contentGeneration: context.contentGeneration,
      policy: inputs.policy,
      acknowledgements: inputs.acknowledgements,
      regions: inputs.regions,
      planDigest: planSnapshot.planDigest,
      workflowRevision: inputs.workflowRevision,
    });

    state.setSaveDialogOpen(true);
    let targetPath;
    try {
      targetPath = await saveFile({
        title: "Doğrulanmış güvenli redaksiyon çıktısını kaydet",
        defaultPath: defaultTargetName(context.fileName),
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
    } finally {
      state.setSaveDialogOpen(false);
    }
    if (!targetPath) return null;
    if (
      !mounted || state.workflowRevision !== request.workflowRevision || !state.readyForExecution ||
      !contextMatches(getContext, request)
    ) throw new Error("Belge veya redaksiyon girdileri değişti; yeniden planlayın.");

    const token = ++executionToken;
    let handle;
    try {
      handle = startExecution(
        request.sessionId,
        request.contentGeneration,
        request.policy,
        request.acknowledgements,
        request.regions,
        request.planDigest,
        targetPath,
      );
      if (!validHandle(handle)) throw new Error("Execution job başlatılamadı.");
    } catch (error) {
      state.failExecution(null, executionFailure(error));
      throw error;
    }
    activeExecutionHandle = handle;
    state.beginExecution(handle.jobId, { ...request, targetPath });

    handle.promise.then((report) => {
      if (
        !mounted || token !== executionToken || activeExecutionHandle?.jobId !== handle.jobId ||
        !contextMatches(getContext, request, false)
      ) return;
      if (report?.published !== true || report?.verificationReport?.isVerified !== true) {
        state.failExecution(handle.jobId, {
          kind: "verification_failed",
          message: "Çıktı güvenli biçimde doğrulanamadı ve yayımlanmadı.",
        });
        return;
      }
      state.completeExecution(handle.jobId, report);
    }).catch((error) => {
      if (
        !mounted || token !== executionToken || activeExecutionHandle?.jobId !== handle.jobId ||
        !contextMatches(getContext, request, false)
      ) return;
      state.failExecution(handle.jobId, executionFailure(error));
    }).finally(() => {
      if (token === executionToken && activeExecutionHandle?.jobId === handle.jobId) activeExecutionHandle = null;
    });
    return handle;
  }

  function mutate(callback) {
    if (state.executing) throw new Error("Execution sürerken girdiler değiştirilemez.");
    cancelPlanning({ updateState: false });
    return callback();
  }

  function addManualRegion({ pageIndex, normalizedTopLeftRect }) {
    const context = validateContext();
    return mutate(() => {
      state.captureContext(context.sessionId, context.contentGeneration);
      return state.addRegion({
        id: `redaction-region:${crypto.randomUUID()}`,
        pageIndex,
        normalizedTopLeftRect,
        source: "manual_selection",
      });
    });
  }

  function promoteSensitiveScanItem(item) {
    if (
      item?.geometryStatus !== "resolved" || !item.pageGeometry ||
      !Array.isArray(item.sourceRectsPdfPoints) || item.sourceRectsPdfPoints.length === 0
    ) return [];
    const context = validateContext();
    return mutate(() => {
      state.captureContext(context.sessionId, context.contentGeneration);
      const candidates = item.sourceRectsPdfPoints.map((pdfRect) => {
        const normalizedTopLeftRect = pdfPointRectToNormalizedTopLeft(pdfRect, item.pageGeometry);
        if (!normalizedTopLeftRect) throw new Error("Tarama sonucunun sayfa geometrisi redaksiyon adayına çevrilemedi.");
        const { space: _space, ...rect } = normalizedTopLeftRect;
        return {
          id: `redaction-region:${crypto.randomUUID()}`,
          pageIndex: item.pageIndex,
          normalizedTopLeftRect: rect,
          source: "sensitive_scan_candidate",
        };
      });
      return state.addRegions(candidates);
    });
  }

  function setSelectionMode(value) {
    if (value) {
      const context = validateContext();
      state.captureContext(context.sessionId, context.contentGeneration);
    }
    state.setSelectionMode(value);
  }

  function reconcileContext() {
    if (!mounted) return;
    const context = getContext?.() || {};
    const hasCapturedContext = state.capturedSessionId !== null;
    const contextChanged = hasCapturedContext && (
      !context.isPdf || !context.sessionId ||
      state.capturedSessionId !== context.sessionId ||
      state.capturedContentGeneration !== context.contentGeneration
    );
    if (contextChanged) {
      cancelPlanning({ updateState: false });
      executionToken += 1;
      cancelHandle(activeExecutionHandle);
      activeExecutionHandle = null;
      state.reset();
      return;
    }
    if (!context.panelOpen && (
      activePlanHandle || activeExecutionHandle || state.selectionMode || state.plan || state.result ||
      state.planningError || state.executionError
    )) {
      cancelPlanning({ updateState: false });
      executionToken += 1;
      cancelHandle(activeExecutionHandle);
      activeExecutionHandle = null;
      state.clearTransient({ preservePending: true });
    }
  }

  function destroy() {
    mounted = false;
    cancelPlanning({ updateState: false });
    executionToken += 1;
    cancelHandle(activeExecutionHandle);
    activeExecutionHandle = null;
    state.reset();
  }

  return Object.freeze({
    start,
    plan,
    execute,
    cancelPlanning,
    cancelExecution,
    reconcileContext,
    destroy,
    addManualRegion,
    promoteSensitiveScanItem,
    removeRegion: (id) => mutate(() => state.removeRegion(id)),
    clearRegions: () => mutate(() => state.clearRegions()),
    setPolicy: (policy) => mutate(() => state.setPolicy(policy)),
    updatePolicy: (patch) => mutate(() => state.updatePolicy(patch)),
    setAcknowledgement: (name, value) => mutate(() => state.setAcknowledgement(name, value)),
    setSelectionMode,
    newRedaction: () => state.newRedaction(),
  });
}

export { executionFailure as mapSecureRedactionExecutionFailure };
