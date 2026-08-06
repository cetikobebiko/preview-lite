import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import {
  detectScannedPage,
  getAvailableOcrLanguages,
  runOcrOnPage,
} from "../pdfState.js";
import OcrLayer from "./OcrLayer.svelte";

vi.mock("../pdfState.js", () => ({
  detectScannedPage: vi.fn(),
  getAvailableOcrLanguages: vi.fn(),
  runOcrOnPage: vi.fn(),
}));

const originalAnimate = Element.prototype.animate;
const originalAlert = globalThis.alert;

function createDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  Element.prototype.animate = vi.fn(() => ({
    cancel: vi.fn(),
    finished: Promise.resolve(),
  }));
  globalThis.alert = vi.fn();
  detectScannedPage.mockResolvedValue(true);
  getAvailableOcrLanguages.mockResolvedValue(["eng"]);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  Element.prototype.animate = originalAnimate;
  globalThis.alert = originalAlert;
});

describe("OcrLayer session identity", () => {
  it("uses the active session and generation for scan detection and OCR", async () => {
    detectScannedPage.mockResolvedValue(true);
    getAvailableOcrLanguages.mockResolvedValue(["eng", "tur"]);
    runOcrOnPage.mockResolvedValue({ words: [] });

    const { getByRole } = render(OcrLayer, {
      props: {
        sessionId: "session-ocr",
        contentGeneration: 7,
        pageIndex: 3,
        pageWidth: 100,
        pageHeight: 100,
      },
    });

    await waitFor(() => {
      expect(detectScannedPage).toHaveBeenCalledWith("session-ocr", 7, 3);
      expect(getByRole("button", { name: "OCR Başlat" })).toBeTruthy();
    });

    await fireEvent.click(getByRole("button", { name: "OCR Başlat" }));
    await waitFor(() => {
      expect(runOcrOnPage).toHaveBeenCalledWith(
        "session-ocr",
        7,
        3,
        "tur",
      );
    });
  });

  it("does not render a stale scan result after the session changes", async () => {
    let resolveOld;
    detectScannedPage
      .mockImplementationOnce(
        () => new Promise((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValueOnce(false);
    getAvailableOcrLanguages.mockResolvedValue(["eng"]);

    const view = render(OcrLayer, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageIndex: 0,
      },
    });
    await waitFor(() => expect(detectScannedPage).toHaveBeenCalledTimes(1));

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
      pageIndex: 0,
    });
    await waitFor(() => expect(detectScannedPage).toHaveBeenCalledTimes(2));

    resolveOld(true);
    await Promise.resolve();
    expect(view.queryByRole("button", { name: "OCR Başlat" })).toBeNull();
  });

  it("ignores stale OCR success after identity changes", async () => {
    const pending = createDeferred();
    runOcrOnPage.mockReturnValue(pending.promise);
    const view = render(OcrLayer, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageIndex: 0,
        pageWidth: 100,
        pageHeight: 100,
      },
    });
    const button = await waitFor(() =>
      view.getByRole("button", { name: "OCR Başlat" }),
    );
    await fireEvent.click(button);
    await waitFor(() =>
      expect(view.getByRole("button", { name: "Taranıyor..." })).toBeTruthy(),
    );

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
      pageIndex: 0,
      pageWidth: 100,
      pageHeight: 100,
    });
    pending.resolve({
      words: [{
        text: "STALE_OCR_WORD",
        x: 0,
        y: 0,
        width: 10,
        height: 10,
      }],
    });

    await Promise.resolve();
    expect(view.queryByText("STALE_OCR_WORD")).toBeNull();
  });

  it("ignores stale OCR errors without alerting", async () => {
    const pending = createDeferred();
    runOcrOnPage.mockReturnValue(pending.promise);
    const view = render(OcrLayer, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageIndex: 0,
      },
    });
    await fireEvent.click(
      await waitFor(() => view.getByRole("button", { name: "OCR Başlat" })),
    );

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
      pageIndex: 0,
    });
    pending.reject(new Error("stale OCR failure"));
    await Promise.resolve();
    await Promise.resolve();

    expect(globalThis.alert).not.toHaveBeenCalled();
  });

  it("leaves the new identity out of loading state after stale completion", async () => {
    const pending = createDeferred();
    runOcrOnPage.mockReturnValue(pending.promise);
    const view = render(OcrLayer, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageIndex: 0,
      },
    });
    await fireEvent.click(
      await waitFor(() => view.getByRole("button", { name: "OCR Başlat" })),
    );

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
      pageIndex: 0,
    });
    await waitFor(() => {
      const currentButton = view.getByRole("button", { name: "OCR Başlat" });
      expect(currentButton.disabled).toBe(false);
    });

    pending.resolve({ words: [] });
    await Promise.resolve();
    expect(view.queryByRole("button", { name: "Taranıyor..." })).toBeNull();
    expect(view.getByRole("button", { name: "OCR Başlat" }).disabled).toBe(false);
  });

  it("reports the current OCR error and clears loading", async () => {
    runOcrOnPage.mockRejectedValue(new Error("current OCR failure"));
    const view = render(OcrLayer, {
      props: {
        sessionId: "current-session",
        contentGeneration: 4,
        pageIndex: 2,
      },
    });
    await fireEvent.click(
      await waitFor(() => view.getByRole("button", { name: "OCR Başlat" })),
    );

    await waitFor(() => expect(globalThis.alert).toHaveBeenCalledOnce());
    expect(globalThis.alert.mock.calls[0][0]).toContain("current OCR failure");
    expect(view.getByRole("button", { name: "OCR Başlat" }).disabled).toBe(false);
  });
});
