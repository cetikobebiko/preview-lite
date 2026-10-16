import { toPhysicalPageIndex } from "./viewerNavigationModel.js";

/**
 * Coordinates user-initiated viewer edits. It owns no reactive data: viewer and
 * annotation stores remain authoritative, while render/toast stay injected side effects.
 */
export function createViewerEditActions({ viewer, annotation, render, onToast = () => {}, preserveViewportAnchor = (mutate) => mutate() }) {
  function requestRender(options) {
    if (options === undefined) {
      void render();
      return;
    }
    void render(options);
  }

  // Drawings are keyed by physical page index (see annotationCanvasController).
  const currentPhysicalPage = () => toPhysicalPageIndex(viewer.pageOrder, viewer.currentPageIndex);

  function undoLastAnnotation() {
    const currentAnnotations = annotation.annotationsMap[currentPhysicalPage()];
    if (!currentAnnotations?.length) return false;

    currentAnnotations.pop();
    annotation.annotationsMap = { ...annotation.annotationsMap };
    requestRender({ silent: true });
    onToast("Son çizim geri alındı.");
    return true;
  }

  function clearCurrentPageAnnotations() {
    const pageIndex = currentPhysicalPage();
    if (pageIndex < 0) return;
    annotation.annotationsMap[pageIndex] = [];
    annotation.annotationsMap = { ...annotation.annotationsMap };
    requestRender({ silent: true });
    onToast("Sayfadaki tüm çizimler temizlendi.");
  }

  function normalizeCurrentPage() {
    viewer.currentPageIndex = Math.min(viewer.currentPageIndex, viewer.pageOrder.length - 1);
  }

  function keepCurrentPhysicalPage(pageOrder) {
    const currentPhysicalIndex = viewer.pageOrder[viewer.currentPageIndex];
    viewer.pageOrder = pageOrder;
    const displayIndex = pageOrder.indexOf(currentPhysicalIndex);
    if (displayIndex >= 0) viewer.currentPageIndex = displayIndex;
    else normalizeCurrentPage();
  }

  function reorderPages(pageOrder) {
    keepCurrentPhysicalPage(pageOrder);
    requestRender();
    onToast("Sayfa sırası değiştirildi!");
  }

  function deletePage(displayIndex) {
    if (viewer.pageOrder.length <= 1) {
      onToast("Tek sayfalı bir belgeden son sayfa silinemez!");
      return false;
    }

    keepCurrentPhysicalPage(viewer.pageOrder.filter((_, index) => index !== displayIndex));
    requestRender();
    onToast(`Sayfa ${displayIndex + 1} kaldırıldı.`);
    return true;
  }

  function rotate(delta) {
    // Rotation changes every page height; keep the page under the viewport centre in place.
    void preserveViewportAnchor(() => { viewer.rotation = (viewer.rotation + delta + 360) % 360; });
    requestRender();
  }

  function toggleFlip(axis) {
    if (axis === "horizontal") viewer.flipH = !viewer.flipH;
    if (axis === "vertical") viewer.flipV = !viewer.flipV;
    requestRender();
  }

  return Object.freeze({
    undoLastAnnotation,
    clearCurrentPageAnnotations,
    reorderPages,
    deletePage,
    rotateLeft: () => rotate(-90),
    rotateRight: () => rotate(90),
    toggleFlipH: () => toggleFlip("horizontal"),
    toggleFlipV: () => toggleFlip("vertical"),
  });
}
