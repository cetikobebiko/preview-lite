import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { detectScannedPages, getAvailableOcrLanguages } from "./pdfState.js";
import {
  OCR_FAILURE_RETRY_AFTER_MS,
  getOcrLanguages,
  isPageScanned,
  resetOcrDocumentStateForTests,
} from "./ocrDocumentState.js";

vi.mock("./pdfState.js", () => ({
  detectScannedPages: vi.fn(),
  getAvailableOcrLanguages: vi.fn(),
}));

let now;
let consoleError;

beforeEach(() => {
  resetOcrDocumentStateForTests();
  now = vi.spyOn(Date, "now").mockReturnValue(10_000);
  consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  vi.clearAllMocks();
  now.mockRestore();
  consoleError.mockRestore();
});

describe("getOcrLanguages", () => {
  it("shares one backend call between concurrent and later callers", async () => {
    getAvailableOcrLanguages.mockResolvedValue(["eng", "tur"]);

    const results = await Promise.all([getOcrLanguages(), getOcrLanguages(), getOcrLanguages()]);
    expect(await getOcrLanguages()).toEqual(["eng", "tur"]);

    expect(results).toEqual([["eng", "tur"], ["eng", "tur"], ["eng", "tur"]]);
    expect(getAvailableOcrLanguages).toHaveBeenCalledTimes(1);
  });

  it("caches the failure fallback and retries once after the retry window", async () => {
    getAvailableOcrLanguages
      .mockRejectedValueOnce("Tesseract bulunamadı")
      .mockResolvedValueOnce(["eng", "deu"]);

    expect(await getOcrLanguages()).toEqual(["eng"]);
    now.mockReturnValue(10_000 + OCR_FAILURE_RETRY_AFTER_MS - 1);
    expect(await getOcrLanguages()).toEqual(["eng"]);
    expect(getAvailableOcrLanguages).toHaveBeenCalledTimes(1);

    now.mockReturnValue(10_000 + OCR_FAILURE_RETRY_AFTER_MS);
    expect(await getOcrLanguages()).toEqual(["eng", "deu"]);
    now.mockReturnValue(10_000 + OCR_FAILURE_RETRY_AFTER_MS * 5);
    expect(await getOcrLanguages()).toEqual(["eng", "deu"]);
    expect(getAvailableOcrLanguages).toHaveBeenCalledTimes(2);
  });

  it("treats a malformed language payload as a failure", async () => {
    getAvailableOcrLanguages.mockResolvedValue({ langs: ["eng"] });
    expect(await getOcrLanguages()).toEqual(["eng"]);
    expect(consoleError).toHaveBeenCalledOnce();
  });
});

describe("isPageScanned", () => {
  it("runs one detection job per session generation for all pages", async () => {
    detectScannedPages.mockResolvedValue([true, false, true]);

    const flags = await Promise.all([0, 1, 2, 3].map((page) => isPageScanned("s1", 1, page)));

    expect(flags).toEqual([true, false, true, false]);
    expect(detectScannedPages).toHaveBeenCalledTimes(1);
    expect(detectScannedPages).toHaveBeenCalledWith("s1", 1);
  });

  it("replaces the cached generation when the content generation changes", async () => {
    detectScannedPages.mockResolvedValueOnce([true]).mockResolvedValueOnce([false]);

    expect(await isPageScanned("s1", 1, 0)).toBe(true);
    expect(await isPageScanned("s1", 2, 0)).toBe(false);
    expect(await isPageScanned("s1", 2, 0)).toBe(false);

    expect(detectScannedPages.mock.calls).toEqual([["s1", 1], ["s1", 2]]);
  });

  it("keeps sessions independent and bounds the cache", async () => {
    detectScannedPages.mockResolvedValue([true]);

    for (let index = 0; index < 9; index += 1) {
      await isPageScanned(`s${index}`, 1, 0);
    }
    await isPageScanned("s8", 1, 0);
    expect(detectScannedPages).toHaveBeenCalledTimes(9);

    // En eski session LRU sınırı nedeniyle düşmüştür.
    await isPageScanned("s0", 1, 0);
    expect(detectScannedPages).toHaveBeenCalledTimes(10);
  });

  it("caches detection failures as not scanned until the retry window elapses", async () => {
    detectScannedPages
      .mockRejectedValueOnce(new Error("SESSION_JOB_FAILED"))
      .mockResolvedValueOnce([true]);

    expect(await isPageScanned("s1", 1, 0)).toBe(false);
    expect(await isPageScanned("s1", 1, 0)).toBe(false);
    expect(detectScannedPages).toHaveBeenCalledTimes(1);

    now.mockReturnValue(10_000 + OCR_FAILURE_RETRY_AFTER_MS);
    expect(await isPageScanned("s1", 1, 0)).toBe(true);
    expect(detectScannedPages).toHaveBeenCalledTimes(2);
  });

  it("rejects malformed detection payloads", async () => {
    detectScannedPages.mockResolvedValue([true, "yes"]);
    expect(await isPageScanned("s1", 1, 0)).toBe(false);
    expect(consoleError).toHaveBeenCalledOnce();
  });

  it("does not start a job without a session", async () => {
    expect(await isPageScanned("", 1, 0)).toBe(false);
    expect(detectScannedPages).not.toHaveBeenCalled();
  });
});
