import { describe, expect, it } from "vitest";
import {
  validatePageGeometry,
  pdfPointRectToNormalizedTopLeft,
  normalizedTopLeftRectToPdfPointRect,
  clampNormalizedRect,
  rotateNormalizedRectForViewport,
  composeRenderedPageRotation,
  buildLogicalHighlightsByPage,
  selectViewportHighlights,
  clientPointToLayerNormalized,
  normalizedRectBetween,
  fixedSizeNormalizedRect,
} from "./pdfGeometry.js";
import { normalizeSensitiveScanReport } from "../services/backend/redaction.js";

describe("pdfGeometry module", () => {
  const standardGeometry = {
    coordinateSpace: "pdf_user_space",
    unit: "pdf_point",
    origin: "bottom_left",
    mediaBox: { left: 0, bottom: 0, width: 400, height: 600 },
    cropBox: null,
    effectiveBox: { left: 0, bottom: 0, width: 400, height: 600 },
    intrinsicRotation: 0,
  };

  const nonzeroGeometry = {
    coordinateSpace: "pdf_user_space",
    unit: "pdf_point",
    origin: "bottom_left",
    mediaBox: { left: 0, bottom: 0, width: 600, height: 800 },
    cropBox: { left: 100, bottom: 150, width: 400, height: 500 },
    effectiveBox: { left: 100, bottom: 150, width: 400, height: 500 },
    intrinsicRotation: 90,
  };

  const negativeGeometry = {
    coordinateSpace: "pdf_user_space",
    unit: "pdf_point",
    origin: "bottom_left",
    mediaBox: { left: -100, bottom: -100, width: 600, height: 800 },
    cropBox: { left: -50, bottom: -40, width: 500, height: 700 },
    effectiveBox: { left: -50, bottom: -40, width: 500, height: 700 },
    intrinsicRotation: 180,
  };

  describe("validatePageGeometry", () => {
    it("accepts valid page geometry objects", () => {
      expect(validatePageGeometry(standardGeometry)).toBe(true);
      expect(validatePageGeometry(nonzeroGeometry)).toBe(true);
      expect(validatePageGeometry(negativeGeometry)).toBe(true);
    });

    it("integrates directly with normalizeSensitiveScanReport output", () => {
      const validRawSchemaV2 = {
        schema_version: 2,
        truncated: false,
        warnings: [],
        items: [
          {
            page_index: 0,
            pattern_index: 0,
            text_preview: "MATCH123",
            geometry_status: "resolved",
            page_geometry: {
              coordinate_space: "pdf_user_space",
              unit: "pdf_point",
              origin: "bottom_left",
              media_box: { left: 0, bottom: 0, width: 400, height: 600 },
              crop_box: null,
              effective_box: { left: 0, bottom: 0, width: 400, height: 600 },
              intrinsic_rotation: 0,
            },
            source_rects_pdf_points: [{ x: 50, y: 100, width: 200, height: 30 }],
            warnings: [],
          },
        ],
      };

      const normalized = normalizeSensitiveScanReport(validRawSchemaV2);
      const geometry = normalized.items[0].pageGeometry;

      expect(validatePageGeometry(geometry)).toBe(true);
      expect(
        pdfPointRectToNormalizedTopLeft(
          normalized.items[0].sourceRectsPdfPoints[0],
          geometry,
        ),
      ).not.toBeNull();
    });

    it("rejects invalid page geometry fields or unknown keys", () => {
      expect(validatePageGeometry(null)).toBe(false);
      expect(validatePageGeometry({})).toBe(false);
      expect(
        validatePageGeometry({ ...standardGeometry, coordinateSpace: "screen" }),
      ).toBe(false);
      expect(validatePageGeometry({ ...standardGeometry, unit: "pixel" })).toBe(false);
      expect(validatePageGeometry({ ...standardGeometry, origin: "top_left" })).toBe(false);
      expect(
        validatePageGeometry({ ...standardGeometry, intrinsicRotation: 45 }),
      ).toBe(false);
      expect(
        validatePageGeometry({ ...standardGeometry, unknown_field: "value" }),
      ).toBe(false);
      expect(
        validatePageGeometry({
          ...standardGeometry,
          effectiveBox: { left: 0, bottom: 0, width: 0, height: 100 },
        }),
      ).toBe(false);
    });

    it("rejects non-numeric types (numeric string, boolean, empty string) without coercion", () => {
      expect(
        validatePageGeometry({
          ...standardGeometry,
          effectiveBox: { left: "0", bottom: 0, width: 400, height: 600 },
        }),
      ).toBe(false);
      expect(
        validatePageGeometry({
          ...standardGeometry,
          effectiveBox: { left: 0, bottom: true, width: 400, height: 600 },
        }),
      ).toBe(false);
      expect(
        validatePageGeometry({
          ...standardGeometry,
          effectiveBox: { left: 0, bottom: 0, width: "", height: 600 },
        }),
      ).toBe(false);
      expect(
        validatePageGeometry({
          ...standardGeometry,
          mediaBox: { left: 0, bottom: 0, width: 400, height: "600" },
        }),
      ).toBe(false);
    });
  });

  describe("pdfPointRectToNormalizedTopLeft", () => {
    it("converts four corners and center on zero-origin page correctly", () => {
      const topLeft = pdfPointRectToNormalizedTopLeft(
        { x: 0, y: 550, width: 40, height: 50 },
        standardGeometry,
      );
      expect(topLeft).toEqual({
        space: "normalized_page",
        x: 0,
        y: 0,
        width: 0.1,
        height: 0.083333333333,
      });

      const bottomRight = pdfPointRectToNormalizedTopLeft(
        { x: 360, y: 0, width: 40, height: 50 },
        standardGeometry,
      );
      expect(bottomRight).toEqual({
        space: "normalized_page",
        x: 0.9,
        y: 0.916666666667,
        width: 0.1,
        height: 0.083333333333,
      });

      const center = pdfPointRectToNormalizedTopLeft(
        { x: 150, y: 250, width: 100, height: 100 },
        standardGeometry,
      );
      expect(center).toEqual({
        space: "normalized_page",
        x: 0.375,
        y: 0.416666666667,
        width: 0.25,
        height: 0.166666666667,
      });
    });

    it("handles non-zero and negative box origins correctly", () => {
      // Effective box: left: 100, bottom: 150, width: 400, height: 500 (pageTop = 650)
      const topRect = pdfPointRectToNormalizedTopLeft(
        { x: 100, y: 600, width: 40, height: 50 },
        nonzeroGeometry,
      );
      expect(topRect).toEqual({
        space: "normalized_page",
        x: 0,
        y: 0,
        width: 0.1,
        height: 0.1,
      });

      // Negative origin: left: -50, bottom: -40, width: 500, height: 700 (pageTop = 660)
      const negRect = pdfPointRectToNormalizedTopLeft(
        { x: -50, y: 610, width: 50, height: 50 },
        negativeGeometry,
      );
      expect(negRect).toEqual({
        space: "normalized_page",
        x: 0,
        y: 0,
        width: 0.1,
        height: 0.071428571429,
      });
    });

    it("returns raw unclamped normalized rect for out-of-bounds rects", () => {
      const outRect = pdfPointRectToNormalizedTopLeft(
        { x: -40, y: 650, width: 100, height: 100 },
        standardGeometry,
      );
      expect(outRect).toEqual({
        space: "normalized_page",
        x: -0.1,
        y: -0.25,
        width: 0.25,
        height: 0.166666666667,
      });
    });

    it("rejects invalid, zero-area, negative size, or NaN inputs", () => {
      expect(
        pdfPointRectToNormalizedTopLeft({ x: 0, y: 0, width: 0, height: 10 }, standardGeometry),
      ).toBeNull();
      expect(
        pdfPointRectToNormalizedTopLeft({ x: 0, y: 0, width: 10, height: -5 }, standardGeometry),
      ).toBeNull();
      expect(
        pdfPointRectToNormalizedTopLeft({ x: NaN, y: 0, width: 10, height: 10 }, standardGeometry),
      ).toBeNull();
      expect(
        pdfPointRectToNormalizedTopLeft({ x: 0, y: Infinity, width: 10, height: 10 }, standardGeometry),
      ).toBeNull();
    });
  });

  describe("normalizedTopLeftRectToPdfPointRect & Round-Trip", () => {
    it("accurately converts normalized rect back to PDF point rect", () => {
      const normalized = { space: "normalized_page", x: 0.2, y: 0.3, width: 0.4, height: 0.5 };
      const pdfRect = normalizedTopLeftRectToPdfPointRect(normalized, nonzeroGeometry);

      // Effective box: left: 100, bottom: 150, width: 400, height: 500 (pageTop = 650)
      // pdfX = 100 + 0.2 * 400 = 180
      // pdfY = 650 - (0.3 + 0.5) * 500 = 650 - 400 = 250
      // pdfWidth = 0.4 * 400 = 160
      // pdfHeight = 0.5 * 500 = 250
      expect(pdfRect).toEqual({
        x: 180,
        y: 250,
        width: 160,
        height: 250,
      });
    });

    it("maintains round-trip accuracy (< 1e-5 delta) for valid in-bounds rects", () => {
      const originalPdfRect = { x: 120, y: 220, width: 85, height: 110 };
      const normalized = pdfPointRectToNormalizedTopLeft(originalPdfRect, negativeGeometry);
      const restored = normalizedTopLeftRectToPdfPointRect(normalized, negativeGeometry);

      expect(Math.abs(restored.x - originalPdfRect.x)).toBeLessThan(1e-5);
      expect(Math.abs(restored.y - originalPdfRect.y)).toBeLessThan(1e-5);
      expect(Math.abs(restored.width - originalPdfRect.width)).toBeLessThan(1e-5);
      expect(Math.abs(restored.height - originalPdfRect.height)).toBeLessThan(1e-5);
    });
  });

  describe("clampNormalizedRect", () => {
    it("leaves fully inside rects untouched", () => {
      const rect = { space: "normalized_page", x: 0.1, y: 0.2, width: 0.3, height: 0.4 };
      expect(clampNormalizedRect(rect)).toEqual(rect);
    });

    it("clamps partially out-of-bounds rects to [0, 1] edges and recomputes size", () => {
      const partial = { space: "normalized_page", x: -0.1, y: 0.8, width: 0.5, height: 0.4 };
      const clamped = clampNormalizedRect(partial);
      expect(clamped).toEqual({
        space: "normalized_page",
        x: 0,
        y: 0.8,
        width: 0.4,
        height: 0.2,
      });
    });

    it("returns null for completely out-of-bounds or zero-area rects", () => {
      expect(clampNormalizedRect({ x: 1.2, y: 0.1, width: 0.3, height: 0.4 })).toBeNull();
      expect(clampNormalizedRect({ x: -0.5, y: 0.1, width: 0.2, height: 0.4 })).toBeNull();
      expect(clampNormalizedRect({ x: 0.1, y: 1.5, width: 0.3, height: 0.4 })).toBeNull();
    });
  });

  describe("rotateNormalizedRectForViewport & composeRenderedPageRotation", () => {
    const rect = { space: "normalized_page", x: 0.1, y: 0.2, width: 0.3, height: 0.4 };

    it("rotates rects correctly for 0, 90, 180, and 270 viewport rotations", () => {
      expect(rotateNormalizedRectForViewport(rect, 0)).toEqual({
        space: "normalized_page",
        x: 0.1,
        y: 0.2,
        width: 0.3,
        height: 0.4,
      });

      expect(rotateNormalizedRectForViewport(rect, 90)).toEqual({
        space: "normalized_page",
        x: 0.4, // 1 - 0.2 - 0.4
        y: 0.1,
        width: 0.4,
        height: 0.3,
      });

      expect(rotateNormalizedRectForViewport(rect, 180)).toEqual({
        space: "normalized_page",
        x: 0.6, // 1 - 0.1 - 0.3
        y: 0.4, // 1 - 0.2 - 0.4
        width: 0.3,
        height: 0.4,
      });

      expect(rotateNormalizedRectForViewport(rect, 270)).toEqual({
        space: "normalized_page",
        x: 0.2,
        y: 0.6, // 1 - 0.1 - 0.3
        width: 0.4,
        height: 0.3,
      });
    });

    it("composes intrinsic and viewer rotations correctly", () => {
      expect(composeRenderedPageRotation(90, 90)).toBe(180);
      expect(composeRenderedPageRotation(270, 90)).toBe(0);
      expect(composeRenderedPageRotation(180, 270)).toBe(90);
    });
  });

  describe("buildLogicalHighlightsByPage and selectViewportHighlights", () => {
    const report = {
      schemaVersion: 2,
      truncated: false,
      warnings: [],
      items: [
        {
          pageIndex: 0,
          patternIndex: 0,
          textPreview: "ABC",
          geometryStatus: "resolved",
          pageGeometry: standardGeometry,
          sourceRectsPdfPoints: [
            { x: 0, y: 300, width: 200, height: 300 },
            { x: 200, y: 0, width: 200, height: 300 },
          ],
          warnings: [],
        },
        {
          pageIndex: 0,
          patternIndex: 1,
          textPreview: "DEF",
          geometryStatus: "unresolved",
          pageGeometry: null,
          sourceRectsPdfPoints: [],
          warnings: [],
        },
      ],
    };

    it("builds logical highlights for resolved items and ignores unresolved items", () => {
      const logicals = buildLogicalHighlightsByPage(report);
      expect(Object.keys(logicals)).toEqual(["0"]);
      expect(logicals[0]).toHaveLength(2);
      expect(logicals[0][0]).toMatchObject({ space: "normalized_page", x: 0, y: 0, width: 0.5, height: 0.5, intrinsicRotation: 0 });
      expect(logicals[0][1]).toMatchObject({ space: "normalized_page", x: 0.5, y: 0.5, width: 0.5, height: 0.5, intrinsicRotation: 0 });
    });

    it("selects viewport highlights with viewer rotation and clamps out of bounds rects", () => {
      const logicals = buildLogicalHighlightsByPage(report);
      const viewport0 = selectViewportHighlights(logicals, 0, 0);
      expect(viewport0).toHaveLength(2);
      expect(viewport0[0]).toEqual({ space: "normalized_page", x: 0, y: 0, width: 0.5, height: 0.5 });

      const viewport90 = selectViewportHighlights(logicals, 0, 90);
      expect(viewport90).toHaveLength(2);
      expect(viewport90[0]).toEqual({ space: "normalized_page", x: 0.5, y: 0, width: 0.5, height: 0.5 });

      // Out of bounds logical rect
      const outOfBoundsLogicals = {
        0: [{ space: "normalized_page", x: 1.5, y: 1.5, width: 0.5, height: 0.5, intrinsicRotation: 0 }],
      };
      expect(selectViewportHighlights(outOfBoundsLogicals, 0, 0)).toEqual([]);
    });
  });
});


