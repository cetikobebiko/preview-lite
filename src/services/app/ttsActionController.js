import { readPageText } from "./pageTextSource.js";

/** Coordinates TTS request ownership, page-text/OCR fallback, and speaking-state cleanup. */
export function createTtsActionController({ getContext, extractText, performOcr, speak, stop, guard, featureContext, reportUnavailable, onToast }) {
  function isCurrent(context) {
    const current = getContext();
    return current.sessionId === context.sessionId && current.contentGeneration === context.contentGeneration;
  }

  async function speakCurrentPage() {
    try { return await guard("text_to_speech", featureContext, executeSpeak); }
    catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  async function executeSpeak() {
    const context = getContext();
    const { viewer } = context;
    try {
      onToast("Sayfa metni seslendiriliyor (macOS TTS)...");
      viewer.isSpeaking = true;
      if (!context.isPdf || !context.sessionId || context.pageIndex < 0) {
        viewer.isSpeaking = false;
        onToast("Bağımsız görseller için OCR bu sürümde desteklenmiyor.");
        return;
      }
      const { text: textToRead } = await readPageText(context, { extractText, performOcr, featureContext });
      if (!isCurrent(context)) {
        viewer.isSpeaking = false;
        return;
      }
      await speak(textToRead, "Yelda", featureContext);
    } catch (error) {
      console.error("TTS hatası:", error);
      viewer.isSpeaking = false;
      onToast("Seslendirme hatası: " + error.toString());
    }
  }

  async function stopSpeaking() {
    try {
      return await stop(featureContext, () => {
        getContext().viewer.isSpeaking = false;
        onToast("Sesli okuma durduruldu.");
      });
    } catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  return Object.freeze({ speakCurrentPage, stopSpeaking });
}
