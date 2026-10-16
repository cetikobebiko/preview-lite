import { readPageText } from "./pageTextSource.js";

/** Backend `tts::SPEECH_FINISHED_EVENT`; payload: `{ generation, status: "completed" | "failed", message }`. */
export const TTS_FINISHED_EVENT = "tts://finished";
export const DEFAULT_TTS_VOICE = "Yelda";

// `speak_text` yanıtı gelmeden önce aktif isteğin generation'ı bilinmez.
const PENDING_GENERATION = Symbol("pending");

/** Coordinates TTS request ownership, page-text/OCR fallback, and speaking-state cleanup. */
export function createTtsActionController({
  getContext,
  extractText,
  performOcr,
  speak,
  stop,
  guard,
  featureContext,
  reportUnavailable,
  onToast,
  listenToEvent = null,
  onError = (message, error) => console.warn(message, error),
}) {
  let requestSequence = 0;
  /** @type {{ id: number, generation: number | null | typeof PENDING_GENERATION, viewer: any } | null} */
  let activeRequest = null;
  // Olay, `speak_text` yanıtından önce gelebilir; en son olay generation'ı öğrenilince eşlenir.
  let earlyFinished = null;
  let listenerEpoch = 0;
  let listening = false;
  let unlistenFinished = null;

  function isCurrent(context) {
    const current = getContext();
    return current.sessionId === context.sessionId && current.contentGeneration === context.contentGeneration;
  }

  function release(request) {
    if (activeRequest !== request) return;
    activeRequest = null;
    earlyFinished = null;
    request.viewer.isSpeaking = false;
  }

  function settle(request, payload) {
    release(request);
    if (payload.status === "failed") onToast("Seslendirme hatası: " + (payload.message || "bilinmeyen hata"));
  }

  function handleFinished(payload) {
    if (!payload || !Number.isSafeInteger(payload.generation)) return;
    const request = activeRequest;
    if (!request) return;
    if (request.generation === PENDING_GENERATION) {
      earlyFinished = payload;
      return;
    }
    // Generation bilinmiyorsa (eski backend yanıtı) ilk bitiş olayı aktif isteğe aittir.
    if (request.generation !== null && request.generation !== payload.generation) return;
    settle(request, payload);
  }

  function start() {
    if (listening || typeof listenToEvent !== "function") return;
    listening = true;
    const epoch = ++listenerEpoch;
    Promise.resolve()
      .then(() => listenToEvent(TTS_FINISHED_EVENT, (event) => handleFinished(event?.payload)))
      .then((dispose) => {
        if (epoch !== listenerEpoch) dispose?.();
        else unlistenFinished = dispose;
      })
      .catch((error) => {
        if (epoch === listenerEpoch) onError("Seslendirme bitiş dinleyicisi kurulamadı", error);
      });
  }

  function destroy() {
    listening = false;
    listenerEpoch += 1;
    unlistenFinished?.();
    unlistenFinished = null;
    activeRequest = null;
    earlyFinished = null;
  }

  async function speakCurrentPage() {
    try { return await guard("text_to_speech", featureContext, executeSpeak); }
    catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  async function executeSpeak() {
    const context = getContext();
    const { viewer } = context;
    const request = { id: ++requestSequence, generation: PENDING_GENERATION, viewer };
    activeRequest = request;
    earlyFinished = null;
    try {
      onToast("Sayfa metni seslendiriliyor...");
      viewer.isSpeaking = true;
      if (!context.isPdf || !context.sessionId || context.pageIndex < 0) {
        release(request);
        onToast("Bağımsız görseller için OCR bu sürümde desteklenmiyor.");
        return;
      }
      const { text: textToRead } = await readPageText(context, { extractText, performOcr, featureContext });
      if (!isCurrent(context)) {
        release(request);
        return;
      }
      if (activeRequest !== request) return;
      const started = await speak(textToRead, DEFAULT_TTS_VOICE, featureContext);
      // Başlatma sürerken durdurulan veya yenisiyle değiştirilen istek durumu değiştirmez.
      if (activeRequest !== request) return;
      // Backend, hazırlık sırasında stop/yeni istek geldiyse süreci hiç başlatmaz.
      if (started?.cancelled) {
        release(request);
        return;
      }
      request.generation = Number.isSafeInteger(started?.generation) ? started.generation : null;
      if (started?.voice_fallback) {
        onToast(`${DEFAULT_TTS_VOICE} sesi kurulu değil; sistem varsayılan sesi kullanılıyor.`);
      }
      const early = earlyFinished;
      earlyFinished = null;
      if (early && early.generation === request.generation) settle(request, early);
    } catch (error) {
      console.error("TTS hatası:", error);
      // Durdurulmuş veya yenisiyle değiştirilmiş isteğin hatası kullanıcıya gösterilmez.
      if (activeRequest !== request) return;
      release(request);
      onToast("Seslendirme hatası: " + error.toString());
    }
  }

  async function stopSpeaking() {
    try {
      return await stop(featureContext, () => {
        activeRequest = null;
        earlyFinished = null;
        getContext().viewer.isSpeaking = false;
        onToast("Sesli okuma durduruldu.");
      });
    } catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  return Object.freeze({ speakCurrentPage, stopSpeaking, start, destroy });
}
