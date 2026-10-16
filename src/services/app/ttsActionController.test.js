import { describe, expect, it, vi } from "vitest";
import { createTtsActionController, TTS_FINISHED_EVENT } from "./ttsActionController.js";

function setup(overrides = {}) {
  const viewer = { documentTextContext: "Hazır metin", documentTextPageIndex: 2, isSpeaking: false };
  const context = { isPdf: true, sessionId: "session-a", contentGeneration: 4, pageIndex: 2, displayPageIndex: 2, viewer };
  const extractText = vi.fn(async () => "\n--- Sayfa 3 ---\n\n");
  const guard = vi.fn(async (_feature, _runtime, action) => action());
  const performOcr = vi.fn(async () => "OCR metni");
  const speak = vi.fn(async () => undefined);
  const stop = vi.fn(async (_runtime, onStopped) => onStopped());
  const onToast = vi.fn();
  const listeners = new Map();
  const unlisten = vi.fn();
  const listenToEvent = vi.fn(async (name, handler) => {
    listeners.set(name, handler);
    return unlisten;
  });
  const controller = createTtsActionController({
    getContext: () => ({ ...context }), extractText, performOcr, speak, stop, guard, featureContext: {},
    reportUnavailable: () => false, onToast, listenToEvent, ...overrides,
  });
  const emitFinished = (payload) => listeners.get(TTS_FINISHED_EVENT)?.({ payload });
  return { controller, context, viewer, guard, extractText, performOcr, speak, stop, onToast, listenToEvent, unlisten, emitFinished };
}

