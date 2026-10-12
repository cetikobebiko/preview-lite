import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/svelte";
import {
  detectScannedPages,
  getAvailableOcrLanguages,
  runOcrOnPage,
} from "../pdfState.js";
import {
  OCR_FAILURE_RETRY_AFTER_MS,
  resetOcrDocumentStateForTests,
} from "../ocrDocumentState.js";
import { tick } from "svelte";
import OcrLayer from "./OcrLayer.svelte";

vi.mock("../pdfState.js", () => ({
  detectScannedPages: vi.fn(),
  getAvailableOcrLanguages: vi.fn(),
  runOcrOnPage: vi.fn(),
}));

const originalAnimate = Element.prototype.animate;
const originalAlert = globalThis.alert;

// Paylaşılan tespit promise zinciri birkaç microtask sürer; DOM'u kontrol
// etmeden önce zinciri ve Svelte flush'ını tamamen boşalt.
async function flushAsync() {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await tick();
}

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
  resetOcrDocumentStateForTests();
  detectScannedPages.mockResolvedValue(Array(8).fill(true));
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
    detectScannedPages.mockResolvedValue([false, false, false, true]);
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
      expect(detectScannedPages).toHaveBeenCalledWith("session-ocr", 7);
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
    detectScannedPages
      .mockImplementationOnce(
        () => new Promise((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValueOnce([false]);
    getAvailableOcrLanguages.mockResolvedValue(["eng"]);

    const view = render(OcrLayer, {
      props: {
        sessionId: "old-session",
        contentGeneration: 1,
        pageIndex: 0,
      },
    });
    await waitFor(() => expect(detectScannedPages).toHaveBeenCalledTimes(1));

    await view.rerender({
      sessionId: "new-session",
      contentGeneration: 2,
      pageIndex: 0,
    });
    await waitFor(() => expect(detectScannedPages).toHaveBeenCalledTimes(2));

    resolveOld([true]);
    await flushAsync();
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

function scanButton(view) {
  return within(view.container).queryByRole("button", { name: "OCR Başlat" });
}

function renderPages(count, props = {}) {
  return Array.from({ length: count }, (_, pageIndex) =>
    render(OcrLayer, {
      props: {
        sessionId: "session-many",
        contentGeneration: 3,
        pageIndex,
        pageWidth: 100,
        pageHeight: 100,
        ...props,
      },
    }),
  );
}

describe("OcrLayer document-level detection", () => {
  it("runs one detection job and one language lookup for N mounted pages", async () => {
    detectScannedPages.mockResolvedValue([true, false, true, false, false, true]);
    const views = renderPages(6);

    await waitFor(() => {
      expect(scanButton(views[5])).not.toBeNull();
    });
    expect(detectScannedPages).toHaveBeenCalledTimes(1);
    expect(detectScannedPages).toHaveBeenCalledWith("session-many", 3);
    expect(getAvailableOcrLanguages).toHaveBeenCalledTimes(1);
    expect(
      views.map((view) => scanButton(view) !== null),
    ).toEqual([true, false, true, false, false, true]);
  });

  it("does not start new jobs when pages scroll out of the buffer and back", async () => {
    let views = renderPages(4);
    await waitFor(() => expect(scanButton(views[3])).not.toBeNull());
    cleanup();

    views = renderPages(4);
    await waitFor(() => expect(scanButton(views[3])).not.toBeNull());

    expect(detectScannedPages).toHaveBeenCalledTimes(1);
    expect(getAvailableOcrLanguages).toHaveBeenCalledTimes(1);
  });

  it("runs one new detection job per content generation and keeps the language cache", async () => {
    const views = renderPages(3);
    await waitFor(() => expect(scanButton(views[2])).not.toBeNull());

    for (const view of views) {
      await view.rerender({ sessionId: "session-many", contentGeneration: 4, pageIndex: 0 });
    }
    await waitFor(() => expect(detectScannedPages).toHaveBeenCalledTimes(2));
    expect(detectScannedPages).toHaveBeenLastCalledWith("session-many", 4);
    expect(getAvailableOcrLanguages).toHaveBeenCalledTimes(1);
  });

  it("ignores an old-generation detection result while the new generation is pending", async () => {
    const oldGeneration = createDeferred();
    const newGeneration = createDeferred();
    detectScannedPages
      .mockReturnValueOnce(oldGeneration.promise)
      .mockReturnValueOnce(newGeneration.promise);
    const view = render(OcrLayer, {
      props: { sessionId: "session-stale", contentGeneration: 1, pageIndex: 0 },
    });
    await waitFor(() => expect(detectScannedPages).toHaveBeenCalledTimes(1));

    await view.rerender({ sessionId: "session-stale", contentGeneration: 2, pageIndex: 0 });
    await waitFor(() => expect(detectScannedPages).toHaveBeenCalledTimes(2));
    oldGeneration.resolve([true]);
    await flushAsync();
    expect(scanButton(view)).toBeNull();

    newGeneration.resolve([false]);
    await flushAsync();
    expect(scanButton(view)).toBeNull();
  });

  it("caches detection failures until the retry window elapses", async () => {
    const now = vi.spyOn(Date, "now").mockReturnValue(1_000);
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    detectScannedPages.mockRejectedValueOnce(new Error("SESSION_JOB_CLEANUP_BACKPRESSURE"));

    renderPages(3);
    await waitFor(() => expect(consoleError).toHaveBeenCalledOnce());
    cleanup();
    renderPages(3);
    await flushAsync();
    expect(detectScannedPages).toHaveBeenCalledTimes(1);

    cleanup();
    now.mockReturnValue(1_000 + OCR_FAILURE_RETRY_AFTER_MS);
    const views = renderPages(3);
    await waitFor(() => expect(scanButton(views[2])).not.toBeNull());
    expect(detectScannedPages).toHaveBeenCalledTimes(2);

    now.mockRestore();
    consoleError.mockRestore();
  });
});
