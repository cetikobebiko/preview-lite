import { describe, expect, it } from "vitest";
import {
  validatePrintResourceInspectionReport,
  validateRgbDerivedCoveragePreview,
  validateRgbDerivedProcessPreview,
} from "./prepressSchemas.js";

const expected = { sessionId: "s", contentGeneration: 3, physicalPageIndex: 1, requestId: "r" };
const identity = { ...expected, methodologyVersion: "naive-rgb-to-cmyk/1" };
const metadata = { schemaVersion: "rgb-derived-print-preview/1", methodology: "naive-rgb-to-cmyk/1", sourceColorModel: "pdfium_display_rgb", approximate: true, colorManaged: false, outputIntentApplied: false, spotColorsPreserved: false, overprintSimulated: false, renderingIntent: "not_applied", warnings: [] };
const observedBox = (coordinates, source, validGeometry) => ({ coordinates, source, validGeometry, nonZeroOrigin: false, negativeOrigin: false });
const resourceCounts = { visitedObjects: 1, resourceReferences: 1, xobjects: 0, patterns: 0, shadings: 0, colorSpaceDefinitions: 1, uniqueColorSpaceObjects: 0, iccProfiles: 0, outputIntents: 0, spotColors: 0, fonts: 0, transparencyObservations: 0 };
const report = { schemaVersion: "print-resource-inspection/1", inspectionVersion: "print-resource-inspection/1.0", identity: { ...identity, methodologyVersion: "print-resource-inspection/1.0" }, summaryStatus: "inspection_incomplete", documentFacts: { pdfVersion: "1.7", pageCount: 1, encrypted: false }, colorSpaces: [{ id: "direct:DeviceRGB", objectId: null, resourceName: null, family: "device_rgb", definitionCount: 1, referenceCount: 1, affectedPages: [1], alternateFamily: null, usageStatus: "usage_not_checked" }], iccProfiles: [], outputIntents: [], spotColors: [], transparency: [], fonts: [], pageBoxes: [{ physicalPageIndex: 1, rotationDegrees: 0, mediaBox: observedBox([0, 0, 10, 10], "explicit", true), cropBox: observedBox(null, "not_present", false), bleedBox: observedBox(null, "not_present", false), trimBox: observedBox(null, "not_present", false), artBox: observedBox(null, "not_present", false) }], coverage: { inspectedAreas: ["catalog"], notInspectedAreas: [], incompleteAreas: [], limitations: [], resourceCounts, pagesInspected: 1, areas: [{ area: "catalog", status: "inspected" }] }, warnings: [], truncated: false, conformanceClaim: false };

describe("prepress runtime trust boundary", () => {
  it("accepts the known inspection schema with finite read-only boxes", () => {
    expect(validatePrintResourceInspectionReport(report, expected)).toBe(report);
  });
  it("rejects unknown schemas, conformance claims, stale identity, and non-finite boxes", () => {
    expect(() => validatePrintResourceInspectionReport({ ...report, schemaVersion: "legacy" }, expected)).toThrow(/bilinmeyen inspection schema/);
    expect(() => validatePrintResourceInspectionReport({ ...report, conformanceClaim: true }, expected)).toThrow(/conformanceClaim/);
    expect(() => validatePrintResourceInspectionReport({ ...report, identity: { ...report.identity, requestId: "stale" } }, expected)).toThrow(/identity/);
    const brokenBoxes = structuredClone(report); brokenBoxes.pageBoxes[0].mediaBox.coordinates[1] = Number.NaN;
    expect(() => validatePrintResourceInspectionReport(brokenBoxes, expected)).toThrow(/finite/);
  });
  it("requires honest preview semantics and matching encoded-size metadata", () => {
    const coverage = { identity, metadata, pageIndex: 1, width: 1, height: 1, thresholdPercent: 300, heatmapPngBase64: "AAAA", encodedPngBytes: 3, pixelStatistics: { pixelsAboveThreshold: 0, totalPixels: 1, maximumEstimatedChannelSumPercent: 0, averageEstimatedChannelSumPercent: 0 }, truncated: false };
    expect(validateRgbDerivedCoveragePreview(coverage, expected)).toBe(coverage);
    expect(() => validateRgbDerivedCoveragePreview({ ...coverage, metadata: { ...metadata, colorManaged: true } }, expected)).toThrow(/colorManaged/);
    expect(() => validateRgbDerivedCoveragePreview({ ...coverage, encodedPngBytes: 4 }, expected)).toThrow(/size metadata/);

    const process = { identity, metadata, pageIndex: 1, width: 1, height: 1, cPreviewPngBase64: "AAAA", mPreviewPngBase64: "AAAA", yPreviewPngBase64: "AAAA", kPreviewPngBase64: "AAAA", encodedPngBytes: [3, 3, 3, 3], totalEncodedPngBytes: 12, truncated: false };
    expect(validateRgbDerivedProcessPreview(process, expected)).toBe(process);
    expect(() => validateRgbDerivedProcessPreview({ ...process, totalEncodedPngBytes: 11 }, expected)).toThrow(/totalEncoded/);
  });
});
