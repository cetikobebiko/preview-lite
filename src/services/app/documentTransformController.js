/** Coordinates guarded document transformations that create an output and may reopen it. */
export function createDocumentTransformController({
  getContext, openFile, saveFile, merge, sanitize, openDocument,
  guard, featureContext, reportUnavailable, onToast, onError,
}) {
  function isCurrent(context) {
    const current = getContext();
    return current.sessionId === context.sessionId && current.contentGeneration === context.contentGeneration;
  }

  async function mergePdf() {
    try { return await guard("page_merge_direct", featureContext, executeMerge); }
    catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  async function executeMerge() {
    try {
      const selected = await openFile({
        title: "Birleştirilecek İkinci PDF Belgesini Seçin", multiple: false,
        filters: [{ name: "PDF Belgesi", extensions: ["pdf"] }],
      });
      if (!selected) return;
      const secondPdfPath = typeof selected === "string" ? selected : selected.path;
      const targetPath = await saveFile({
        title: "Birleştirilen PDF Belgesini Kaydet", defaultPath: "Birleştirilmiş_Doküman.pdf",
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
      if (!targetPath) return;
      const context = getContext();
      if (!context.path) return;
      await merge([context.path, secondPdfPath], targetPath, featureContext);
      if (!isCurrent(context)) return;
      onToast("PDF belgeleri başarıyla birleştirildi!");
      await openDocument(targetPath);
    } catch (error) {
      console.error("PDF birleştirme hatası:", error);
      onError("Birleştirme başarısız: " + error.toString());
    }
  }

  async function sanitizeDocument(options) {
    try { return await guard("sanitization", featureContext, () => executeSanitize(options)); }
    catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  async function executeSanitize(options) {
    const context = getContext();
    if (!context.path || !context.sessionId) return;
    try {
      const targetPath = context.path.replace(".pdf", "_sanitized.pdf");
      await sanitize(context.sessionId, context.contentGeneration, targetPath, options, featureContext);
      if (!isCurrent(context)) return;
      onToast("Sınırlı temizleme çıktısı oluşturuldu; eksiksiz arındırma olarak kabul etmeyin.");
      await openDocument(targetPath);
    } catch (error) { onToast("Temizleme hatası: " + error); }
  }

  return Object.freeze({ mergePdf, sanitizeDocument });
}
