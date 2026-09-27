import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { tick } from "svelte";
import {
  generateSessionRgbDerivedCoveragePreview,
  generateSessionRgbDerivedProcessPreview,
  inspectSessionPrintResources,
} from "../services/backend/prepress.js";
import PrepressPanel from "./PrepressPanel.svelte";

vi.mock("../services/backend/prepress.js", () => ({
  inspectSessionPrintResources: vi.fn(),
  generateSessionRgbDerivedCoveragePreview: vi.fn(),
  generateSessionRgbDerivedProcessPreview: vi.fn(),
}));

const identity = (requestId) => ({ sessionId: "session-test-1", contentGeneration: 7, physicalPageIndex: 2, requestId, methodologyVersion: "naive-rgb-to-cmyk/1" });
const metadata = { schemaVersion: "rgb-derived-print-preview/1", methodology: "naive-rgb-to-cmyk/1", sourceColorModel: "pdfium_display_rgb", approximate: true, colorManaged: false, outputIntentApplied: false, spotColorsPreserved: false, overprintSimulated: false, renderingIntent: "not_applied", warnings: [] };
const report = (requestId) => ({ schemaVersion: "print-resource-inspection/1", inspectionVersion: "print-resource-inspection/1.0", identity: { ...identity(requestId), methodologyVersion: "print-resource-inspection/1.0" }, summaryStatus: "inspection_incomplete", documentFacts: { pdfVersion: "1.7", pageCount: 1, encrypted: false }, colorSpaces: [{ family: "device_cmyk", referenceCount: 1 }], iccProfiles: [], outputIntents: [], spotColors: [], transparency: [], fonts: [], pageBoxes: [{ physicalPageIndex: 2, rotationDegrees: 0, mediaBox: { coordinates: [0, 0, 100, 200] }, cropBox: { coordinates: [0, 0, 100, 200] }, bleedBox: { coordinates: null }, trimBox: { coordinates: null }, artBox: { coordinates: null } }], coverage: { inspectedAreas: ["catalog"], notInspectedAreas: ["content_stream_usage"], incompleteAreas: [], limitations: ["usage not checked"] }, warnings: [], truncated: false, conformanceClaim: false });
const processPreview = (requestId) => ({ identity: identity(requestId), metadata, cPreviewPngBase64: "c", mPreviewPngBase64: "m", yPreviewPngBase64: "y", kPreviewPngBase64: "k" });
const coveragePreview = (requestId) => ({ identity: identity(requestId), metadata, heatmapPngBase64: "heat", pixelStatistics: { pixelsAboveThreshold: 1, totalPixels: 2, averageEstimatedChannelSumPercent: 50 } });

function deferredHandle() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject, cancel: vi.fn().mockResolvedValue({ status: "cancel_pending" }) };
}
const resolvedHandle = (value) => ({ promise: Promise.resolve(value), cancel: vi.fn().mockResolvedValue({}) });
function renderReady(props = {}, events = {}) {
  return render(PrepressPanel, { props: { sessionId: "session-test-1", pageIndex: 2, contentGeneration: 7, isDocumentReady: true, scale: 1.5, ...props }, events });
}

