import {
  estimateRenderedPageCssSize,
  isValidPageSize,
  renderDevicePixelRatio,
} from "../../utils/pageGeometry.js";

/**
 * Vertical layout of the PDF page list, computed without the DOM.
 *
 * Rows are the viewer's spreads: one page per row in continuous and single-page mode,
 * up to two side by side in book mode. Positions are CSS pixels relative to the top of
 * `.pdf-continuous-scroll`, before the pinch-zoom `visualScale` transform. Single-page
 * mode keeps inactive pages as empty boxes of the same height, so it shares this layout.
 *
 * VIEWER_ROW_LAYOUT mirrors the CSS that boxes a row today:
 *   row box   = max(minRowHeight, tallest page + page gap)   (ViewerShell .book-spread-row)
 *   row pitch = row box + rowGap                              (.book-spread-row margin-bottom)
 *   page gap  = PdfPage .pdf-page-container margin-bottom; 0 in book mode (app.css)
 * Page heights are the placeholder heights PdfPage gets from the worker page size.
 *
 * Only worker sizes are used. Without one PdfPage guesses (this page's last rendered
 * height, the document's last measured page, A4) from per-page and per-component state
 * the model cannot see, so a model built on its own guess would drift from the DOM.
 * The model is therefore null when any listed page has no usable worker size, and
 * callers fall back to the DOM.
 */
export const VIEWER_ROW_LAYOUT = Object.freeze({
  minRowHeight: 800,
  rowGap: 24,
  pageGapContinuous: 24,
  pageGapBook: 0,
});

function upperBound(tops, rowCount, y) {
  let low = 0;
  let high = rowCount - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (tops[middle] <= y) low = middle;
    else high = middle - 1;
  }
  return low;
}

/**
 * @param {object} options
 * @param {number[][]} options.spreads physical page indices per row, as viewer.spreads
 * @param {Array<{widthPt:number,heightPt:number}|null>|null} options.pageSizes worker sizes by physical index
 * @returns the layout, or null when a listed page has no usable worker size (or its
 *   height cannot be estimated at this scale)
 */
export function createPageLayoutModel({
  spreads,
  pageSizes = null,
  scale,
  rotation = 0,
  devicePixelRatio = 1,
  isBookMode = false,
  layout = VIEWER_ROW_LAYOUT,
}) {
  const rows = Array.isArray(spreads) ? spreads : [];
  const rowCount = rows.length;
  const pageGap = isBookMode ? layout.pageGapBook : layout.pageGapContinuous;
  const tops = new Float64Array(rowCount + 1);
  const pageHeights = new Map();
  const rowOfPage = new Map();

  for (let row = 0; row < rowCount; row += 1) {
    let tallest = 0;
    for (const physicalIndex of rows[row]) {
      const size = pageSizes?.[physicalIndex];
      if (!isValidPageSize(size)) return null;
      const height = estimateRenderedPageCssSize({ pageSize: size, scale, rotation, devicePixelRatio })?.height;
      if (height === undefined) return null;
      pageHeights.set(physicalIndex, height);
      rowOfPage.set(physicalIndex, row);
      tallest = Math.max(tallest, height + pageGap);
    }
    tops[row + 1] = tops[row] + Math.max(layout.minRowHeight, tallest) + layout.rowGap;
  }

  const totalHeight = tops[rowCount];

  /** Row whose pitch contains y (clamped to the first/last row); -1 for an empty list. */
  function rowAt(y) {
    if (rowCount === 0) return -1;
    return upperBound(tops, rowCount, y);
  }

  /** { physicalIndex, top, height } of a page box, or null when the page is not listed. */
  function pageRect(physicalIndex) {
    const row = rowOfPage.get(physicalIndex);
    if (row === undefined) return null;
    return { physicalIndex, top: tops[row], height: pageHeights.get(physicalIndex) };
  }

  /**
   * Rows [startRow, endRow] intersecting [top - overscan, top + height + overscan].
   * Empty list: { startRow: 0, endRow: -1 }.
   */
  function rowsInRange(top, height, overscan = 0) {
    if (rowCount === 0) return { startRow: 0, endRow: -1 };
    return { startRow: rowAt(top - overscan), endRow: rowAt(top + height + overscan) };
  }

  return Object.freeze({
    rowCount,
    totalHeight,
    rowTop: (row) => tops[Math.min(Math.max(row, 0), rowCount)],
    rowAt,
    rowsInRange,
    pageRect,
    pageRects: () => rows.flatMap((spread) => spread.map(pageRect)),
  });
}

/**
 * Keeps the layout model of the current viewer state, rebuilt only when one of its
 * inputs changes (spreads identity, scale, rotation, dpr, page sizes). Returns null
 * while the page sizes are not loaded, after a failed load (pageSizeStore does not
 * retry it) and whenever createPageLayoutModel returns null.
 */
export function createPageLayoutProvider({
  viewer,
  documentSession,
  peekPageSizes,
  getDevicePixelRatio = () => globalThis.window?.devicePixelRatio,
}) {
  let inputs = null;
  let model = null;

  return function currentPageLayout() {
    const next = {
      spreads: viewer.spreads,
      pageSizes: peekPageSizes(documentSession.activeSessionId, documentSession.contentGeneration),
      scale: viewer.scale,
      rotation: viewer.rotation,
      devicePixelRatio: renderDevicePixelRatio(getDevicePixelRatio()),
      isBookMode: viewer.isBookMode,
    };
    if (!next.pageSizes) {
      inputs = null;
      model = null;
      return null;
    }
    if (!inputs || Object.keys(next).some((key) => next[key] !== inputs[key])) {
      inputs = next;
      model = createPageLayoutModel(next);
    }
    return model;
  };
}
