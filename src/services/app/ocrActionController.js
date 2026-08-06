/** Coordinates guarded OCR with captured session identity and stale-result rejection. */
export function createOcrActionController({ getContext, runOcr, guard, featureContext, reportUnavailable, onToast, present }) {
  async function run() {
    try {
      return await guard("macos_vision_ocr", featureContext, execute);
    } catch (error) { if (!reportUnavailable(error)) throw error; }
  }
  async function execute() {
    const context = getContext();
    if (!context.isPdf || !context.sessionId) { onToast("Bağımsız görseller için OCR bu sürümde desteklenmiyor."); return; }
    const { sessionId, contentGeneration, pageIndex, viewer } = context;
    onToast("Apple Vision OCR çalıştırılıyor...");
    try {
      const text = await runOcr(sessionId, contentGeneration, pageIndex, featureContext);
      const current = getContext();
      if (current.sessionId !== sessionId || current.contentGeneration !== contentGeneration) return;
      viewer.documentTextContext = text;
      onToast("OCR tamamlandı! Metin taptaze tanındı.");
      present(pageIndex, text);
    } catch (error) { console.error("OCR hatası:", error); onToast("OCR yürütülemedi: " + error); }
  }
  return Object.freeze({ run });
}
