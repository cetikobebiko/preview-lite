import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createPageLayoutModel, createPageLayoutProvider, VIEWER_ROW_LAYOUT } from "./pageLayoutModel.js";
import { estimateRenderedPageCssSize, renderDevicePixelRatio } from "../../utils/pageGeometry.js";
import { createPageSizeStore } from "../pageSizeStore.js";

const A4 = { widthPt: 595.276, heightPt: 841.89 };
const A3 = { widthPt: 841.89, heightPt: 1190.551 };
const LANDSCAPE = { widthPt: 841.89, heightPt: 595.276 };
const { minRowHeight, rowGap, pageGapContinuous } = VIEWER_ROW_LAYOUT;

const heightOf = (pageSize, scale, rotation = 0, devicePixelRatio = 1) =>
  estimateRenderedPageCssSize({ pageSize, scale, rotation, devicePixelRatio }).height;
const pitch = (pageHeight, pageGap = pageGapContinuous) => Math.max(minRowHeight, pageHeight + pageGap) + rowGap;

describe("createPageLayoutModel", () => {
  it("stacks continuous rows with the placeholder height PdfPage gets", () => {
    const model = createPageLayoutModel({ spreads: [[0], [1], [2]], pageSizes: [A4, A3, A4], scale: 1.5, devicePixelRatio: 2 });
    const a4 = heightOf(A4, 1.5, 0, 2);
    const a3 = heightOf(A3, 1.5, 0, 2);
    expect(model.rowCount).toBe(3);
    expect(model.pageRect(0)).toEqual({ physicalIndex: 0, top: 0, height: a4 });
    expect(model.pageRect(1)).toEqual({ physicalIndex: 1, top: pitch(a4), height: a3 });
    expect(model.pageRect(2).top).toBe(pitch(a4) + pitch(a3));
    expect(model.totalHeight).toBe(2 * pitch(a4) + pitch(a3));
  });

  it("applies the row minimum height to small pages", () => {
    const model = createPageLayoutModel({ spreads: [[0], [1]], pageSizes: [A4, A4], scale: 0.5 });
    expect(heightOf(A4, 0.5) + pageGapContinuous).toBeLessThan(minRowHeight);
    expect(model.rowTop(1)).toBe(minRowHeight + rowGap);
  });

  it("puts book spreads side by side, sized by the taller page and without a page gap", () => {
    const model = createPageLayoutModel({
      spreads: [[0], [1, 2]],
      pageSizes: [A4, A4, A3],
      scale: 1,
      isBookMode: true,
    });
    const a4 = heightOf(A4, 1);
    const a3 = heightOf(A3, 1);
    expect(model.pageRect(1).top).toBe(model.pageRect(2).top);
    expect(model.pageRect(1).top).toBe(pitch(a4, 0));
    expect(model.totalHeight).toBe(pitch(a4, 0) + pitch(a3, 0));
  });

  it("swaps width and height for a sideways viewer rotation", () => {
    const upright = createPageLayoutModel({ spreads: [[0]], pageSizes: [LANDSCAPE], scale: 2 });
    const sideways = createPageLayoutModel({ spreads: [[0]], pageSizes: [LANDSCAPE], scale: 2, rotation: 90 });
    expect(upright.pageRect(0).height).toBe(heightOf(LANDSCAPE, 2));
    expect(sideways.pageRect(0).height).toBe(heightOf(LANDSCAPE, 2, 90));
    expect(sideways.pageRect(0).height).toBeGreaterThan(upright.pageRect(0).height);
  });

  it("has no layout when a listed page has no usable worker size", () => {
    // PdfPage would guess these heights from state the model cannot see; no A4 guess here.
    expect(createPageLayoutModel({ spreads: [[0]], pageSizes: null, scale: 1 })).toBeNull();
    expect(createPageLayoutModel({ spreads: [[0], [1]], pageSizes: [A4, null], scale: 1 })).toBeNull();
    expect(createPageLayoutModel({ spreads: [[0], [1]], pageSizes: [A4, { widthPt: 0, heightPt: 10 }], scale: 1 })).toBeNull();
    // A shorter size list than the page list (e.g. a stale answer) is unknown too.
    expect(createPageLayoutModel({ spreads: [[0], [1], [2]], pageSizes: [A4, A4], scale: 1 })).toBeNull();
    // Book spread: one unknown page leaves the whole row height unknown.
    expect(createPageLayoutModel({ spreads: [[0, 1]], pageSizes: [A4, null], scale: 1, isBookMode: true })).toBeNull();
  });

  it("has no layout when a page height cannot be estimated at this scale", () => {
    expect(createPageLayoutModel({ spreads: [[0]], pageSizes: [A4], scale: 0 })).toBeNull();
    expect(createPageLayoutModel({ spreads: [[0]], pageSizes: [{ widthPt: 1, heightPt: 1 }], scale: 0.25 })).toBeNull();
  });

  it("ignores sizes of pages that are not listed", () => {
    const model = createPageLayoutModel({ spreads: [[1]], pageSizes: [null, A4], scale: 1 });
    expect(model.pageRect(1)).toEqual({ physicalIndex: 1, top: 0, height: heightOf(A4, 1) });
  });

  it("follows pageOrder: rows hold physical indices in display order", () => {
    const model = createPageLayoutModel({ spreads: [[2], [0], [1]], pageSizes: [A4, A4, A3], scale: 1 });
    expect(model.pageRect(2).top).toBe(0);
    // Physical page 2 (A3) comes first, then page 0 (A4).
    expect(model.pageRect(1).top).toBe(pitch(heightOf(A3, 1)) + pitch(heightOf(A4, 1)));
    expect(model.pageRect(7)).toBeNull();
    expect(model.pageRects().map((page) => page.physicalIndex)).toEqual([2, 0, 1]);
  });

  it("finds rows by offset, clamped to the list", () => {
    const model = createPageLayoutModel({ spreads: [[0], [1], [2]], pageSizes: [A4, A4, A4], scale: 1 });
    const rowPitch = pitch(heightOf(A4, 1));
    expect(model.rowAt(-50)).toBe(0);
    expect(model.rowAt(0)).toBe(0);
    expect(model.rowAt(rowPitch - 1)).toBe(0);
    expect(model.rowAt(rowPitch)).toBe(1);
    expect(model.rowAt(1e9)).toBe(2);
    expect(model.rowsInRange(rowPitch + 10, 100)).toEqual({ startRow: 1, endRow: 1 });
    expect(model.rowsInRange(rowPitch + 10, 100, rowPitch)).toEqual({ startRow: 0, endRow: 2 });
  });

  it("handles an empty list", () => {
    const model = createPageLayoutModel({ spreads: [], scale: 1 });
    expect(model.totalHeight).toBe(0);
    expect(model.rowAt(10)).toBe(-1);
    expect(model.rowsInRange(0, 100)).toEqual({ startRow: 0, endRow: -1 });
    expect(model.pageRects()).toEqual([]);
  });

  it("builds a 3000-page model quickly", () => {
    const spreads = Array.from({ length: 3000 }, (_, index) => [index]);
    const pageSizes = spreads.map((_, index) => (index % 7 === 0 ? A3 : A4));
    const started = performance.now();
    const model = createPageLayoutModel({ spreads, pageSizes, scale: 1.25, devicePixelRatio: 2 });
    expect(model.rowAt(model.pageRect(2999).top)).toBe(2999);
    expect(performance.now() - started).toBeLessThan(200);
  });
});

