import { describe, expect, it, vi } from "vitest";
import { createNotesExportController, NOTES_EXPORT_EMPTY } from "./notesExportController.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function failure(code, message = code) {
  return Object.assign(new Error(message), { code });
}

function setup({ savePath = "/out/rapor_Notlar.md", context = {} } = {}) {
  const state = {
    isPdf: true, sessionId: "session-1", contentGeneration: 4, fileName: "rapor.pdf", pageCount: 12,
    ...context,
  };
  const job = deferred();
  const order = [];
  const handle = { jobId: "job-1", promise: job.promise, cancel: vi.fn(async () => {}) };
  const saveFile = vi.fn(async () => { order.push("dialog"); return savePath; });
  const exportNotes = vi.fn(() => { order.push("backend"); return handle; });
  const onToast = vi.fn((message) => order.push(`toast:${message}`));
  const onError = vi.fn();
  const onExportComplete = vi.fn();
  const controller = createNotesExportController({
    getContext: () => ({ ...state }),
    saveFile, exportNotes, onToast, onError, onExportComplete,
    now: () => ({ getTimezoneOffset: () => -180 }),
  });
  return { state, job, handle, order, saveFile, exportNotes, onToast, onError, onExportComplete, controller };
}

describe("notesExportController", () => {
  it("shows the success toast only after the backend reports success", async () => {
    const t = setup();
    const running = t.controller.exportNotesMarkdown();
    await vi.waitFor(() => expect(t.exportNotes).toHaveBeenCalled());
    expect(t.saveFile).toHaveBeenCalledWith(expect.objectContaining({
      defaultPath: "rapor_Notlar.md",
      filters: [{ name: "Markdown Belgesi (*.md)", extensions: ["md"] }],
    }));
    expect(t.exportNotes).toHaveBeenCalledWith("session-1", 4, "/out/rapor_Notlar.md", { utcOffsetMinutes: 180 });
    expect(t.onToast).not.toHaveBeenCalled();
    expect(t.onExportComplete).not.toHaveBeenCalled();

    t.job.resolve(null);
    await running;
    expect(t.order).toEqual(["dialog", "backend", "toast:Notlar Markdown olarak dışa aktarıldı."]);
    expect(t.onExportComplete).toHaveBeenCalledWith("md", 12);
    expect(t.onError).not.toHaveBeenCalled();
  });

  it("does nothing when the save dialog is cancelled", async () => {
    const t = setup({ savePath: null });
    await t.controller.exportNotesMarkdown();
    expect(t.exportNotes).not.toHaveBeenCalled();
    expect(t.onToast).not.toHaveBeenCalled();
    expect(t.onError).not.toHaveBeenCalled();
    expect(t.onExportComplete).not.toHaveBeenCalled();
  });

  it("reports an empty document with its own message and no export event", async () => {
    const t = setup();
    t.job.reject(failure(NOTES_EXPORT_EMPTY, "Belgede dışa aktarılacak açıklama yok"));
    await t.controller.exportNotesMarkdown();
    expect(t.onToast).toHaveBeenCalledWith("Dışa aktarılacak not yok.");
    expect(t.onError).not.toHaveBeenCalled();
    expect(t.onExportComplete).not.toHaveBeenCalled();
  });

  it("shows other failures through exportErrorMessage", async () => {
    const t = setup();
    t.job.reject(failure("SESSION_JOB_WORKER_MEMORY_LIMIT", "Bellek aşıldı."));
    await t.controller.exportNotesMarkdown();
    expect(t.onError).toHaveBeenCalledWith(
      "Notlar dışa aktarılamadı: İşlem PDF motorunun bellek sınırını aştığı için durduruldu. Bellek aşıldı.",
    );
    expect(t.onToast).not.toHaveBeenCalled();
  });

  it("skips the toast when the document changed before the result arrived", async () => {
    const t = setup();
    const running = t.controller.exportNotesMarkdown();
    await vi.waitFor(() => expect(t.exportNotes).toHaveBeenCalled());
    t.state.sessionId = "session-2";
    t.job.resolve(null);
    await running;
    // The file is written by then; only the stale toast is dropped.
    expect(t.onToast).not.toHaveBeenCalled();
    expect(t.onExportComplete).toHaveBeenCalledWith("md", 12);
  });

  it("skips the empty-notes message for a document that was edited meanwhile", async () => {
    const t = setup();
    const running = t.controller.exportNotesMarkdown();
    await vi.waitFor(() => expect(t.exportNotes).toHaveBeenCalled());
    t.state.contentGeneration = 5;
    t.job.reject(failure(NOTES_EXPORT_EMPTY));
    await running;
    expect(t.onToast).not.toHaveBeenCalled();
  });

  it("does not start when the document changed while the dialog was open", async () => {
    const t = setup();
    t.saveFile.mockImplementation(async () => { t.state.sessionId = "session-2"; return "/out/a.md"; });
    await t.controller.exportNotesMarkdown();
    expect(t.exportNotes).not.toHaveBeenCalled();
    expect(t.onToast).toHaveBeenCalledWith("Belge değiştiği için notlar dışa aktarılmadı.");
  });

  it("requires an open PDF session", async () => {
    const t = setup({ context: { isPdf: false, sessionId: null } });
    await t.controller.exportNotesMarkdown();
    expect(t.saveFile).not.toHaveBeenCalled();
    expect(t.onToast).toHaveBeenCalledWith("Notları dışa aktarmak için açık bir PDF gerekir.");
  });

  it("runs one export at a time and cancels the running job on destroy, silently", async () => {
    const t = setup();
    const running = t.controller.exportNotesMarkdown();
    await vi.waitFor(() => expect(t.exportNotes).toHaveBeenCalled());
    await t.controller.exportNotesMarkdown();
    expect(t.exportNotes).toHaveBeenCalledTimes(1);
    expect(t.onToast).toHaveBeenCalledWith("Not dışa aktarma zaten sürüyor.");

    t.onToast.mockClear();
    t.controller.destroy();
    expect(t.handle.cancel).toHaveBeenCalledTimes(1);
    t.job.reject(failure("SESSION_JOB_CANCELLED"));
    await running;
    expect(t.onToast).not.toHaveBeenCalled();
    expect(t.onError).not.toHaveBeenCalled();
  });
});
