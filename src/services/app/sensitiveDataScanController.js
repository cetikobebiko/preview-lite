import { buildLogicalHighlightsByPage } from "../../utils/pdfGeometry.js";

export function createSensitiveDataScanController({ state, startScan, getContext }) {
  let isMounted = false;
  let requestToken = 0;
  let activeHandle = null;
  let capturedContext = null;

  function cancelActive({ reset = false } = {}) {
    requestToken += 1;
    const handle = activeHandle;
    activeHandle = null;

    if (typeof handle?.cancel === "function") {
      try {
        const cancelResult = handle.cancel();
        if (cancelResult && typeof cancelResult.catch === "function") {
          cancelResult.catch(() => {});
        }
      } catch (_) {
        // Synchronous throw in cancel is swallowed best-effort
      }
    }

    if (reset) {
      capturedContext = null;
      state.reset();
    }
  }

  function start() {
    isMounted = true;
  }

  function scan(patterns) {
    if (!isMounted) return;

    // Invalidate and detach any existing active request FIRST
    cancelActive({ reset: false });

    const capturedPatterns = (patterns || [])
      .filter((pattern) => pattern && pattern.active)
      .map(({ label, regex, builtin }) => ({
        label,
        spec: builtin ? { kind: "builtin", value: builtin } : { kind: "regex", value: regex },
      }));

    if (capturedPatterns.length === 0) {
      state.failScan("En az bir aktif kalıp seçilmelidir.");
      return;
    }

    const ctx = typeof getContext === "function" ? getContext() : {};
    const { panelOpen, isPdf, sessionId, contentGeneration } = ctx;

    if (!panelOpen || !isPdf || !sessionId) {
      state.failScan("Belge taranamıyor veya panel kapalı.");
      return;
    }

    const currentToken = ++requestToken;
    capturedContext = { sessionId, contentGeneration, isPdf };

    const patternSpecs = capturedPatterns.map((p) => p.spec);
    const patternLabels = capturedPatterns.map((p) => p.label);

    state.beginScan(patternLabels);

    try {
      const handle = startScan(sessionId, contentGeneration, patternSpecs);

      const isValidHandle =
        handle &&
        typeof handle === "object" &&
        typeof handle.jobId === "string" &&
        handle.jobId.trim().length > 0 &&
        typeof handle.promise?.then === "function" &&
        typeof handle.cancel === "function";

      if (!isValidHandle) {
        capturedContext = null;
        state.failScan("Scan job başlatılamadı.");
        return;
      }

      activeHandle = handle;
      const canonicalJobId = handle.jobId;

      handle.promise
        .then((report) => {
          if (
            !isMounted ||
            requestToken !== currentToken ||
            !activeHandle ||
            activeHandle.jobId !== canonicalJobId
          ) {
            return;
          }

          const latestCtx = typeof getContext === "function" ? getContext() : {};
          if (
            !latestCtx.panelOpen ||
            !latestCtx.isPdf ||
            latestCtx.sessionId !== sessionId ||
            latestCtx.contentGeneration !== contentGeneration
          ) {
            return;
          }

          const logicalHighlights = buildLogicalHighlightsByPage(report);
          state.completeScan(report, logicalHighlights);
        })
        .catch((error) => {
          if (
            !isMounted ||
            requestToken !== currentToken ||
            !activeHandle ||
            activeHandle.jobId !== canonicalJobId
          ) {
            return;
          }

          const latestCtx = typeof getContext === "function" ? getContext() : {};
          if (
            !latestCtx.panelOpen ||
            !latestCtx.isPdf ||
            latestCtx.sessionId !== sessionId ||
            latestCtx.contentGeneration !== contentGeneration
          ) {
            return;
          }

          if (
            error?.code === "SESSION_JOB_CANCELLED" ||
            String(error?.message).includes("iptal")
          ) {
            capturedContext = null;
            state.reset();
            return;
          }

          state.failScan(error);
        })
        .finally(() => {
          if (
            isMounted &&
            requestToken === currentToken &&
            activeHandle &&
            activeHandle.jobId === canonicalJobId
          ) {
            activeHandle = null;
          }
        });
    } catch (err) {
      if (isMounted && requestToken === currentToken) {
        capturedContext = null;
        state.failScan(err);
      }
    }
  }

  function reconcileContext() {
    if (!isMounted) return;
    const ctx = typeof getContext === "function" ? getContext() : {};
    const { panelOpen, isPdf, sessionId, contentGeneration } = ctx;

    const isContextValid =
      panelOpen &&
      isPdf &&
      sessionId &&
      capturedContext &&
      capturedContext.sessionId === sessionId &&
      capturedContext.contentGeneration === contentGeneration;

    if (!isContextValid) {
      if (
        activeHandle ||
        state.isScanning ||
        state.report ||
        state.error ||
        capturedContext
      ) {
        cancelActive({ reset: true });
        capturedContext = null;
      }
    }
  }

  function destroy() {
    isMounted = false;
    cancelActive({ reset: true });
    capturedContext = null;
  }

  return Object.freeze({
    start,
    scan,
    cancelActive,
    reconcileContext,
    destroy,
  });
}
