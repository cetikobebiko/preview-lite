/**
 * Coordinates user-initiated viewer edits. It owns no reactive data: viewer and
 * annotation stores remain authoritative, while render/toast stay injected side effects.
 */
export function createViewerEditActions({ viewer, annotation, render, onToast = () => {} }) {
  function requestRender(options) {
    if (options === undefined) {
      void render();
      return;
    }
    void render(options);
  }

  function undoLastAnnotation() {
    const currentAnnotations = annotation.annotationsMap[viewer.currentPageIndex];
    if (!currentAnnotations?.length) return false;

    currentAnnotations.pop();
    annotation.annotationsMap = { ...annotation.annotationsMap };
    requestRender({ silent: true });
    onToast("Son çizim geri alındı.");
    return true;
  }

  function clearCurrentPageAnnotations() {
    annotation.annotationsMap[viewer.currentPageIndex] = [];
    annotation.annotationsMap = { ...annotation.annotationsMap };
    requestRender({ silent: true });
    onToast("Sayfadaki tüm çizimler temizlendi.");
  }

  function normalizeCurrentPage() {
    viewer.currentPageIndex = Math.min(viewer.currentPageIndex, viewer.pageOrder.length - 1);
  }

  function reorderPages(pageOrder) {
    viewer.pageOrder = pageOrder;
    normalizeCurrentPage();
    requestRender();
    onToast("Sayfa sırası değiştirildi!");
  }

  function deletePage(displayIndex) {
    if (viewer.pageOrder.length <= 1) {
      onToast("Tek sayfalı bir belgeden son sayfa silinemez!");
      return false;
    }

    viewer.pageOrder = viewer.pageOrder.filter((_, index) => index !== displayIndex);
    normalizeCurrentPage();
    requestRender();
    onToast(`Sayfa ${displayIndex + 1} kaldırıldı.`);
    return true;
  }

  function rotate(delta) {
    viewer.rotation = (viewer.rotation + delta + 360) % 360;
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
