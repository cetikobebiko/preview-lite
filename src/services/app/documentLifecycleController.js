import { saveDocumentSession, saveDocumentSessionAs } from "../backend/index.js";

function isSaveConflict(error) {
  const code = error?.cause?.code ?? error?.code;
  return code === "TARGET_NOT_FOUND" || code === "DOCUMENT_SESSION_CONFLICT";
}

/** Coordinates explicit save flows and the single pending password prompt. */
export function createDocumentLifecycleController({
  documentSession,
  applySnapshot,
  chooseSavePath,
  askSaveConflict,
  getDefaultFileName,
  addRecentFile = async () => {},
  onSaved = () => {},
  onError = () => {},
  onPasswordPrompt = () => {},
  onPasswordError = () => {},
  onPasswordDismiss = () => {},
} = {}) {
  let passwordAttempt = null;
  let destroyed = false;

  function activeIdentity() {
    const sessionId = documentSession?.activeSessionId;
    return sessionId ? { sessionId, stateRevision: documentSession.stateRevision } : null;
  }

  async function saveAs() {
    const identity = activeIdentity();
    if (!identity || destroyed) return { status: "unavailable" };
    const targetPath = await chooseSavePath({ defaultPath: getDefaultFileName() });
    if (!targetPath || destroyed) return { status: "cancelled" };
    try {
      const snapshot = await saveDocumentSessionAs(identity.sessionId, targetPath, identity.stateRevision);
      if (destroyed || !applySnapshot(snapshot)) return { status: "stale" };
      await addRecentFile(snapshot.displayPath);
      onSaved({ kind: "save_as", snapshot });
      return { status: "saved", snapshot };
    } catch (error) {
      onError({ kind: "save_as", error });
      return { status: "error", error };
    }
  }

  async function save() {
    const identity = activeIdentity();
    if (!identity || destroyed) return { status: "unavailable" };
    try {
      const snapshot = await saveDocumentSession(identity.sessionId, identity.stateRevision);
      if (destroyed || !applySnapshot(snapshot)) return { status: "stale" };
      onSaved({ kind: "save", snapshot });
      return { status: "saved", snapshot };
    } catch (error) {
      onError({ kind: "save", error });
      if (isSaveConflict(error) && await askSaveConflict(error)) return saveAs();
      return { status: "error", error };
    }
  }

  function requestPassword({ attemptId, path }) {
    if (destroyed) return Promise.resolve(false);
    if (passwordAttempt && passwordAttempt.attemptId !== attemptId) {
      passwordAttempt.resolve?.(false);
      onPasswordDismiss({ attemptId: passwordAttempt.attemptId });
      passwordAttempt = null;
    }
    const candidateName = path.split(/[\\/]/).pop() || path;
    return new Promise((resolve) => {
      passwordAttempt = {
        attemptId,
        path,
        candidateName,
        error: passwordAttempt?.attemptId === attemptId ? passwordAttempt.error : "",
        resolve,
      };
      onPasswordPrompt({
        attemptId,
        candidateName,
        error: passwordAttempt.error,
      });
    });
  }

  function submitPassword(password, attemptId) {
    if (!passwordAttempt?.resolve || passwordAttempt.attemptId !== attemptId) return false;
    const resolve = passwordAttempt.resolve;
    passwordAttempt.resolve = null;
    resolve(password);
    onPasswordDismiss({ attemptId });
    return true;
  }

  function finishPasswordAttempt(attemptId) {
    if (!passwordAttempt || passwordAttempt.attemptId !== attemptId) return false;
    passwordAttempt.resolve?.(false);
    passwordAttempt = null;
    onPasswordDismiss({ attemptId });
    return true;
  }

  function reportPasswordError({ attemptId }) {
    if (!passwordAttempt || passwordAttempt.attemptId !== attemptId) return false;
    passwordAttempt.error = "Parola geçersiz. Yeniden deneyin.";
    onPasswordError({
      attemptId,
      candidateName: passwordAttempt.candidateName,
      error: passwordAttempt.error,
    });
    return true;
  }

  function dismissPasswordAttempt(attemptId) { return finishPasswordAttempt(attemptId); }
  function cancelPassword(attemptId) { return finishPasswordAttempt(attemptId); }
  function destroy() {
    if (destroyed) return;
    destroyed = true;
    if (passwordAttempt) finishPasswordAttempt(passwordAttempt.attemptId);
  }

  return Object.freeze({ save, saveAs, requestPassword, reportPasswordError, dismissPasswordAttempt, submitPassword, cancelPassword, destroy });
}