beforeEach(() => {
  vi.clearAllMocks();
  let sequence = 0;
  vi.spyOn(globalThis.crypto, "randomUUID").mockImplementation(() => `request-${++sequence}`);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("Print inspection and approximate preview panel", () => {
  it("uses honest permanent terminology and has no editable page-box controls", () => {
    const { getByText, queryByText, queryByLabelText } = renderReady();
    expect(getByText("Print Inspection & Approximate Preview")).toBeTruthy();
    expect(getByText(/Gerçek separasyon, spot plate, ICC transform veya RIP çıktısı değildir/)).toBeTruthy();
    expect(getByText(/gerçek Total Area Coverage veya mürekkep tüketimi değildir/)).toBeTruthy();
    expect(queryByText("Renk Ayrımları (Separation)")).toBeNull();
    expect(queryByLabelText(/Taşırma Payı|Kesim Payı/)).toBeNull();
  });

  it("sends authoritative page identity and renders read-only inspection coverage", async () => {
    inspectSessionPrintResources.mockImplementation((_session, _generation, _page, requestId) => resolvedHandle(report(requestId)));
    const { getByRole, getByText } = renderReady();
    await fireEvent.click(getByRole("button", { name: "PDF kaynaklarını incele" }));
    await waitFor(() => expect(getByText(/device_cmyk/)).toBeTruthy());
    expect(inspectSessionPrintResources).toHaveBeenCalledWith("session-test-1", 7, 2, "request-1");
    expect(getByText(/Conformance claim:/).parentElement.textContent).toContain("false");
    expect(getByText(/MediaBox/)).toBeTruthy();
    expect(getByText(/İncelenmeyen:/)).toBeTruthy();
  });

  it("emits process preview with request ownership and methodology metadata", async () => {
    generateSessionRgbDerivedProcessPreview.mockImplementation((_session, _generation, _page, requestId) => resolvedHandle(processPreview(requestId)));
    const onReady = vi.fn();
    const { getByRole, getAllByText } = renderReady({}, { processPreviewReady: onReady });
    await fireEvent.click(getByRole("button", { name: "Tahmini C/M/Y/K görünümü üret" }));
    await waitFor(() => expect(onReady.mock.calls.some(([event]) => event.detail?.result)).toBe(true));
    expect(generateSessionRgbDerivedProcessPreview).toHaveBeenCalledWith("session-test-1", 7, 2, "request-1", 1.5);
    const readyEvent = onReady.mock.calls.find(([event]) => event.detail?.result)[0];
    expect(readyEvent.detail).toMatchObject({ physicalPageIndex: 2, requestId: "request-1" });
    expect(getAllByText("APPROXIMATE RGB-DERIVED PREVIEW").length).toBeGreaterThan(0);
    expect(getAllByText(/outputIntentApplied: false/).length).toBeGreaterThan(0);
  });

  it("keeps only one approximate preview mode active", async () => {
    generateSessionRgbDerivedProcessPreview.mockImplementation((_a, _b, _c, requestId) => resolvedHandle(processPreview(requestId)));
    generateSessionRgbDerivedCoveragePreview.mockImplementation((_a, _b, _c, requestId) => resolvedHandle(coveragePreview(requestId)));
    const onProcess = vi.fn(), onCoverage = vi.fn();
    const { getByRole } = renderReady({}, { processPreviewReady: onProcess, coveragePreviewReady: onCoverage });
    await fireEvent.click(getByRole("button", { name: "Tahmini C/M/Y/K görünümü üret" }));
    await waitFor(() => expect(onProcess).toHaveBeenCalled());
    await fireEvent.click(getByRole("button", { name: "Tahmini eşik ısı haritası üret" }));
    await waitFor(() => expect(onCoverage).toHaveBeenCalled());
    expect(onProcess.mock.calls.some(([event]) => event.detail?.result === null && event.detail.requestId === "request-1")).toBe(true);
  });

  it("cancels and rejects stale work after a page switch", async () => {
    const oldHandle = deferredHandle();
    generateSessionRgbDerivedProcessPreview.mockReturnValue(oldHandle);
    const onReady = vi.fn();
    const { getByRole, rerender } = renderReady({}, { processPreviewReady: onReady });
    await fireEvent.click(getByRole("button", { name: "Tahmini C/M/Y/K görünümü üret" }));
    await rerender({ sessionId: "session-test-1", pageIndex: 3, contentGeneration: 7, isDocumentReady: true, scale: 1.5 });
    await tick();
    expect(oldHandle.cancel).toHaveBeenCalled();
    oldHandle.resolve(processPreview("request-1"));
    await tick();
    expect(onReady.mock.calls.some(([event]) => event.detail?.result)).toBe(false);
  });

  it("does not call PDF operations before the session document is ready", async () => {
    const { getByRole } = renderReady({ isDocumentReady: false });
    const buttons = ["PDF kaynaklarını incele", "Tahmini C/M/Y/K görünümü üret", "Tahmini eşik ısı haritası üret"];
    for (const name of buttons) { const button = getByRole("button", { name }); expect(button.disabled).toBe(true); await fireEvent.click(button); }
    expect(inspectSessionPrintResources).not.toHaveBeenCalled();
    expect(generateSessionRgbDerivedProcessPreview).not.toHaveBeenCalled();
  });

  it("shows the encrypted inspection capability difference as a typed failure", async () => {
    const error = Object.assign(new Error("Print resource inspection encrypted session snapshot girdisini desteklemiyor"), { code: "PRINT_INSPECTION_ENCRYPTED_INPUT_UNSUPPORTED" });
    inspectSessionPrintResources.mockReturnValue({ promise: Promise.reject(error), cancel: vi.fn() });
    const { getByRole, findByText } = renderReady();
    await fireEvent.click(getByRole("button", { name: "PDF kaynaklarını incele" }));
    expect(await findByText(/encrypted session snapshot/)).toBeTruthy();
  });
});
