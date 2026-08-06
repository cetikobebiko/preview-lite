/** Mirrors transient presentation into uiState and owns only presentation timers. */
export function createPresentationController({ ui, toastDurationMs = 3000 } = {}) {
  let toastTimer = null;
  let destroyed = false;

  function showToast(message) {
    if (destroyed) return;
    ui.toast = String(message ?? "");
    if (toastTimer !== null) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      toastTimer = null;
      if (!destroyed) ui.toast = "";
    }, toastDurationMs);
  }

  function setError(message) {
    if (!destroyed) ui.error = String(message ?? "");
  }

  function showPasswordAttempt(payload) {
    if (destroyed) return;
    ui.openModal("password", { ...payload });
  }

  function showPasswordError(payload) {
    if (destroyed) return;
    const current = ui.activeModal;
    if (current?.type === "password" && current.props?.attemptId !== payload.attemptId) return;
    ui.openModal("password", { ...payload });
  }

  function dismissPasswordAttempt({ attemptId } = {}) {
    const current = ui.activeModal;
    if (current?.type === "password" && current.props?.attemptId === attemptId) ui.closeModal("password");
  }

  function destroy() {
    if (destroyed) return;
    destroyed = true;
    if (toastTimer !== null) clearTimeout(toastTimer);
    toastTimer = null;
    ui.toast = "";
  }

  return Object.freeze({
    showToast,
    setError,
    showPasswordAttempt,
    showPasswordError,
    dismissPasswordAttempt,
    destroy,
  });
}