// 2D afin matris [a, b, c, d, e, f]: (x, y) → (a·x + c·y + e, b·x + d·y + f).
const multiply = (m, n) => [
  m[0] * n[0] + m[2] * n[1],
  m[1] * n[0] + m[3] * n[1],
  m[0] * n[2] + m[2] * n[3],
  m[1] * n[2] + m[3] * n[3],
  m[0] * n[4] + m[2] * n[5] + m[4],
  m[1] * n[4] + m[3] * n[5] + m[5],
];
const translate = (x, y) => [1, 0, 0, 1, x, y];
const scaleBy = (s) => [s, 0, 0, s, 0, 0];
const rotateDeg = (deg) => {
  const r = (deg * Math.PI) / 180;
  return [Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0];
};
// CSS `transform` + `transform-origin`: T(o) · M · T(−o).
const aroundOrigin = (matrix, ox, oy) =>
  multiply(translate(ox, oy), multiply(matrix, translate(-ox, -oy)));
const apply = (m, [x, y]) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];

/**
 * PdfPage/ViewerShell zincirinin modeli: AnnotationLayer (ow×oh) → döndürülmüş kap
 * (rotate θ, merkez) → overlay-plane (translate −50%, display-surface ortasında) →
 * display-surface (scale cssScale, sol-üst) → canvas-container (scale visualScale,
 * transform-origin 50% zoomOriginY) → sayfa ofseti.
 */
