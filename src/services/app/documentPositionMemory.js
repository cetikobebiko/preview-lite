const MIN_SCALE = 0.25;
const MAX_SCALE = 4;

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

/**
 * Persists the small, per-document viewer preference without owning viewer state.
 * The viewer store remains the sole reactive owner; this helper only coordinates
 * storage and deferred scroll restoration.
 */
export function createDocumentPositionMemory({
  viewer,
  getPath,
  getState,
  saveState,
  onToast = () => {},
  schedule = globalThis.setTimeout,
  scrollToPage = () => {},
  onError = console.warn,
}) {
  function save() {
    const path = getPath();
    if (!path) return;

    const state = {
      pageIndex: viewer.currentPageIndex,
      scale: viewer.scale,
      isBookMode: viewer.isBookMode,
      isSinglePageMode: viewer.isSinglePageMode,
    };
    Promise.resolve(saveState(path, state)).catch(onError);
  }

  function restore(filePath) {
    const state = getState(filePath);
    if (!state) return false;

    if (Number.isInteger(state.pageIndex) && viewer.pageOrder.length > 0) {
      viewer.currentPageIndex = clamp(state.pageIndex, 0, viewer.pageOrder.length - 1);
    }
    if (Number.isFinite(state.scale)) {
      viewer.scale = clamp(state.scale, MIN_SCALE, MAX_SCALE);
    }
    if (typeof state.isBookMode === "boolean") viewer.isBookMode = state.isBookMode;
    if (typeof state.isSinglePageMode === "boolean") viewer.isSinglePageMode = state.isSinglePageMode;

    onToast(`Son kalınan konum hatırlandı (Sayfa ${viewer.currentPageIndex + 1})`);
    schedule(() => scrollToPage(viewer.currentPageIndex), 300);
    return true;
  }

  return Object.freeze({ save, restore });
}
