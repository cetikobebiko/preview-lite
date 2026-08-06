/** Owns the single native window-close transaction for one mounted App. */
export function createWindowLifecycleController({
  windowApi,
  documentOpenCoordinator,
  getActiveSessionId,
  beginSessionTransition,
  chooseTransition,
  saveTransition,
  discardTransition,
  cancelTransition,
  closeSession,
  closeAllDocumentSessions,
  onError = () => {},
}) {
  let closePromise = null;
  let pendingTransition = null;
  let unlistenClose = null;
  let destroyed = false;

  async function close() {
    let windowDestroyed = false;
    try {
      const stopResult = await documentOpenCoordinator.stop();
      if (stopResult.status === "commit_outcome_unknown") {
        const result = await documentOpenCoordinator.retryReconciliation();
        if (result.status === "commit_outcome_unknown") throw new Error("Belge commit sonucu doğrulanmadan pencere kapatılamaz.");
      }
      if (stopResult.status === "precommit_recovery_required") {
        const result = await documentOpenCoordinator.retryPrecommitRecovery();
        if (result.status === "precommit_recovery_required") throw new Error("Belge temizliği doğrulanmadan pencere kapatılamaz.");
      }
      const sessionId = getActiveSessionId();
      if (sessionId) {
        const transition = pendingTransition ?? await beginSessionTransition(sessionId, "window_close");
        pendingTransition = transition;
        const decision = await chooseTransition(transition);
        if (decision.action === "cancel") {
          await cancelTransition(transition.token);
          pendingTransition = null;
          return;
        }
        if (decision.action === "save") await saveTransition(transition.token, null);
        else await discardTransition(transition.token);
        pendingTransition = null;
        closeSession(sessionId);
      }
      await closeAllDocumentSessions();
      await windowApi.destroy();
      windowDestroyed = true;
    } catch (error) {
      onError(error);
    } finally {
      if (!windowDestroyed) documentOpenCoordinator.resume();
    }
  }

  async function start() {
    const unlisten = await windowApi.onCloseRequested((event) => {
      event.preventDefault();
      if (closePromise) return;
      closePromise = close().finally(() => { closePromise = null; });
    });
    if (destroyed) unlisten();
    else unlistenClose = unlisten;
  }

  function destroy() {
    destroyed = true;
    unlistenClose?.();
    unlistenClose = null;
  }

  return Object.freeze({ start, destroy, closeInFlight: () => closePromise });
}