describe("VIEWER_ROW_LAYOUT mirrors the viewer CSS", () => {
  const read = (path) => readFileSync(resolve(process.cwd(), path), "utf8");
  const block = (source, selector) => {
    const start = source.indexOf(`${selector} {`);
    expect(start, `${selector} rule`).toBeGreaterThanOrEqual(0);
    return source.slice(start, source.indexOf("}", start));
  };

  it("matches row and page margins", () => {
    const shellRow = block(read("src/components/ViewerShell.svelte"), ".book-spread-row");
    expect(shellRow).toContain(`min-height: ${VIEWER_ROW_LAYOUT.minRowHeight}px;`);
    expect(shellRow).toContain(`margin-bottom: ${VIEWER_ROW_LAYOUT.rowGap}px;`);
    const appCss = read("src/app.css");
    expect(block(appCss, ".book-spread-row")).toContain(`margin-bottom: ${VIEWER_ROW_LAYOUT.rowGap}px;`);
    expect(block(appCss, ".book-spread-row.book-mode .pdf-page-container")).toContain(`margin-bottom: ${VIEWER_ROW_LAYOUT.pageGapBook};`);
    expect(block(read("src/components/PdfPage.svelte"), ".pdf-page-container"))
      .toContain(`margin-bottom: ${VIEWER_ROW_LAYOUT.pageGapContinuous}px;`);
  });
});

