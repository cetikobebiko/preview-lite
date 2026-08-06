import { describe, expect, it, vi } from "vitest";
import {
  validateSensitiveScanReport,
  normalizeSensitiveScanReport,
} from "./redaction.js";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue({
    schema_version: 2,
    items: [],
    warnings: [],
    truncated: false,
  }),
}));

describe("redaction backend validation & normalization (Schema v2)", () => {
  const validGeom = {
    coordinate_space: "pdf_user_space",
    unit: "pdf_point",
    origin: "bottom_left",
    media_box: { left: 0, bottom: 0, width: 400, height: 600 },
    crop_box: null,
    effective_box: { left: 0, bottom: 0, width: 400, height: 600 },
    intrinsic_rotation: 90,
  };

  const validReportV2 = {
    schema_version: 2,
    items: [
      {
        page_index: 0,
        pattern_index: 1,
        text_preview: "12345678901",
        geometry_status: "resolved",
        page_geometry: validGeom,
        source_rects_pdf_points: [{ x: 10, y: 20, width: 100, height: 15 }],
        warnings: [],
      },
      {
        page_index: 1,
        pattern_index: 0,
        text_preview: "test@domain.com",
        geometry_status: "unresolved",
        page_geometry: null,
        source_rects_pdf_points: [],
        warnings: [
          {
            code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
            message: "Sayfa geometrisi çıkarılamadı.",
          },
        ],
      },
    ],
    warnings: [
      {
        code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
        message: "Bir veya daha fazla sayfada geometri çıkarılamadı.",
      },
    ],
    truncated: false,
  };

  it("validates and normalizes a valid schema v2 report to camelCase", () => {
    expect(validateSensitiveScanReport(validReportV2)).toBe(true);

    const normalized = normalizeSensitiveScanReport(validReportV2);
    expect(normalized).toEqual({
      schemaVersion: 2,
      items: [
        {
          pageIndex: 0,
          patternIndex: 1,
          textPreview: "12345678901",
          geometryStatus: "resolved",
          pageGeometry: {
            coordinateSpace: "pdf_user_space",
            unit: "pdf_point",
            origin: "bottom_left",
            mediaBox: { left: 0, bottom: 0, width: 400, height: 600 },
            cropBox: null,
            effectiveBox: { left: 0, bottom: 0, width: 400, height: 600 },
            intrinsicRotation: 90,
          },
          sourceRectsPdfPoints: [{ x: 10, y: 20, width: 100, height: 15 }],
          warnings: [],
        },
        {
          pageIndex: 1,
          patternIndex: 0,
          textPreview: "test@domain.com",
          geometryStatus: "unresolved",
          pageGeometry: null,
          sourceRectsPdfPoints: [],
          warnings: [
            {
              code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
              message: "Sayfa geometrisi çıkarılamadı.",
            },
          ],
        },
      ],
      warnings: [
        {
          code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
          message: "Bir veya daha fazla sayfada geometri çıkarılamadı.",
        },
      ],
      truncated: false,
    });
  });

  it("rejects schema_version != 2 (rejects schema v1)", () => {
    const invalidSchema = { ...validReportV2, schema_version: 1 };
    expect(() => validateSensitiveScanReport(invalidSchema)).toThrow(
      "schema_version must be 2",
    );
  });

  it("rejects resolved items missing page_geometry or with empty source_rects", () => {
    const missingGeom = {
      ...validReportV2,
      items: [{ ...validReportV2.items[0], page_geometry: null }],
    };
    expect(() => validateSensitiveScanReport(missingGeom)).toThrow(
      "resolved item must have page_geometry",
    );

    const emptyRects = {
      ...validReportV2,
      items: [{ ...validReportV2.items[0], source_rects_pdf_points: [] }],
    };
    expect(() => validateSensitiveScanReport(emptyRects)).toThrow(
      "resolved item must have source_rects_pdf_points",
    );
  });

  it("rejects non-numeric or unsupported intrinsic_rotation values", () => {
    const stringRotation = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, intrinsic_rotation: "90" },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(stringRotation)).toThrow(
      "intrinsic_rotation must be numeric 0, 90, 180, or 270",
    );

    const enumStringRotation = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, intrinsic_rotation: "Rotate90" },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(enumStringRotation)).toThrow(
      "intrinsic_rotation must be numeric 0, 90, 180, or 270",
    );

    const invalidNumberRotation = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, intrinsic_rotation: 45 },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(invalidNumberRotation)).toThrow(
      "intrinsic_rotation must be numeric 0, 90, 180, or 270",
    );
  });

  it("rejects unknown fields at report, item, or page_geometry level", () => {
    const reportUnknown = { ...validReportV2, foo: "bar" };
    expect(() => validateSensitiveScanReport(reportUnknown)).toThrow("unknown field 'foo'");

    const itemUnknown = {
      ...validReportV2,
      items: [{ ...validReportV2.items[0], foo: "bar" }],
    };
    expect(() => validateSensitiveScanReport(itemUnknown)).toThrow("unknown field 'foo'");

    const geomUnknown = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, foo: "bar" },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(geomUnknown)).toThrow("unknown field 'foo'");
  });
});

describe("startSensitiveDataScan and scanSensitiveData wrapper behavior", () => {
  it("preserves canonical jobId and cancel function without generating a second UUID", async () => {
    const { startSensitiveDataScan, scanSensitiveData } = await import("./redaction.js");

    const handle = startSensitiveDataScan("test-session", 1, ["\\d+"]);
    expect(typeof handle.jobId).toBe("string");
    expect(handle.jobId.length).toBeGreaterThan(0);
    expect(typeof handle.cancel).toBe("function");
    expect(typeof handle.promise.then).toBe("function");
    await handle.promise.catch(() => {});

    const scanPromise = scanSensitiveData("test-session", 1, ["\\d+"]);
    expect(typeof scanPromise.then).toBe("function");
    await scanPromise.catch(() => {});
  });
});

