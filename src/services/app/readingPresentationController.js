/** Coordinates Bionic Reading text preparation without becoming a reactive text owner. */
export function createReadingPresentationController({ getContext, performOcr, featureContext, onToast }) {
  function isCurrent(context) {
    const current = getContext();
    return current.sessionId === context.sessionId && current.contentGeneration === context.contentGeneration;
  }

  async function openBionicReading() {
    const context = getContext();
    const { viewer, ui } = context;
    onToast("Biyonik okuma metni hazırlanıyor...");
    try {
      let text = viewer.documentTextContext;
      if (!text) {
        if (!context.isPdf || !context.sessionId) {
          onToast("Bağımsız görseller için OCR bu sürümde desteklenmiyor.");
          return;
        }
        text = await performOcr(context.sessionId, context.contentGeneration, context.pageIndex, featureContext);
        if (!isCurrent(context)) return;
      }
      viewer.bionicText = text;
      ui.isBionicModalOpen = true;
    } catch (error) {
      console.error("Biyonik metin hatası:", error);
      onToast("Biyonik metin hazırlanamadı: " + error.toString());
    }
  }

  return Object.freeze({ openBionicReading });
}