describe("createPageLayoutProvider", () => {
  function setup() {
    const viewer = { spreads: [[0], [1]], scale: 1, rotation: 0, isBookMode: false };
    const documentSession = { activeSessionId: "s1", contentGeneration: 3 };
    let sizes = null;
    const peekPageSizes = vi.fn(() => sizes);
    const current = createPageLayoutProvider({ viewer, documentSession, peekPageSizes, getDevicePixelRatio: () => 1 });
    return { viewer, peekPageSizes, current, setSizes: (value) => { sizes = value; } };
  }

  it("rebuilds only when an input changes", () => {
    const { viewer, current, setSizes } = setup();
    setSizes([A4, A4]);
    const first = current();
    expect(current()).toBe(first);
    viewer.scale = 2;
    const zoomed = current();
    expect(zoomed).not.toBe(first);
    expect(zoomed.pageRect(1).top).toBeGreaterThan(first.pageRect(1).top);
    viewer.spreads = [[1], [0]];
    expect(current().pageRect(1).top).toBe(0);
  });

  it("has no layout until worker sizes are loaded, then picks them up", () => {
    const { current, peekPageSizes, setSizes } = setup();
    expect(current()).toBeNull();
    setSizes([A3, A3]);
    expect(current().pageRect(0).height).toBe(heightOf(A3, 1));
    expect(peekPageSizes).toHaveBeenLastCalledWith("s1", 3);
  });

  it("drops the layout when the sizes are gone again (new session or generation)", () => {
    const { current, setSizes } = setup();
    setSizes([A4, A4]);
    expect(current()).not.toBeNull();
    setSizes(null);
    expect(current()).toBeNull();
  });

  it("stays without a layout after the page-size load fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const store = createPageSizeStore({ load: vi.fn(async () => { throw new Error("worker down"); }) });
    const viewer = { spreads: [[0], [1]], scale: 1, rotation: 0, isBookMode: false };
    const documentSession = { activeSessionId: "s1", contentGeneration: 1 };
    const current = createPageLayoutProvider({ viewer, documentSession, peekPageSizes: store.peekAll, getDevicePixelRatio: () => 1 });
    expect(await store.request("s1", 1)).toBeNull();
    // Same key: the store does not retry, so the model must not invent sizes either.
    expect(await store.request("s1", 1)).toBeNull();
    expect(current()).toBeNull();
    warn.mockRestore();
  });
});

/**
 * PdfPage.svelte placeholder height when it has a worker size:
 *   estimateRenderedPageCssSize({ pageSize, scale, rotation: normalizedRotation,
 *     devicePixelRatio: currentDevicePixelRatio() }).height
 * with normalizedRotation = ((rotation % 360) + 360) % 360 and
 * currentDevicePixelRatio() = renderDevicePixelRatio(window.devicePixelRatio).
 */
function pdfPagePlaceholderHeight({ pageSize, scale, rotation, devicePixelRatio }) {
  return estimateRenderedPageCssSize({
    pageSize,
    scale,
    rotation: ((rotation % 360) + 360) % 360,
    devicePixelRatio: renderDevicePixelRatio(devicePixelRatio),
  }).height;
}

describe("layout model and PdfPage agree on page heights", () => {
  const LETTER = { widthPt: 612, heightPt: 792 };
  const ODD = { widthPt: 612.37, heightPt: 1008.71 };
  const sizes = [A4, A3, LANDSCAPE, LETTER, ODD];
  const scales = [0.25, 0.5, 1, 1.33, 2.5, 5];
  const rotations = [0, 90, 180, 270, -90, 360, 450];
  const ratios = [1, 1.25, 1.5, 2, 3, 0, Number.NaN, undefined];

  it("for every size, scale, rotation and device pixel ratio, continuous and book mode", () => {
    const pageOrder = sizes.map((_, index) => index);
    const mismatches = [];
    for (const isBookMode of [false, true]) {
      const spreads = isBookMode ? [[0], [1, 2], [3, 4]] : pageOrder.map((index) => [index]);
      const viewer = { spreads, scale: 1, rotation: 0, isBookMode };
      let ratio = 1;
      const current = createPageLayoutProvider({
        viewer,
        documentSession: { activeSessionId: "s1", contentGeneration: 1 },
        peekPageSizes: () => sizes,
        getDevicePixelRatio: () => ratio,
      });
      for (const scale of scales) {
        for (const rotation of rotations) {
          for (const devicePixelRatio of ratios) {
            Object.assign(viewer, { scale, rotation });
            ratio = devicePixelRatio;
            const model = current();
            for (const physicalIndex of pageOrder) {
              const expected = pdfPagePlaceholderHeight({ pageSize: sizes[physicalIndex], scale, rotation, devicePixelRatio });
              const actual = model.pageRect(physicalIndex).height;
              if (actual !== expected) mismatches.push({ isBookMode, scale, rotation, devicePixelRatio, physicalIndex, actual, expected });
            }
          }
        }
      }
    }
    expect(mismatches).toEqual([]);
  });

  it("stacks the same heights PdfPage boxes into rows", () => {
    const pageSizes = [A4, A3, LANDSCAPE];
    const model = createPageLayoutModel({ spreads: [[0], [1], [2]], pageSizes, scale: 1.33, rotation: -90, devicePixelRatio: 1.5 });
    const heights = pageSizes.map((pageSize) =>
      pdfPagePlaceholderHeight({ pageSize, scale: 1.33, rotation: -90, devicePixelRatio: 1.5 }));
    expect(model.pageRect(2).top).toBe(pitch(heights[0]) + pitch(heights[1]));
    expect(model.totalHeight).toBe(heights.reduce((sum, height) => sum + pitch(height), 0));
  });
});