async function startedController(overrides = {}) {
  const harness = setup(overrides);
  harness.controller.start();
  await vi.waitFor(() => expect(harness.listenToEvent).toHaveBeenCalledOnce());
  harness.extractText.mockResolvedValue("\n--- Sayfa 3 ---\n\nPDF katmanı metni");
  return harness;
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

  it("resets speaking state when the backend reports the active generation finished", async () => {
    const { controller, viewer, speak, onToast, emitFinished } = await startedController();
    speak.mockResolvedValueOnce({ generation: 7, voice: "Yelda", voice_fallback: false });
    await controller.speakCurrentPage();
    expect(viewer.isSpeaking).toBe(true);

    emitFinished({ generation: 7, status: "completed", message: null });
    expect(viewer.isSpeaking).toBe(false);
    expect(onToast).not.toHaveBeenCalledWith(expect.stringContaining("hata"));
  });

  it("shows the backend failure message and clears speaking state", async () => {
    const { controller, viewer, speak, onToast, emitFinished } = await startedController();
    speak.mockResolvedValueOnce({ generation: 3, voice: null, voice_fallback: false });
    await controller.speakCurrentPage();

    emitFinished({ generation: 3, status: "failed", message: "Seslendirme süreci hata koduyla sonlandı (1)" });
    expect(viewer.isSpeaking).toBe(false);
    expect(onToast).toHaveBeenLastCalledWith("Seslendirme hatası: Seslendirme süreci hata koduyla sonlandı (1)");
  });

  it("ignores finished events from stale generations and after stopping", async () => {
    const { controller, viewer, speak, onToast, emitFinished } = await startedController();
    speak.mockResolvedValueOnce({ generation: 4, voice: "Yelda", voice_fallback: false });
    await controller.speakCurrentPage();
    speak.mockResolvedValueOnce({ generation: 5, voice: "Yelda", voice_fallback: false });
    await controller.speakCurrentPage();

    emitFinished({ generation: 4, status: "failed", message: "eski" });
    expect(viewer.isSpeaking).toBe(true);
    expect(onToast).not.toHaveBeenCalledWith("Seslendirme hatası: eski");

    await controller.stopSpeaking();
    expect(viewer.isSpeaking).toBe(false);
    viewer.isSpeaking = true;
    emitFinished({ generation: 5, status: "failed", message: "durdurulan" });
    expect(viewer.isSpeaking).toBe(true);
    expect(onToast).not.toHaveBeenCalledWith("Seslendirme hatası: durdurulan");
    emitFinished({ generation: "5", status: "completed" });
    emitFinished(null);
    expect(viewer.isSpeaking).toBe(true);
  });

  it("applies a finished event that arrives before speak_text resolves", async () => {
    const { controller, viewer, speak, onToast, emitFinished } = await startedController();
    let resolveSpeak;
    speak.mockImplementationOnce(() => new Promise((resolve) => { resolveSpeak = resolve; }));
    const pending = controller.speakCurrentPage();
    await vi.waitFor(() => expect(speak).toHaveBeenCalledOnce());

    emitFinished({ generation: 8, status: "failed", message: "ses yok" });
    expect(viewer.isSpeaking).toBe(true);
    resolveSpeak({ generation: 8, voice: null, voice_fallback: false });
    await pending;
    expect(viewer.isSpeaking).toBe(false);
    expect(onToast).toHaveBeenLastCalledWith("Seslendirme hatası: ses yok");
  });

  it("does not apply an early event from another generation", async () => {
    const { controller, viewer, speak, emitFinished } = await startedController();
    let resolveSpeak;
    speak.mockImplementationOnce(() => new Promise((resolve) => { resolveSpeak = resolve; }));
    const pending = controller.speakCurrentPage();
    await vi.waitFor(() => expect(speak).toHaveBeenCalledOnce());

    emitFinished({ generation: 1, status: "completed", message: null });
    resolveSpeak({ generation: 2, voice: "Yelda", voice_fallback: false });
    await pending;
    expect(viewer.isSpeaking).toBe(true);
    emitFinished({ generation: 2, status: "completed", message: null });
    expect(viewer.isSpeaking).toBe(false);
  });

  it("keeps state untouched when stopped while speak_text is still starting", async () => {
    const { controller, viewer, speak, emitFinished } = await startedController();
    let resolveSpeak;
    speak.mockImplementationOnce(() => new Promise((resolve) => { resolveSpeak = resolve; }));
    const pending = controller.speakCurrentPage();
    await vi.waitFor(() => expect(speak).toHaveBeenCalledOnce());
    await controller.stopSpeaking();
    resolveSpeak({ generation: 9, voice: "Yelda", voice_fallback: false });
    await pending;
    expect(viewer.isSpeaking).toBe(false);
    viewer.isSpeaking = true;
    emitFinished({ generation: 9, status: "completed", message: null });
    expect(viewer.isSpeaking).toBe(true);
  });

  it("announces the system-voice fallback with a platform-neutral message", async () => {
    const { controller, speak, onToast } = await startedController();
    speak.mockResolvedValueOnce({ generation: 1, voice: null, voice_fallback: true });
    await controller.speakCurrentPage();
    expect(onToast).toHaveBeenCalledWith("Sayfa metni seslendiriliyor...");
    expect(onToast).toHaveBeenLastCalledWith("Yelda sesi kurulu değil; sistem varsayılan sesi kullanılıyor.");
    for (const [message] of onToast.mock.calls) expect(message).not.toMatch(/macOS|Windows/);
  });

  it("removes the listener on destroy, including one that resolves after destroy", async () => {
    const { controller, unlisten, listenToEvent } = await startedController();
    controller.destroy();
    expect(unlisten).toHaveBeenCalledOnce();

    let resolveListen;
    const lateUnlisten = vi.fn();
    listenToEvent.mockImplementationOnce(() => new Promise((resolve) => { resolveListen = resolve; }));
    controller.start();
    await vi.waitFor(() => expect(listenToEvent).toHaveBeenCalledTimes(2));
    controller.destroy();
    resolveListen(lateUnlisten);
    await vi.waitFor(() => expect(lateUnlisten).toHaveBeenCalledOnce());
  });

  it("clears speaking state when the backend cancels a start without a process", async () => {
    const { controller, viewer, speak, onToast, emitFinished } = await startedController();
    speak.mockResolvedValueOnce({ generation: null, voice: "Yelda", voice_fallback: false, cancelled: true });
    await controller.speakCurrentPage();
    expect(viewer.isSpeaking).toBe(false);
    expect(onToast).not.toHaveBeenCalledWith(expect.stringContaining("hata"));
    viewer.isSpeaking = true;
    emitFinished({ generation: 1, status: "completed", message: null });
    expect(viewer.isSpeaking).toBe(true);
  });

  it("does not surface start errors of a request that was already stopped", async () => {
    const { controller, viewer, speak, onToast } = await startedController();
    let rejectSpeak;
    speak.mockImplementationOnce(() => new Promise((_resolve, reject) => { rejectSpeak = reject; }));
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const pending = controller.speakCurrentPage();
    await vi.waitFor(() => expect(speak).toHaveBeenCalledOnce());
    await controller.stopSpeaking();
    rejectSpeak(new Error("geç hata"));
    await pending;
    errorSpy.mockRestore();
    expect(viewer.isSpeaking).toBe(false);
    expect(onToast).not.toHaveBeenCalledWith(expect.stringContaining("geç hata"));
  });
});
