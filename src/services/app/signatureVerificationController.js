// Request lifecycle of the signature verification panel (Prompt 11 G2): one job at a time,
// results of a superseded, cancelled or stale request never reach the state.
export function createSignatureVerificationController({ state, startInspection, getContext }) {
  let token = 0;
  let active = null;

  function cancelActive() {
    token += 1;
    const handle = active;
    active = null;
    if (typeof handle?.cancel === "function") {
      try {
        const result = handle.cancel();
        if (typeof result?.catch === "function") result.catch(() => {});
      } catch {
        // Best effort: the job may already be finished.
      }
    }
  }

  function current() {
    return typeof getContext === "function" ? getContext() : {};
  }

  function run() {
    cancelActive();
    const { sessionId, contentGeneration, isPdf } = current();
    if (!isPdf || !sessionId) {
      state.fail("İmzaları denetlemek için açık bir PDF gerekir.");
      return;
    }
    const mine = ++token;
    state.begin();
    let handle;
    try {
      handle = startInspection(sessionId, contentGeneration);
    } catch (error) {
      if (mine === token) state.fail(error);
      return;
    }
    if (!handle || typeof handle.promise?.then !== "function") {
      state.fail("İmza denetimi başlatılamadı.");
      return;
    }
    active = handle;
    const stillCurrent = () => {
      const latest = current();
      return (
        mine === token &&
        latest.isPdf &&
        latest.sessionId === sessionId &&
        latest.contentGeneration === contentGeneration
      );
    };
    handle.promise
      .then((report) => {
        if (!stillCurrent()) return;
        if (report.sessionId !== sessionId || report.contentGeneration !== contentGeneration) return;
        state.complete(report);
      })
      .catch((error) => {
        if (!stillCurrent()) return;
        if (error?.code === "SESSION_JOB_CANCELLED") {
          state.reset();
          return;
        }
        state.fail(error);
      })
      .finally(() => {
        if (mine === token) active = null;
      });
  }

  function cancel() {
    cancelActive();
    state.reset();
  }

  // The document or its generation changed: a running job is cancelled; a finished report
  // stays visible and the panel marks it stale (state.isStaleFor).
  function reconcileContext() {
    const { sessionId, isPdf } = current();
    if (!isPdf || !sessionId) {
      cancel();
      return;
    }
    if (active) cancel();
  }

  function destroy() {
    cancelActive();
    state.reset();
  }

  return Object.freeze({ run, cancel, reconcileContext, destroy });
}
