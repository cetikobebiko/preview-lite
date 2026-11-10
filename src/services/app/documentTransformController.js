import { exportErrorMessage } from "./exportErrorMessage.js";
import { mergeTargetConflict } from "./mergeTargetGuard.js";

/** Coordinates guarded document transformations that create an output and may reopen it. */
export function createDocumentTransformController({
  getContext, getDocumentPath = () => "", openFile, saveFile, merge, openDocument,
  guard, featureContext, reportUnavailable, onToast, onError,
  // Plugin on_export_complete (fire-and-forget): (format, pageCount) after a written output.
  onExportComplete = () => {},
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
      // Kaynak, oturumun kaydedilmemiş halidir; diskteki dosya değil.
      if (!getContext().sessionId) {
        onError("Birleştirme başarısız: Aktif belge oturumu bulunamadı.");
        return;
      }
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
      const conflict = mergeTargetConflict(targetPath, {
        documentPath: getDocumentPath(),
        inputPaths: [secondPdfPath],
        platform: featureContext?.platform,
      });
      if (conflict) {
        onError(`Birleştirme başarısız: ${conflict}`);
        return;
      }
      const context = getContext();
      if (!context.sessionId) {
        onError("Birleştirme başarısız: Aktif belge oturumu bulunamadı.");
        return;
      }
      await merge(context.sessionId, context.contentGeneration, [secondPdfPath], targetPath);
      // page_count: pages of the open document in the output (the other file's count is not known here).
      if (Number.isInteger(context.pageCount)) onExportComplete("pdf", context.pageCount);
      if (!isCurrent(context)) return;
      onToast("PDF belgeleri başarıyla birleştirildi!");
      await openDocument(targetPath);
    } catch (error) {
      console.error("PDF birleştirme hatası:", error);
      onError("Birleştirme başarısız: " + exportErrorMessage(error));
    }
  }

  return Object.freeze({ mergePdf });
}
