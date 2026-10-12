import { readPageText } from "./pageTextSource.js";

/**
 * Coordinates Bionic Reading text preparation without becoming a reactive text owner.
 * Bionic Reading covers the current page only and labels where the text came from.
 */
export function createReadingPresentationController({ getContext, extractText, performOcr, featureContext, onToast }) {
  function isCurrent(context) {
    const current = getContext();
    return current.sessionId === context.sessionId && current.contentGeneration === context.contentGeneration;
  }

  async function openBionicReading() {
    const context = getContext();
    const { viewer, ui } = context;
    if (!context.isPdf || !context.sessionId || context.pageIndex < 0) {
      onToast("Bağımsız görseller için OCR bu sürümde desteklenmiyor.");
      return;
    }
    onToast("Biyonik okuma metni hazırlanıyor...");
    try {
      const { text, method } = await readPageText(context, { extractText, performOcr, featureContext });
      if (!isCurrent(context)) return;
      viewer.bionicText = text;
      viewer.bionicSource = { pageNumber: context.displayPageIndex + 1, method };
      ui.isBionicModalOpen = true;
    } catch (error) {
      console.error("Biyonik metin hatası:", error);
      onToast("Biyonik metin hazırlanamadı: " + error.toString());
    }
  }

  return Object.freeze({ openBionicReading });
}
