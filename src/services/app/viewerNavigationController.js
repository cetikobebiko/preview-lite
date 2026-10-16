import {
  findViewportAnchor,
  parsePageInput,
  resolveAnchorScrollTop,
  selectMostVisiblePage,
  toDisplayPageIndex,
  toPhysicalPageIndex,
} from "./viewerNavigationModel.js";
import { MAX_SCALE, MIN_SCALE, normalizeScale } from "./zoomLimits.js";

const NAVIGATION_LOCK_MS = 1000;

/**
 * Coordinates viewer navigation, history, mode transitions, and zoom handoff without owning viewer state.
 * Public page arguments are displayIndex values (see viewerNavigationModel.js); history stacks hold
 * physicalIndex values so back/forward keep pointing at the same PDF page after a pageOrder change.
 * A page that is not in the DOM is located through `getPageLayout` (pageLayoutModel.js), which
 * is null while worker page sizes are unknown; such a page is then not located at all.
 */
export function createViewerNavigationController({ viewer, viewerController, isPdf, getViewerArea, tick, render, saveSettings, onToast, getDocument = () => globalThis.document, getPageLayout = () => null, now = () => Date.now() }) {
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
    scrollToPhysicalPage(physicalIndex, { behavior, block: "start" });
  }
  /**
   * Scrolls a physical page to the viewport top ("start") or centre ("center"). Returns false
   * when the page is neither in the DOM nor in the layout model.
   */
  function scrollToPhysicalPage(physicalIndex, { behavior = "auto", block = "start" } = {}) {
    if (!isPdf()) return false;
    const element = pageElement(physicalIndex);
    if (element) {
      element.scrollIntoView({ behavior, block });
      return true;
    }
    const area = getViewerArea();
    const page = area ? modelPage(area, physicalIndex, listTop(area)) : null;
    if (!page) return false;
    const top = block === "center" ? page.top + page.height / 2 - area.clientHeight / 2 : page.top;
    const target = Math.max(0, top);
    if (behavior === "smooth" && typeof area.scrollTo === "function") area.scrollTo({ top: target, behavior });
    else area.scrollTop = target;
    return true;
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
    // Spreads regroup the rows, so the page under the viewport centre is kept in place.
    await preserveViewportAnchor(() => {
      viewer.isBookMode = !viewer.isBookMode;
      if (viewer.isBookMode) viewer.isSinglePageMode = false;
      viewer.renderGenerationId += 1;
    });
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
  /** Top of the page list (`.pdf-continuous-scroll`) in scroll-content coordinates, or null. */
  function listTop(area) {
    const list = area.querySelector?.(".pdf-continuous-scroll");
    if (!list) return null;
    return list.getBoundingClientRect().top - viewportContentTop(area) + area.scrollTop;
  }
  function modelPage(area, physicalIndex, origin) {
    if (origin === null) return null;
    const rect = getPageLayout()?.pageRect(physicalIndex);
    return rect ? { physicalIndex, top: origin + rect.top, height: rect.height } : null;
  }
  /** Page box in scroll-content coordinates: measured when mounted, else from the layout model. */
  function measurePage(area, physicalIndex, origin) {
    const element = pageElement(physicalIndex);
    if (!element) return modelPage(area, physicalIndex, origin === undefined ? listTop(area) : origin);
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
    const origin = listTop(area);
    const pages = viewer.pageOrder.map((physicalIndex) => measurePage(area, physicalIndex, origin)).filter(Boolean);
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
  /** Keeps the viewport-centre document point in place across a layout change (e.g. rotation). */
  function preserveViewportAnchor(mutate) {
    if (viewer.isZooming) { mutate(); return Promise.resolve(); }
    captureZoomAnchor();
    mutate();
    return restoreZoomAnchor();
  }
  function setDebouncedZoom(targetScale) {
    const nextScale = normalizeScale(targetScale);
    if (!viewer.isZooming) captureZoomAnchor();
    viewer.visualScale = nextScale / viewer.scale;
    viewer.isZooming = true;
    viewerController.commitZoom({
      viewer,
      nextScale,
      onRender: (options) => { render(options); void restoreZoomAnchor(); },
    });
  }
  function zoomIn() { const scale = viewer.scale * viewer.visualScale; if (scale < MAX_SCALE) setDebouncedZoom(scale + 0.2); }
  function zoomOut() { const scale = viewer.scale * viewer.visualScale; if (scale > MIN_SCALE) setDebouncedZoom(scale - 0.2); }
  function resetZoom() { setDebouncedZoom(1); }
  /**
   * Keyboard zoom: steps the committed scale by `delta`, or resets it to 1 when `delta` is null.
   * During a wheel or button zoom the step continues from the scale on screen (scale × visualScale)
   * and replaces the pending deferred commit, which would otherwise overwrite this scale with its own
   * product 300 ms later. The gesture's anchor (captured when it started) is restored as that
   * commit would have done.
   */
  function adjustZoom(delta) {
    const gesture = viewer.isZooming;
    if (gesture) viewerController.cancelPendingZoom();
    else captureZoomAnchor();
    const base = gesture ? viewer.scale * viewer.visualScale : viewer.scale;
    viewer.scale = delta === null ? 1 : normalizeScale(base + delta);
    viewer.visualScale = 1;
    viewer.isZooming = false;
    if (gesture) render({ silent: true });
    saveSettings();
    return restoreZoomAnchor();
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
  return Object.freeze({ jumpToPage, jumpToPhysicalPage, scrollToPhysicalPage, goBack, goForward, handlePageIntersect, toggleBookMode, toggleSinglePageMode, goToNextPage, goToPrevPage, zoomIn, zoomOut, resetZoom, adjustZoom, captureZoomAnchor, restoreZoomAnchor, preserveViewportAnchor, handlePageInputChange });
}
