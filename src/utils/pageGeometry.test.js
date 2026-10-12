import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_PAGE_SIZE_PT,
  createPageSizeFallback,
  estimateRenderedPageCssSize,
  normalizePageSizes,
  renderDevicePixelRatio,
} from "./pageGeometry.js";

// Real PDFium bitmap sizes, pinned by worker_pdfium_integration.rs
// (page_sizes_and_render_sizes_match_geometry_contract).
const contract = JSON.parse(readFileSync(
  resolve(process.cwd(), "src-tauri/tests/fixtures/contracts/page-render-geometry-v1.json"),
  "utf8",
));
const pagesById = new Map(contract.pages.map((page) => [page.id, page]));

describe("estimateRenderedPageCssSize against real PDFium renders", () => {
  it("covers A4, Letter, landscape, /Rotate 90 and cropped pages at every scale, DPR and rotation", () => {
    const expected = contract.pages.length
      * contract.viewScales.length
      * contract.devicePixelRatios.length
      * contract.rotations.length;
    expect(contract.renders).toHaveLength(expected);
    expect(new Set(contract.renders.map((render) => render.page))).toEqual(new Set(pagesById.keys()));
  });

  it.each(contract.renders.map((render) => [
    `${render.page} scale=${render.viewScale} dpr=${render.devicePixelRatio} rot=${render.rotation}`,
    render,
  ]))("%s: placeholder is within 1px of the rendered page", (_label, render) => {
    const page = pagesById.get(render.page);
    const estimate = estimateRenderedPageCssSize({
      pageSize: { widthPt: page.widthPt, heightPt: page.heightPt },
      scale: render.viewScale,
      rotation: render.rotation,
      devicePixelRatio: render.devicePixelRatio,
    });
    const renderedCssHeight = render.bitmapHeight / render.devicePixelRatio;
    const renderedCssWidth = render.bitmapWidth / render.devicePixelRatio;
    expect(Math.abs(estimate.height - renderedCssHeight)).toBeLessThanOrEqual(1);
    expect(Math.abs(estimate.width - renderedCssWidth)).toBeLessThanOrEqual(1);
    // The model mirrors the worker's integer sizing, so it is exact in device pixels.
    expect(estimate.height * render.devicePixelRatio).toBe(render.bitmapHeight);
    expect(estimate.width * render.devicePixelRatio).toBe(render.bitmapWidth);
  });

  it("documents why the previous container-width formula was wrong", () => {
    // Old fallback: (containerWidth / aspect) * scale, with containerWidth = viewer column width.
    const a4 = pagesById.get("a4_portrait");
    const containerWidth = 1000;
    const oldEstimate = (containerWidth / (a4.widthPt / a4.heightPt)) * 1;
    const rendered = contract.renders.find((render) =>
      render.page === "a4_portrait" && render.viewScale === 1 && render.devicePixelRatio === 1 && render.rotation === 0);
    expect(oldEstimate - rendered.bitmapHeight).toBeGreaterThan(500);
  });
});

describe("page geometry helpers", () => {
  it("caps the render DPR the same way PdfPage requests renders", () => {
    expect(renderDevicePixelRatio(3)).toBe(2);
    expect(renderDevicePixelRatio(1.5)).toBe(1.5);
    expect(renderDevicePixelRatio(undefined)).toBe(1);
    expect(renderDevicePixelRatio(0)).toBe(1);
  });

  it("rejects unusable sizes and scales", () => {
    expect(estimateRenderedPageCssSize({ pageSize: null, scale: 1 })).toBeNull();
    expect(estimateRenderedPageCssSize({ pageSize: { widthPt: 0, heightPt: 0 }, scale: 1 })).toBeNull();
    expect(estimateRenderedPageCssSize({ pageSize: DEFAULT_PAGE_SIZE_PT, scale: 0 })).toBeNull();
  });

  it("normalizes worker page-size lists", () => {
    expect(normalizePageSizes(null)).toBeNull();
    expect(normalizePageSizes([
      { widthPt: 612, heightPt: 792 },
      { widthPt: 0, heightPt: 0 },
      { widthPt: "x", heightPt: 1 },
    ])).toEqual([{ widthPt: 612, heightPt: 792 }, null, null]);
  });

  it("resets the measured fallback size when the document changes", () => {
    const fallback = createPageSizeFallback();
    expect(fallback.get("doc-a:1")).toBe(DEFAULT_PAGE_SIZE_PT);
    fallback.remember("doc-a:1", { widthPt: 612, heightPt: 792 });
    expect(fallback.get("doc-a:1")).toEqual({ widthPt: 612, heightPt: 792 });
    expect(fallback.get("doc-a:2")).toBe(DEFAULT_PAGE_SIZE_PT);
    expect(fallback.get("doc-b:1")).toBe(DEFAULT_PAGE_SIZE_PT);
    fallback.remember("doc-b:1", { widthPt: 841.89, heightPt: 595.276 });
    expect(fallback.get("doc-a:1")).toBe(DEFAULT_PAGE_SIZE_PT);
    fallback.remember("doc-b:1", { widthPt: Number.NaN, heightPt: 1 });
    expect(fallback.get("doc-b:1")).toEqual({ widthPt: 841.89, heightPt: 595.276 });
  });
});
