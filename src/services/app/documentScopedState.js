import { remapPageKeyedDrawings } from "./pageIndexMapping.js";

/**
 * Single owner for document-scoped transient viewer data: clears it when the active
 * document changes and keeps it on the right page when the backend renumbers or rotates
 * pages. Text (OCR-derived context and Bionic text) is tied to the (document,
 * contentGeneration) it was produced from; highlights and canvas drawings are tied to the
 * document and survive generation changes, since page mutations are applied explicitly.
 * Drawings must never reach another document's burn-in or notes export.
 */
export function createDocumentScopedStateReconciler({
  viewer, annotation, bindHighlightSession, remapHighlightPages, rotateHighlightPage, closeDocumentModals = () => {},
}) {
  let documentKey;
  let contentGeneration;

  const keyFor = (sessionId, imagePath) => sessionId ? `session:${sessionId}` : `image:${imagePath ?? ""}`;

  function reconcile({ sessionId, contentGeneration: nextGeneration, imagePath }) {
    const nextKey = keyFor(sessionId, imagePath);
    if (nextKey === documentKey && nextGeneration === contentGeneration) return false;
    const documentChanged = nextKey !== documentKey;
    documentKey = nextKey;
    contentGeneration = nextGeneration;
    viewer.documentTextContext = "";
    viewer.documentTextPageIndex = -1;
    viewer.bionicText = "";
    viewer.bionicSource = null;
    bindHighlightSession(sessionId ?? null);
    if (documentChanged) {
      clearDrawings();
      closeDocumentModals();
    }
    return true;
  }

  function clearDrawings() {
    annotation.annotationsMap = {};
    annotation.isDrawing = false;
    annotation.currentPoints = [];
    annotation.currentAnnoItem = null;
  }

  /** Applies a backend page mutation of the active session to page-scoped state. */
  function applyPageMutation(sessionId, { mapPageIndex = null, rotation = null } = {}) {
    if (!sessionId || keyFor(sessionId) !== documentKey) return false;
    if (typeof mapPageIndex === "function") {
      remapHighlightPages(sessionId, mapPageIndex);
      annotation.annotationsMap = remapPageKeyedDrawings(annotation.annotationsMap, mapPageIndex);
    }
    // Canvas drawings are only created on standalone images, so PDF page rotation has none to turn.
    if (rotation) rotateHighlightPage(sessionId, rotation.pageIndex, rotation.degrees);
    return true;
  }

  return Object.freeze({ reconcile, applyPageMutation });
}
