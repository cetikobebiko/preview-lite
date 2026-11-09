import { describe, expect, it, vi } from "vitest";
import { createDocumentTransformController } from "./documentTransformController.js";

function setup(overrides = {}) {
  const context = { sessionId: "session-a", contentGeneration: 3, pageCount: 6 };
  const guard = vi.fn(async (_feature, _runtime, action) => action());
  const merge = vi.fn(async () => undefined);
  const openDocument = vi.fn(async () => undefined);
  const openFile = vi.fn(async () => "/tmp/second.pdf");
  const saveFile = vi.fn(async () => "/tmp/combined.pdf");
  const onToast = vi.fn();
  const onError = vi.fn();
  const featureContext = { build: "production", platform: "macos" };
  const controller = createDocumentTransformController({
    getContext: () => ({ ...context }), getDocumentPath: () => "/Volumes/Docs/original.pdf",
    openFile, saveFile, merge, openDocument, guard, featureContext,
    reportUnavailable: () => false, onToast, onError, ...overrides,
  });
  return { controller, context, guard, merge, openDocument, openFile, saveFile, onToast, onError, featureContext };
}

describe("documentTransformController", () => {
  it("merges the session snapshot through mergeSessionPdfs and opens the output", async () => {
    const { controller, guard, merge, openDocument, onToast, onError, featureContext } = setup();
    await controller.mergePdf();
    expect(guard).toHaveBeenCalledWith("page_merge_direct", featureContext, expect.any(Function));
    expect(merge).toHaveBeenCalledTimes(1);
    expect(merge).toHaveBeenCalledWith("session-a", 3, ["/tmp/second.pdf"], "/tmp/combined.pdf");
    expect(onToast).toHaveBeenCalledWith("PDF belgeleri başarıyla birleştirildi!");
    expect(openDocument).toHaveBeenCalledWith("/tmp/combined.pdf");
    expect(onError).not.toHaveBeenCalled();
  });

  it("accepts an object selection from the open dialog", async () => {
    const { controller, merge } = setup({ openFile: vi.fn(async () => ({ path: "/tmp/obj.pdf" })) });
    await controller.mergePdf();
    expect(merge).toHaveBeenCalledWith("session-a", 3, ["/tmp/obj.pdf"], "/tmp/combined.pdf");
  });

  it.each([
    ["open dialog", { openFile: vi.fn(async () => null) }],
    ["save dialog", { saveFile: vi.fn(async () => null) }],
  ])("does nothing when the %s is cancelled", async (_label, overrides) => {
    const { controller, merge, openDocument, onToast, onError } = setup(overrides);
    await controller.mergePdf();
    expect(merge).not.toHaveBeenCalled();
    expect(openDocument).not.toHaveBeenCalled();
    expect(onToast).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it("skips the toast and reopen when the document generation advanced during the merge", async () => {
    const live = { sessionId: "session-a", contentGeneration: 3 };
    const merge = vi.fn(async () => { live.contentGeneration = 4; });
    const { controller, openDocument, onToast, onError } = setup({ getContext: () => ({ ...live }), merge });
    await controller.mergePdf();
    expect(merge).toHaveBeenCalledWith("session-a", 3, ["/tmp/second.pdf"], "/tmp/combined.pdf");
    expect(onToast).not.toHaveBeenCalled();
    expect(openDocument).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it("skips the toast and reopen when another session became active", async () => {
    const live = { sessionId: "session-a", contentGeneration: 3 };
    const merge = vi.fn(async () => { live.sessionId = "session-b"; live.contentGeneration = 1; });
    const { controller, openDocument, onToast } = setup({ getContext: () => ({ ...live }), merge });
    await controller.mergePdf();
    expect(onToast).not.toHaveBeenCalled();
    expect(openDocument).not.toHaveBeenCalled();
  });

  it.each([
    ["the open document itself", "/Volumes/Docs/original.pdf", "açık belgenin kendi dosyasına"],
    ["the open document with a different case and dot segments", "/volumes/DOCS/./x/../ORIGINAL.pdf", "açık belgenin kendi dosyasına"],
    ["the second PDF", "/tmp//second.pdf", "birleştirilen PDF dosyalarından birinin"],
  ])("refuses %s as the target", async (_label, target, message) => {
    const { controller, merge, openDocument, onToast, onError } = setup({ saveFile: vi.fn(async () => target) });
    await controller.mergePdf();
    expect(merge).not.toHaveBeenCalled();
    expect(openDocument).not.toHaveBeenCalled();
    expect(onToast).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining(message));
  });

  it("reports a missing session instead of returning silently", async () => {
    const { controller, merge, openFile, onError } = setup({
      getContext: () => ({ sessionId: null, contentGeneration: 0 }),
    });
    await controller.mergePdf();
    expect(openFile).not.toHaveBeenCalled();
    expect(merge).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith("Birleştirme başarısız: Aktif belge oturumu bulunamadı.");
  });

  it("reports a session that closed while the dialogs were open", async () => {
    let calls = 0;
    const { controller, merge, onError } = setup({
      getContext: () => (++calls === 1 ? { sessionId: "session-a", contentGeneration: 3 } : { sessionId: null, contentGeneration: 0 }),
    });
    await controller.mergePdf();
    expect(merge).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith("Birleştirme başarısız: Aktif belge oturumu bulunamadı.");
  });

  it("formats backend failures with the shared error helper", async () => {
    const failure = Object.assign(new Error("Ek merge girdisi açılamadı: PasswordError"), {
      code: "SESSION_JOB_MERGE_ADDITIONAL_INPUT_OPEN_FAILED",
      userMessage: "Ek merge girdisi açılamadı: PasswordError",
    });
    const { controller, openDocument, onError } = setup({ merge: vi.fn(async () => { throw failure; }) });
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    await controller.mergePdf();
    consoleError.mockRestore();
    expect(onError).toHaveBeenCalledWith("Birleştirme başarısız: Ek merge girdisi açılamadı: PasswordError");
    expect(onError.mock.calls[0][0]).not.toContain("Error:");
    expect(openDocument).not.toHaveBeenCalled();
  });

  it("does not expose a redact function", () => {
    const { controller } = setup();
    expect(controller.redact).toBeUndefined();
  });

  it("does not own sanitization policy or output lifecycle", () => {
    const { controller } = setup();
    expect(controller.sanitizeDocument).toBeUndefined();
  });
});

describe("documentTransformController plugin on_export_complete", () => {
  it("reports a palette merge as pdf with the open document's page count", async () => {
    const onExportComplete = vi.fn();
    const { controller } = setup({ onExportComplete });
    await controller.mergePdf();
    expect(onExportComplete).toHaveBeenCalledWith("pdf", 6);
  });

  it("does not report a failed merge", async () => {
    const onExportComplete = vi.fn();
    const { controller } = setup({ onExportComplete, merge: vi.fn(async () => { throw new Error("disk"); }) });
    await controller.mergePdf();
    expect(onExportComplete).not.toHaveBeenCalled();
  });
});