function viewerChain({ ow, oh, rotation, cssScale, visualScale, zoomOriginY, offset }) {
  const sideways = rotation === 90 || rotation === 270;
  const surfaceWidth = sideways ? oh : ow;
  const surfaceHeight = sideways ? ow : oh;
  const containerWidth = surfaceWidth * cssScale + 400;
  const layerToSurface = multiply(
    translate(surfaceWidth / 2 - ow / 2, surfaceHeight / 2 - oh / 2),
    aroundOrigin(rotateDeg(rotation), ow / 2, oh / 2),
  );
  const surfaceToContainer = multiply(translate(200, 30), scaleBy(cssScale));
  const containerToClient = multiply(
    translate(offset[0], offset[1]),
    aroundOrigin(scaleBy(visualScale), containerWidth / 2, zoomOriginY),
  );
  const matrix = multiply(containerToClient, multiply(surfaceToContainer, layerToSurface));
  const corners = [[0, 0], [ow, 0], [0, oh], [ow, oh]].map((p) => apply(matrix, p));
  const xs = corners.map(([x]) => x);
  const ys = corners.map(([, y]) => y);
  const boundingRect = {
    left: Math.min(...xs),
    top: Math.min(...ys),
    width: Math.max(...xs) - Math.min(...xs),
    height: Math.max(...ys) - Math.min(...ys),
  };
  return {
    toClient: (u, v) => apply(matrix, [u * ow, v * oh]),
    frame: { boundingRect, localWidth: ow, localHeight: oh, rotation },
  };
}

