import { createDocumentOpenCoordinator } from "../documentOpenCoordinator.js";

export function isCurrentDocumentSnapshot(documentSession, snapshot) {
  return documentSession.activeSessionId === snapshot.id
    && documentSession.contentGeneration === snapshot.contentGeneration;
}

/** Owns document-open transaction wiring, recovery retries, and viewer activation sequencing. */
export function createDocumentOpenController({
  backend, documentSession, documentLifecycleController, viewer, annotation, viewerController,
  renderSession, getCanvas, getIsMounted, tick, applyBackendSessionSnapshot,
  chooseTransition, chooseTransitionSaveRecovery, invalidateSearch, restoreDocumentPosition,
  addRecentFile, getOutline, getMetadata, setError, chooseFile,
  onWarning = console.warn,
}) {
  let automaticCommitRetryAttemptId = null;
  let automaticRecoveryRetryAttemptId = null;

  const coordinator = createDocumentOpenCoordinator({
    ...backend,
    readImageFile: backend.readImageFile,
    requestPassword: documentLifecycleController.requestPassword,
    dismissAttemptDialog: documentLifecycleController.dismissPasswordAttempt,
    chooseTransition,
    onTransitionSaveError: chooseTransitionSaveRecovery,
    onPasswordError: documentLifecycleController.reportPasswordError,
    prepareImage: ({ decoded }) => {
      if (!decoded || !Number.isInteger(decoded.width) || decoded.width < 1 || !Number.isInteger(decoded.height) || decoded.height < 1 || decoded.data?.length !== decoded.width * decoded.height * 4) {
        throw new Error("IMAGE_VALIDATION_FAILED");
      }
    },
    prepareImageActivation: async () => {
      await tick();
      if (!getIsMounted()) throw new Error("VIEWER_UNAVAILABLE");
    },
    applyAuthoritativeActiveSnapshot: (snapshot) => applyBackendSessionSnapshot(snapshot),
    commitDocumentSnapshot: (snapshot) => {
      documentSession.commitCandidate(snapshot);
      viewer.activeImageSourcePath = "";
      viewer.fileExt = "pdf";
    },
    reconcileCommittedDocumentSnapshot: async (snapshot) => {
      documentSession.commitCandidate(snapshot);
      viewer.activeImageSourcePath = "";
      viewer.fileExt = "pdf";
    },
    activateCommittedPdfRender: (snapshot) => viewerController.activatePdf(snapshot),
    afterPdfCommit: async ({ snapshot, pageCount }) => {
      viewer.pageOrder = Array.from({ length: pageCount }, (_, index) => index);
      viewer.currentPageIndex = 0;
      viewer.scale = 1;
      viewer.visualScale = 1;
      viewer.rotation = 0;
      viewer.flipH = false;
      viewer.flipV = false;
      annotation.activeTool = "pointer";
      viewer.outlineList = [];
      viewer.pdfMetadata = { title: "", author: "", subject: "", keywords: "", creator: "" };
      void getOutline(snapshot.id, snapshot.contentGeneration)
        .then((outline) => { if (getIsMounted() && isCurrentDocumentSnapshot(documentSession, snapshot)) viewer.outlineList = Array.isArray(outline) ? outline : []; })
        .catch((error) => onWarning("PDF outline yüklenemedi:", error));
      void getMetadata(snapshot.id, snapshot.contentGeneration)
        .then((metadata) => { if (getIsMounted() && isCurrentDocumentSnapshot(documentSession, snapshot)) viewer.pdfMetadata = metadata; })
        .catch((error) => onWarning("PDF metadata yüklenemedi:", error));
      restoreDocumentPosition(snapshot.displayPath);
      await addRecentFile(snapshot.displayPath);
    },
    publishCommittedImage: ({ path }) => {
      const previousSessionId = documentSession.activeSessionId;
      if (previousSessionId) documentSession.closeSession(previousSessionId);
      viewer.activeImageSourcePath = path;
      viewer.fileExt = path.split(".").pop()?.toLowerCase() || "";
      viewer.pageOrder = [0];
      viewer.currentPageIndex = 0;
    },
    reconcileCommittedImage: ({ path }) => {
      const previousSessionId = documentSession.activeSessionId;
      if (previousSessionId) documentSession.closeSession(previousSessionId);
      viewer.activeImageSourcePath = path;
      viewer.fileExt = path.split(".").pop()?.toLowerCase() || "";
      viewer.pageOrder = [0];
      viewer.currentPageIndex = 0;
    },
    activateCommittedImageViewer: async () => {
      await viewerController.deactivatePdf(renderSession.snapshot().contentGeneration);
    },
    afterImageCommit: async ({ path, decoded }) => {
      await tick();
      const canvas = getCanvas();
      if (canvas) viewerController.paintImage(canvas, decoded);
      restoreDocumentPosition(path);
      await addRecentFile(path);
    },
    onViewerActivationError: ({ error }) => setError("Belge açıldı; görüntüleyici etkinleştirmesi yeniden denenmeli: " + error.toString()),
    onCommitOutcomeUnknown: ({ attemptId }) => {
      setError("Belge geçiş sonucu doğrulanamadı. Yeni açma istekleri bekletiliyor.");
      if (automaticCommitRetryAttemptId === attemptId) return;
      automaticCommitRetryAttemptId = attemptId;
      void Promise.resolve().then(() => coordinator.retryReconciliation()).then((result) => {
        if (result.status === "committed" || result.status === "rolled_back") setError("");
      });
    },
    onPrecommitRecoveryRequired: ({ attemptId }) => {
      setError("Belge geçiş temizliği yeniden doğrulanmalı.");
      if (automaticRecoveryRetryAttemptId === attemptId) return;
      automaticRecoveryRetryAttemptId = attemptId;
      void Promise.resolve().then(() => coordinator.retryPrecommitRecovery()).then((result) => {
        if (result.status === "recovered") setError("");
      });
    },
    onCleanupWarnings: (warnings) => { if (warnings.length > 0) onWarning("Session cleanup ertelendi:", warnings); },
    onSecondaryError: (error) => onWarning("Belge sonrası ikincil işlem başarısız:", error),
    onError: ({ error }) => setError("Belge yüklenemedi: " + error.toString()),
    onStateChange: ({ phase }) => {
      viewer.isDocumentLoading = !["idle", "commit_outcome_unknown"].includes(phase);
      if (phase !== "idle") invalidateSearch({ clearQuery: true });
    },
  });

  async function requestFile(filePath) {
    if (!getIsMounted() || typeof filePath !== "string" || !filePath.trim()) return;
    viewer.isRendering = true;
    setError("");
    const extension = filePath.split(".").pop()?.toLowerCase() || "";
    const result = extension === "pdf" ? await coordinator.requestPdf(filePath) : await coordinator.requestImage(filePath);
    if (result.status !== "commit_outcome_unknown") viewer.isRendering = false;
    return result;
  }

  async function selectFile() {
    setError("");
    try {
      const selected = await chooseFile?.();
      if (!selected) return;
      const filePath = typeof selected === "string" ? selected : selected.path;
      return requestFile(filePath);
    } catch (error) {
      onWarning("Dosya seçimi hatası:", error);
      setError("Dosya açılırken hata oluştu: " + error.toString());
    }
  }

  return Object.freeze({ coordinator, requestFile, selectFile });
}
