import { describe, expect, it, vi } from "vitest";
import { createTtsActionController } from "./ttsActionController.js";

function setup(overrides = {}) {
  const viewer = { documentTextContext: "Hazır metin", documentTextPageIndex: 2, isSpeaking: false };
  const context = { isPdf: true, sessionId: "session-a", contentGeneration: 4, pageIndex: 2, displayPageIndex: 2, viewer };
  const extractText = vi.fn(async () => "\n--- Sayfa 3 ---\n\n");
  const guard = vi.fn(async (_feature, _runtime, action) => action());
  const performOcr = vi.fn(async () => "OCR metni");
  const speak = vi.fn(async () => undefined);
  const stop = vi.fn(async (_runtime, onStopped) => onStopped());
  const onToast = vi.fn();
  const controller = createTtsActionController({
    getContext: () => ({ ...context }), extractText, performOcr, speak, stop, guard, featureContext: {},
    reportUnavailable: () => false, onToast, ...overrides,
  });
  return { controller, context, viewer, guard, extractText, performOcr, speak, stop, onToast };
}

describe("ttsActionController", () => {
  it("speaks the current page's PDF text layer through its guarded action lifecycle", async () => {
    const { controller, viewer, guard, extractText, performOcr, speak } = setup();
    extractText.mockResolvedValueOnce("\n--- Sayfa 3 ---\n\nPDF katmanı metni");
    await controller.speakCurrentPage();
    expect(guard).toHaveBeenCalledOnce();
    expect(extractText).toHaveBeenCalledWith("session-a", 4, [2], "text");
    expect(performOcr).not.toHaveBeenCalled();
    expect(speak).toHaveBeenCalledWith("PDF katmanı metni", "Yelda", {});
    expect(viewer.isSpeaking).toBe(true);
  });

  it("reuses OCR text for a scanned page only when it was recognized from that page", async () => {
    const { controller, context, performOcr, speak } = setup();
    await controller.speakCurrentPage();
    expect(performOcr).not.toHaveBeenCalled();
    expect(speak).toHaveBeenLastCalledWith("Hazır metin", "Yelda", {});

    context.pageIndex = 5;
    await controller.speakCurrentPage();
    expect(performOcr).toHaveBeenCalledWith("session-a", 4, 5, {});
    expect(speak).toHaveBeenLastCalledWith("OCR metni", "Yelda", {});
  });

  it("cleans up speaking state when OCR becomes stale", async () => {
    const { controller, context, viewer, performOcr, speak } = setup();
    viewer.documentTextContext = "";
    performOcr.mockImplementationOnce(async () => {
      context.contentGeneration += 1;
      return "Eski OCR";
    });
    await controller.speakCurrentPage();
    expect(speak).not.toHaveBeenCalled();
    expect(viewer.isSpeaking).toBe(false);
  });

  it("cleans up after an unsupported image request and after stopping", async () => {
    const { controller, context, viewer, stop, onToast } = setup();
    viewer.documentTextContext = "";
    context.isPdf = false;
    await controller.speakCurrentPage();
    expect(viewer.isSpeaking).toBe(false);
    expect(onToast).toHaveBeenLastCalledWith("Bağımsız görseller için OCR bu sürümde desteklenmiyor.");
    viewer.isSpeaking = true;
    await controller.stopSpeaking();
    expect(stop).toHaveBeenCalledOnce();
    expect(viewer.isSpeaking).toBe(false);
  });
});
