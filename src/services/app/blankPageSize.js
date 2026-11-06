import { getPageSizes } from "../backend/viewer.js";
import { normalizePageSizes } from "../../utils/pageGeometry.js";

/** A4 portrait in whole points, the same default as `insertBlankPage` in pdfState.js. */
export const A4_BLANK_PAGE_SIZE = Object.freeze({ width: 595, height: 842 });

/** `MAX_PAGE_DIMENSION_POINTS` in manipulation.rs; the backend rejects larger sides. */
const MAX_BLANK_PAGE_SIDE_PT = 14_400;

/**
 * MediaBox width/height for a blank page placed next to page `neighborIndex`.
 *
 * `get_session_page_sizes` reports `FPDF_GetPageSizeByIndexF`, which PDFium computes in
 * `CPDF_Page::UpdateDimensions`: the CropBox clipped to the MediaBox, with width and height
 * swapped when the page's /Rotate is 90 or 270. So these are the sizes the viewer shows.
 * `FPDFPage_New` creates a page whose MediaBox is [0 0 width height] with no CropBox and no
 * /Rotate, so its displayed size is exactly width x height. Passing the neighbour's displayed
 * width and height unchanged therefore gives the new page the neighbour's on-screen size and
 * orientation (a rotated portrait page that shows as landscape gets a landscape blank page),
 * without copying the neighbour's /Rotate or CropBox.
 *
 * Falls back to A4 portrait when the size is unknown or outside what the backend accepts.
 */
export function blankPageSizeFromNeighbor(sizes, neighborIndex) {
  const size = Array.isArray(sizes) && Number.isInteger(neighborIndex) ? sizes[neighborIndex] : null;
  const width = Number(size?.widthPt);
  const height = Number(size?.heightPt);
  const usable = (side) => Number.isFinite(side) && side > 0 && side <= MAX_BLANK_PAGE_SIDE_PT;
  return usable(width) && usable(height) ? { width, height } : { ...A4_BLANK_PAGE_SIZE };
}

/** Loads the document's page sizes and applies `blankPageSizeFromNeighbor`; A4 on any failure. */
export async function resolveBlankPageSize(sessionId, contentGeneration, neighborIndex, { loadSizes = getPageSizes } = {}) {
  try {
    return blankPageSizeFromNeighbor(normalizePageSizes(await loadSizes(sessionId, contentGeneration)), neighborIndex);
  } catch (error) {
    console.warn("Sayfa boyutu alınamadı; boş sayfa A4 olarak eklenecek:", error);
    return { ...A4_BLANK_PAGE_SIZE };
  }
}
