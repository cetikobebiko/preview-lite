import { exportErrorMessage } from "./exportErrorMessage.js";

/** Backend code: the document has no notes; nothing was written. */
export const NOTES_EXPORT_EMPTY = "NOTES_EXPORT_EMPTY";
const CANCELLED_CODES = new Set(["SESSION_JOB_CANCELLED", "SESSION_JOB_WORKER_CANCELLED"]);

/**
 * "Notları Markdown (.md) Olarak Dışa Aktar": save dialog → backend job → toast. The notes come
 * from the document's PDF annotations, read by the Rust job worker from the session snapshot;
 * the frontend sends no note content.
 */
export function createNotesExportController({
  getContext, saveFile, exportNotes, onToast, onError,
  // Plugin on_export_complete (fire-and-forget): (format, pageCount) after a written output.
  onExportComplete = () => {},
  now = () => new Date(),
}) {
  let activeHandle = null;

  const sameDocument = (a, b) =>
    a.sessionId === b.sessionId && a.contentGeneration === b.contentGeneration;

  async function exportNotesMarkdown() {
    const before = getContext();
    if (!before.isPdf || !before.sessionId) {
      onToast("Notları dışa aktarmak için açık bir PDF gerekir.");
      return;
    }
    if (activeHandle) {
      onToast("Not dışa aktarma zaten sürüyor.");
      return;
    }
    const baseName = (before.fileName || "belge").replace(/\.[^.]+$/, "");
    const targetPath = await saveFile({
      title: "Notları Markdown (.md) Olarak Kaydet",
      defaultPath: `${baseName}_Notlar.md`,
      filters: [{ name: "Markdown Belgesi (*.md)", extensions: ["md"] }],
    });
    if (!targetPath) return;

    const context = getContext();
    if (!context.isPdf || context.sessionId !== before.sessionId) {
      onToast("Belge değiştiği için notlar dışa aktarılmadı.");
      return;
    }
    const handle = exportNotes(context.sessionId, context.contentGeneration, targetPath, {
      utcOffsetMinutes: -now().getTimezoneOffset(),
    });
    activeHandle = handle;
    try {
      await handle.promise;
      if (Number.isInteger(context.pageCount)) onExportComplete("md", context.pageCount);
      // Stale guard: the file is already written when the result arrives. If the user switched
      // or edited the document meanwhile, the toast would describe a document no longer shown,
      // so it is skipped; the written file stays as it is.
      if (!sameDocument(context, getContext())) return;
      onToast("Notlar Markdown olarak dışa aktarıldı.");
    } catch (error) {
      if (CANCELLED_CODES.has(error?.code)) return;
      const current = sameDocument(context, getContext());
      if (error?.code === NOTES_EXPORT_EMPTY) {
        if (current) onToast("Dışa aktarılacak not yok.");
        return;
      }
      // A real failure is reported even for a stale document: the user asked for a file
      // that was not written.
      onError("Notlar dışa aktarılamadı: " + exportErrorMessage(error));
    } finally {
      if (activeHandle === handle) activeHandle = null;
    }
  }

  /** Cancels a running export (runtime teardown); its result is then ignored. */
  function destroy() {
    const handle = activeHandle;
    activeHandle = null;
    try {
      const result = handle?.cancel?.();
      if (typeof result?.catch === "function") result.catch(() => {});
    } catch {
      // Best effort.
    }
  }

  return Object.freeze({ exportNotesMarkdown, destroy });
}
