import {
  findViewportAnchor,
  parsePageInput,
  resolveAnchorScrollTop,
  selectMostVisiblePage,
  toDisplayPageIndex,
  toPhysicalPageIndex,
} from "./viewerNavigationModel.js";

const NAVIGATION_LOCK_MS = 1000;

/**
 * Coordinates viewer navigation, history, mode transitions, and zoom handoff without owning viewer state.
 * Public page arguments are displayIndex values (see viewerNavigationModel.js); history stacks hold
 * physicalIndex values so back/forward keep pointing at the same PDF page after a pageOrder change.
 */
export function createViewerNavigationController({ viewer, viewerController, isPdf, getViewerArea, tick, render, saveSettings, onToast, getDocument = () => globalThis.document, now = () => Date.now() }) {
  const visibleRatios = new Map();
  let navigationLock = null;
  let zoomAnchor = null;

  const currentPhysicalIndex = () => toPhysicalPageIndex(viewer.pageOrder, viewer.currentPageIndex);
  const pageElement = (physicalIndex) => getDocument()?.getElementById(`page-${physicalIndex}`) ?? null;

  function lockNavigation(physicalIndex) {
    navigationLock = { physicalIndex, until: now() + NAVIGATION_LOCK_MS };
  }
  function scrollToDisplayPage(displayIndex, behavior) {
    if (!isPdf()) { render(); return; }
    const physicalIndex = toPhysicalPageIndex(viewer.pageOrder, displayIndex);
    if (physicalIndex < 0) return;
    lockNavigation(physicalIndex);
    pageElement(physicalIndex)?.scrollIntoView({ behavior, block: "start" });
  }
  function pushHistory(physicalIndex) {
    if (physicalIndex < 0) return;
    if (viewer.historyStack[viewer.historyStack.length - 1] !== physicalIndex) viewer.historyStack.push(physicalIndex);
    viewer.forwardStack = [];
  }
  function popDisplayIndex(stack) {
    while (stack.length > 0) {
      const displayIndex = toDisplayPageIndex(viewer.pageOrder, stack.pop());
      if (displayIndex >= 0) return displayIndex;
    }
    return -1;
  }
  async function jumpToPage(displayIndex, saveHistory = true) {
    if (toPhysicalPageIndex(viewer.pageOrder, displayIndex) < 0) return false;
    if (saveHistory && displayIndex !== viewer.currentPageIndex) pushHistory(currentPhysicalIndex());
    viewer.currentPageIndex = displayIndex;
    await tick();
    scrollToDisplayPage(displayIndex, "auto");
    return true;
  }
  function jumpToPhysicalPage(physicalIndex, saveHistory = true) {
    const displayIndex = toDisplayPageIndex(viewer.pageOrder, physicalIndex);
    return displayIndex < 0 ? Promise.resolve(false) : jumpToPage(displayIndex, saveHistory);
  }
  async function goBack() {
    const target = popDisplayIndex(viewer.historyStack);
    if (target < 0) return;
    const current = currentPhysicalIndex();
    if (current >= 0) viewer.forwardStack.push(current);
    await jumpToPage(target, false);
  }
  async function goForward() {
    const target = popDisplayIndex(viewer.forwardStack);
    if (target < 0) return;
    const current = currentPhysicalIndex();
    if (current >= 0) viewer.historyStack.push(current);
    await jumpToPage(target, false);
  }
  function handlePageIntersect(physicalPageIndex, intersectionRatio = 1) {
    if (intersectionRatio > 0) visibleRatios.set(physicalPageIndex, intersectionRatio);
    else visibleRatios.delete(physicalPageIndex);
    const selected = selectMostVisiblePage(visibleRatios, currentPhysicalIndex(), viewer.pageOrder);
    if (navigationLock) {
      if (selected !== navigationLock.physicalIndex && now() < navigationLock.until) return;
      navigationLock = null;
    }
    const displayIndex = toDisplayPageIndex(viewer.pageOrder, selected);
    if (displayIndex >= 0 && displayIndex !== viewer.currentPageIndex) viewer.currentPageIndex = displayIndex;
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
  function stepPage(delta) {
    const target = viewer.currentPageIndex + delta;
    if (toPhysicalPageIndex(viewer.pageOrder, target) < 0) return;
    viewer.currentPageIndex = target;
    scrollToDisplayPage(target, "smooth");
  }
  function goToNextPage() { stepPage(1); }
  function goToPrevPage() { stepPage(-1); }

  function viewportContentTop(area) {
    return area.getBoundingClientRect().top + (area.clientTop || 0);
  }
  function measurePage(area, physicalIndex) {
    const element = pageElement(physicalIndex);
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    return { physicalIndex, top: rect.top - viewportContentTop(area) + area.scrollTop, height: rect.height };
  }
  function centerZoomOrigin(area) {
    const container = area.querySelector?.(".canvas-container");
    if (!container) return;
    const containerTop = container.getBoundingClientRect().top - viewportContentTop(area) + area.scrollTop;
    viewer.zoomOriginY = area.scrollTop + area.clientHeight / 2 - containerTop;
  }
  /** Remembers the document point at the viewport centre before a zoom gesture starts. */
  function captureZoomAnchor() {
    zoomAnchor = null;
    const area = getViewerArea();
    if (!isPdf() || !area) return;
    const pages = viewer.pageOrder.map((physicalIndex) => measurePage(area, physicalIndex)).filter(Boolean);
    zoomAnchor = findViewportAnchor(pages, area.scrollTop, area.clientHeight);
    centerZoomOrigin(area);
  }
  /** After the committed scale is laid out, scrolls the remembered point back to the viewport centre. */
  async function restoreZoomAnchor() {
    const anchor = zoomAnchor;
    zoomAnchor = null;
    if (!anchor) return;
    await tick();
    const area = getViewerArea();
    if (!isPdf() || !area) return;
    const page = measurePage(area, anchor.physicalIndex);
    if (!page) return;
    area.scrollTop = resolveAnchorScrollTop(anchor, page, area.clientHeight, area.scrollHeight - area.clientHeight);
    centerZoomOrigin(area);
  }
  function setDebouncedZoom(targetScale) {
    const nextScale = Math.min(Math.max(targetScale, 0.25), 4);
    if (!viewer.isZooming) captureZoomAnchor();
    viewer.visualScale = nextScale / viewer.scale;
    viewer.isZooming = true;
    viewerController.commitZoom({
      viewer,
      nextScale,
      onRender: (options) => { render(options); void restoreZoomAnchor(); },
    });
  }
  function zoomIn() { const scale = viewer.scale * viewer.visualScale; if (scale < 4) setDebouncedZoom(scale + 0.2); }
  function zoomOut() { const scale = viewer.scale * viewer.visualScale; if (scale > 0.25) setDebouncedZoom(scale - 0.2); }
  function resetZoom() { setDebouncedZoom(1); }
  function adjustZoom(delta) {
    const anchored = !viewer.isZooming;
    if (anchored) captureZoomAnchor();
    viewer.scale = delta === null ? 1 : Math.min(5, Math.max(0.1, viewer.scale + delta));
    saveSettings();
    return anchored ? restoreZoomAnchor() : Promise.resolve();
  }
  function handlePageInputChange(event) {
    if (viewer.pageOrder.length === 0) return Promise.resolve(false);
    const input = event?.target;
    const displayIndex = parsePageInput(input?.value, viewer.pageOrder.length);
    if (displayIndex < 0) {
      if (input) input.value = String(viewer.currentPageIndex + 1);
      return Promise.resolve(false);
    }
    return jumpToPage(displayIndex);
  }
  return Object.freeze({ jumpToPage, jumpToPhysicalPage, goBack, goForward, handlePageIntersect, toggleBookMode, toggleSinglePageMode, goToNextPage, goToPrevPage, zoomIn, zoomOut, resetZoom, adjustZoom, captureZoomAnchor, restoreZoomAnchor, handlePageInputChange });
}