describe("clientPointToLayerNormalized", () => {
  // Kare olmayan katman: 300×500 yerel CSS px.
  const ow = 300;
  const oh = 500;

  it.each([
    // CSS rotate saat yönündedir: yerel sol-üst köşe 90°'de sınır kutusunun sağ-üstüne gider.
    [0, { tl: ["left", "top"], tr: ["right", "top"], bl: ["left", "bottom"] }],
    [90, { tl: ["right", "top"], tr: ["right", "bottom"], bl: ["left", "top"] }],
    [180, { tl: ["right", "bottom"], tr: ["left", "bottom"], bl: ["right", "top"] }],
    [270, { tl: ["left", "bottom"], tr: ["left", "top"], bl: ["right", "bottom"] }],
  ])("maps bounding-box corners back to local corners at %i°", (rotation, corners) => {
    const sideways = rotation % 180 !== 0;
    const s = 2;
    const boundingRect = {
      left: 40,
      top: 70,
      width: (sideways ? oh : ow) * s,
      height: (sideways ? ow : oh) * s,
    };
    const edge = {
      left: boundingRect.left,
      right: boundingRect.left + boundingRect.width,
      top: boundingRect.top,
      bottom: boundingRect.top + boundingRect.height,
    };
    const frame = { boundingRect, localWidth: ow, localHeight: oh, rotation };
    for (const [corner, expected] of [
      ["tl", { x: 0, y: 0 }],
      ["tr", { x: 1, y: 0 }],
      ["bl", { x: 0, y: 1 }],
    ]) {
      const [horizontal, vertical] = corners[corner];
      const local = clientPointToLayerNormalized(edge[horizontal], edge[vertical], frame);
      expect(local.x).toBeCloseTo(expected.x, 9);
      expect(local.y).toBeCloseTo(expected.y, 9);
    }
  });

  const rotations = [0, 90, 180, 270];
  const scales = [
    { cssScale: 1, visualScale: 1 },
    { cssScale: 1.5, visualScale: 1 },
    { cssScale: 0.75, visualScale: 2 },
    { cssScale: 2.25, visualScale: 0.6 },
  ];
  const points = [
    [0, 0],
    [1, 1],
    [0.1, 0.8],
    [0.73, 0.21],
  ];

  for (const rotation of rotations) {
    for (const { cssScale, visualScale } of scales) {
      it(`inverts the viewer transform chain at ${rotation}°, cssScale ${cssScale}, visualScale ${visualScale}`, () => {
        const chain = viewerChain({
          ow,
          oh,
          rotation,
          cssScale,
          visualScale,
          zoomOriginY: 180,
          offset: [17, -260],
        });
        for (const [u, v] of points) {
          const [clientX, clientY] = chain.toClient(u, v);
          const local = clientPointToLayerNormalized(clientX, clientY, chain.frame);
          expect(local.x).toBeCloseTo(u, 9);
          expect(local.y).toBeCloseTo(v, 9);
        }
      });
    }
  }

  it("reports points outside the layer without clamping", () => {
    const chain = viewerChain({
      ow,
      oh,
      rotation: 90,
      cssScale: 1,
      visualScale: 1,
      zoomOriginY: 0,
      offset: [0, 0],
    });
    const [clientX, clientY] = chain.toClient(1.2, -0.1);
    const local = clientPointToLayerNormalized(clientX, clientY, chain.frame);
    expect(local.x).toBeCloseTo(1.2, 9);
    expect(local.y).toBeCloseTo(-0.1, 9);
  });

  it("rejects frames without a measurable layout", () => {
    const boundingRect = { left: 0, top: 0, width: 100, height: 100 };
    expect(
      clientPointToLayerNormalized(1, 1, { boundingRect, localWidth: 0, localHeight: 100 }),
    ).toBeNull();
    expect(
      clientPointToLayerNormalized(1, 1, {
        boundingRect: { ...boundingRect, width: 0 },
        localWidth: 100,
        localHeight: 100,
      }),
    ).toBeNull();
    expect(
      clientPointToLayerNormalized(Number.NaN, 1, {
        boundingRect,
        localWidth: 100,
        localHeight: 100,
      }),
    ).toBeNull();
  });
});

