/** Coordinates viewer navigation, history, mode transitions, and zoom handoff without owning viewer state. */
export function createViewerNavigationController({ viewer, viewerController, isPdf, getViewerArea, tick, render, saveSettings, onToast }) {
  function pushHistory(page) {
    if (viewer.historyStack[viewer.historyStack.length - 1] !== page) viewer.historyStack.push(page);
    viewer.forwardStack = [];
  }
  async function jumpToPage(index, saveHistory = true) {
    if (saveHistory) pushHistory(viewer.currentPageIndex);
    viewer.currentPageIndex = index;
    await tick();
    const page = globalThis.document?.getElementById(`page-${index}`);
    if (page) page.scrollIntoView({ behavior: "auto", block: "start" });
    else if (getViewerArea()) getViewerArea().scrollTop = 1131 * viewer.scale * index;
  }
  async function goBack() {
    if (viewer.historyStack.length === 0) return;
    viewer.forwardStack.push(viewer.currentPageIndex);
    await jumpToPage(viewer.historyStack.pop(), false);
  }
  async function goForward() {
    if (viewer.forwardStack.length === 0) return;
    viewer.historyStack.push(viewer.currentPageIndex);
    await jumpToPage(viewer.forwardStack.pop(), false);
  }
  function handlePageIntersect(physicalPageIndex) {
    const displayIndex = viewer.pageOrder.findIndex((pageIndex) => pageIndex === physicalPageIndex);
    if (displayIndex >= 0) viewer.currentPageIndex = displayIndex;
  }
  async function toggleBookMode() {
    viewer.isBookMode = !viewer.isBookMode;
    if (viewer.isBookMode) viewer.isSinglePageMode = false;
    viewer.renderGenerationId += 1;
    await tick(); render(); onToast(viewer.isBookMode ? "Kitap Modu Aktif" : "Sürekli Kaydırma Modu Aktif");
  }
  async function toggleSinglePageMode() {
    viewer.isSinglePageMode = !viewer.isSinglePageMode;
    if (viewer.isSinglePageMode) viewer.isBookMode = false;
    viewer.renderGenerationId += 1;
    await tick(); render(); onToast(viewer.isSinglePageMode ? "Tek Sayfa Modu Aktif" : "Sürekli Kaydırma Modu Aktif");
  }
  function goToNextPage() {
    if (viewer.currentPageIndex + 1 >= viewer.pageOrder.length) return;
    viewer.currentPageIndex += 1;
    if (isPdf()) globalThis.document?.getElementById(`page-${viewer.pageOrder[viewer.currentPageIndex]}`)?.scrollIntoView({ behavior: "smooth" }); else render();
  }
  function goToPrevPage() {
    if (viewer.currentPageIndex <= 0) return;
    viewer.currentPageIndex -= 1;
    if (isPdf()) globalThis.document?.getElementById(`page-${viewer.pageOrder[viewer.currentPageIndex]}`)?.scrollIntoView({ behavior: "smooth" }); else render();
  }
  function setDebouncedZoom(targetScale) {
    const nextScale = Math.min(Math.max(targetScale, 0.25), 4);
    viewer.visualScale = nextScale / viewer.scale;
    viewer.isZooming = true;
    viewerController.commitZoom({ viewer, nextScale, onRender: render });
  }
  function zoomIn() { const scale = viewer.scale * viewer.visualScale; if (scale < 4) setDebouncedZoom(scale + 0.2); }
  function zoomOut() { const scale = viewer.scale * viewer.visualScale; if (scale > 0.25) setDebouncedZoom(scale - 0.2); }
  function resetZoom() { setDebouncedZoom(1); }
  function adjustZoom(delta) { viewer.scale = delta === null ? 1 : Math.min(5, Math.max(0.1, viewer.scale + delta)); saveSettings(); }
  function handlePageInputChange(event) {
    if (viewer.pageOrder.length === 0) return;
    const page = Number.parseInt(event.target.value, 10);
    if (Number.isInteger(page) && page >= 1 && page <= viewer.pageOrder.length) { viewer.currentPageIndex = page - 1; render(); }
    else event.target.value = viewer.currentPageIndex + 1;
  }
  return Object.freeze({ jumpToPage, goBack, goForward, handlePageIntersect, toggleBookMode, toggleSinglePageMode, goToNextPage, goToPrevPage, zoomIn, zoomOut, resetZoom, adjustZoom, handlePageInputChange });
}
