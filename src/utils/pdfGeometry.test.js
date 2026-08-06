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