describe("normalized annotation rect helpers", () => {
  it("builds a clamped rect from two points in any drag direction", () => {
    expect(normalizedRectBetween({ x: 0.8, y: 0.6 }, { x: 0.2, y: 0.1 })).toEqual({
      x: 0.2,
      y: 0.1,
      width: 0.6000000000000001,
      height: 0.5,
    });
    expect(normalizedRectBetween({ x: 0.5, y: 0.5 }, { x: 1.4, y: -0.3 })).toEqual({
      x: 0.5,
      y: 0,
      width: 0.5,
      height: 0.5,
    });
  });

  it("sizes a fixed note in page points and keeps it on the page", () => {
    // Görüntü uzayında 800×600 pt sayfa (ör. /Rotate 90 uygulanmış 600×800 MediaBox).
    expect(fixedSizeNormalizedRect({ x: 0.5, y: 0.5 }, 24, 800, 600)).toEqual({
      x: 0.5,
      y: 0.5,
      width: 0.03,
      height: 0.04,
    });
    const atEdge = fixedSizeNormalizedRect({ x: 0.99, y: 1 }, 24, 800, 600);
    expect(atEdge.x + atEdge.width).toBeCloseTo(1, 12);
    expect(atEdge.y + atEdge.height).toBeCloseTo(1, 12);
    expect(fixedSizeNormalizedRect({ x: 0, y: 0 }, 24, 0, 600)).toBeNull();
  });
});
