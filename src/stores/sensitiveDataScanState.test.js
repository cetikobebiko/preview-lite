import { describe, expect, it } from "vitest";
import { createSensitiveDataScanState } from "./sensitiveDataScanState.svelte.js";

describe("sensitiveDataScanState store", () => {
  it("initializes with clean default values", () => {
    const store = createSensitiveDataScanState();
    expect(store.isScanning).toBe(false);
    expect(store.report).toBeNull();
    expect(store.error).toBeNull();
    expect(store.capturedPatternLabels).toEqual([]);
    expect(store.logicalHighlightsByPage).toEqual({});
  });

  it("handles beginScan, completeScan, failScan and reset lifecycle", () => {
    const store = createSensitiveDataScanState();

    store.beginScan(["TC Kimlik", "E-Posta"]);
    expect(store.isScanning).toBe(true);
    expect(store.error).toBeNull();
    expect(store.capturedPatternLabels).toEqual(["TC Kimlik", "E-Posta"]);

    const mockReport = { schemaVersion: 2, items: [], warnings: [], truncated: false };
    const mockHighlights = { 0: [{ x: 0.1, y: 0.1, width: 0.2, height: 0.2 }] };
    store.completeScan(mockReport, mockHighlights);
    expect(store.isScanning).toBe(false);
    expect(store.report).toEqual(mockReport);
    expect(store.logicalHighlightsByPage).toEqual(mockHighlights);

    store.failScan("Bir hata oluştu");
    expect(store.isScanning).toBe(false);
    expect(store.error).toBe("Bir hata oluştu");
    expect(store.report).toBeNull();
    expect(store.logicalHighlightsByPage).toEqual({});

    store.reset();
    expect(store.isScanning).toBe(false);
    expect(store.report).toBeNull();
    expect(store.error).toBeNull();
    expect(store.capturedPatternLabels).toEqual([]);
    expect(store.logicalHighlightsByPage).toEqual({});
  });
});
