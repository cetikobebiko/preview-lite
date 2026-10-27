import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const dialogMocks = vi.hoisted(() => ({
  ask: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
}));
const backendMocks = vi.hoisted(() => ({
  exportSessionDocument: vi.fn(),
  exportSessionPagesAsImages: vi.fn(),
  exportSessionTextToFile: vi.fn(),
  printSessionPdf: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
vi.mock("../services/backend/index.js", () => backendMocks);

import ExportModal from "./ExportModal.svelte";

const conflictError = (names) =>
  Object.assign(new Error("Hedef klasörde aynı adlı dosya zaten var"), {
    code: "SESSION_JOB_OUTPUT_CONFLICT",
    jobOutcome: { status: "failed", error: { code: "SESSION_JOB_OUTPUT_CONFLICT", conflictingNames: names } },
  });

describe("ExportModal image export overwrite confirmation", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    dialogMocks.open.mockResolvedValue("/out");
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
  });

  function renderModal() {
    const onsuccess = vi.fn();
    const onclose = vi.fn();
    render(ExportModal, { props: { sessionId: "session-1", contentGeneration: 7, maxPages: 2, onsuccess, onclose } });
    return { onsuccess, onclose };
  }

  it("retries with overwrite after the user confirms the conflicting names", async () => {
    backendMocks.exportSessionPagesAsImages
      .mockRejectedValueOnce(conflictError(["sayfa_1.png"]))
      .mockResolvedValueOnce(["/out/sayfa_1.png", "/out/sayfa_2.png"]);
    dialogMocks.ask.mockResolvedValue(true);
    const { onsuccess, onclose } = renderModal();

    await fireEvent.click(screen.getByText("Dışa Aktar", { selector: "button" }));

    await waitFor(() => expect(onsuccess).toHaveBeenCalledWith("Görseller başarıyla dışa aktarıldı!"));
    expect(dialogMocks.ask).toHaveBeenCalledWith(expect.stringContaining("• sayfa_1.png"), expect.objectContaining({ okLabel: "Üzerine Yaz" }));
    expect(backendMocks.exportSessionPagesAsImages.mock.calls.map((call) => call.at(-1))).toEqual([false, true]);
    expect(onclose).toHaveBeenCalled();
  });

  it("keeps existing files when the user declines", async () => {
    backendMocks.exportSessionPagesAsImages.mockRejectedValueOnce(conflictError(["sayfa_1.png"]));
    dialogMocks.ask.mockResolvedValue(false);
    const { onsuccess, onclose } = renderModal();

    await fireEvent.click(screen.getByText("Dışa Aktar", { selector: "button" }));

    await waitFor(() => expect(screen.getByText(/mevcut dosyalara dokunulmadı/)).toBeTruthy());
    expect(backendMocks.exportSessionPagesAsImages).toHaveBeenCalledTimes(1);
    expect(onsuccess).not.toHaveBeenCalled();
    expect(onclose).not.toHaveBeenCalled();
  });
});

describe("ExportModal page range validation", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    dialogMocks.open.mockResolvedValue("/out");
    backendMocks.exportSessionPagesAsImages.mockResolvedValue([]);
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
  });

  async function submitImageExport(text, extraProps = {}) {
    const onsuccess = vi.fn();
    render(ExportModal, { props: { sessionId: "session-1", contentGeneration: 7, maxPages: 10, onsuccess, onclose: vi.fn(), ...extraProps } });
    await fireEvent.input(screen.getByLabelText(/Sayfa Aralığı/), { target: { value: text } });
    await fireEvent.click(screen.getByText("Dışa Aktar", { selector: "button" }));
    return { onsuccess };
  }

  it.each([
    ["5-3", '"5-3" ters aralık'],
    ["1, 11", '"11" sınır dışında'],
    ["0", '"0" sınır dışında'],
    ["2.5", '"2.5" geçersiz'],
    ["1,,2", "2. parça boş"],
  ])("rejects %j instead of silently fixing it", async (text, message) => {
    await submitImageExport(text);

    expect(screen.getByRole("alert").textContent).toContain(message);
    expect(dialogMocks.open).not.toHaveBeenCalled();
    expect(backendMocks.exportSessionPagesAsImages).not.toHaveBeenCalled();
  });

  it("exports sorted unique 0-based pages", async () => {
    const { onsuccess } = await submitImageExport("8-9, 2, 1-3");

    await waitFor(() => expect(onsuccess).toHaveBeenCalled());
    expect(backendMocks.exportSessionPagesAsImages.mock.calls[0][2]).toEqual([0, 1, 2, 7, 8]);
  });

  it("exports every page when the range is blank", async () => {
    const { onsuccess } = await submitImageExport("   ");

    await waitFor(() => expect(onsuccess).toHaveBeenCalled());
    expect(backendMocks.exportSessionPagesAsImages.mock.calls[0][2]).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
  });

  it("maps displayed page numbers through pageOrder", async () => {
    const { onsuccess } = await submitImageExport("1, 3", { maxPages: 3, pageOrder: [2, 0, 1] });

    await waitFor(() => expect(onsuccess).toHaveBeenCalled());
    expect(backendMocks.exportSessionPagesAsImages.mock.calls[0][2]).toEqual([2, 1]);
  });

  it("exports all pages in display order when blank and reordered", async () => {
    const { onsuccess } = await submitImageExport("", { maxPages: 3, pageOrder: [2, 0, 1] });

    await waitFor(() => expect(onsuccess).toHaveBeenCalled());
    expect(backendMocks.exportSessionPagesAsImages.mock.calls[0][2]).toEqual([2, 0, 1]);
  });
});

describe("ExportModal text export", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
  });

  async function submitTextExport() {
    const onsuccess = vi.fn();
    render(ExportModal, { props: { sessionId: "session-1", contentGeneration: 7, maxPages: 3, onsuccess } });
    await fireEvent.click(screen.getByText("Metin (StExt)"));
    await fireEvent.input(screen.getByLabelText(/Sayfa Aralığı/), { target: { value: "1, 3" } });
    await fireEvent.click(screen.getByText("Dışa Aktar", { selector: "button" }));
    return { onsuccess };
  }

  it("hands the chosen path to the backend instead of writing from the webview", async () => {
    dialogMocks.save.mockResolvedValue("/out/notes.md");
    backendMocks.exportSessionTextToFile.mockResolvedValue(null);
    const { onsuccess } = await submitTextExport();

    await waitFor(() => expect(onsuccess).toHaveBeenCalledWith("Metin başarıyla dışa aktarıldı!"));
    expect(backendMocks.exportSessionTextToFile).toHaveBeenCalledWith("session-1", 7, [0, 2], "markdown", "/out/notes.md");
  });

  it("shows the backend error and reports no success when the write is rejected", async () => {
    dialogMocks.save.mockResolvedValue("/out/notes.pdf");
    backendMocks.exportSessionTextToFile.mockRejectedValue(
      Object.assign(new Error("Metin dışa aktarma hedefi .txt veya .md uzantılı olmalı"), { code: "TEXT_EXPORT_TARGET_INVALID" }),
    );
    const { onsuccess } = await submitTextExport();

    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain(".txt veya .md"));
    expect(onsuccess).not.toHaveBeenCalled();
  });

  it("does nothing when the save dialog is cancelled", async () => {
    dialogMocks.save.mockResolvedValue(null);
    const { onsuccess } = await submitTextExport();

    await waitFor(() => expect(dialogMocks.save).toHaveBeenCalled());
    expect(backendMocks.exportSessionTextToFile).not.toHaveBeenCalled();
    expect(onsuccess).not.toHaveBeenCalled();
  });
});

describe("ExportModal document export", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
  });

  it("asks for a file of the chosen format and exports the selected pages", async () => {
    dialogMocks.save.mockResolvedValue("/out/rapor.pptx");
    backendMocks.exportSessionDocument.mockResolvedValue(null);
    const onsuccess = vi.fn();
    render(ExportModal, { props: { sessionId: "session-1", contentGeneration: 7, maxPages: 3, onsuccess } });
    await fireEvent.click(screen.getByText("Belge"));
    await fireEvent.change(screen.getByLabelText("Belge Formatı"), { target: { value: "pptx" } });
    await fireEvent.input(screen.getByLabelText(/Sayfa Aralığı/), { target: { value: "2-3" } });
    await fireEvent.click(screen.getByText("Dışa Aktar", { selector: "button" }));

    await waitFor(() => expect(onsuccess).toHaveBeenCalledWith("Belge başarıyla dışa aktarıldı!"));
    expect(dialogMocks.save).toHaveBeenCalledWith(expect.objectContaining({
      defaultPath: "disa_aktarim.pptx",
      filters: [{ name: "PowerPoint", extensions: ["pptx"] }],
    }));
    expect(backendMocks.exportSessionDocument).toHaveBeenCalledWith("session-1", 7, [1, 2], "pptx", "/out/rapor.pptx");
  });

  it("does nothing when the save dialog is cancelled and shows backend errors", async () => {
    dialogMocks.save.mockResolvedValueOnce(null);
    render(ExportModal, { props: { sessionId: "session-1", contentGeneration: 7, maxPages: 1 } });
    await fireEvent.click(screen.getByText("Belge"));
    await fireEvent.click(screen.getByText("Dışa Aktar", { selector: "button" }));
    await waitFor(() => expect(dialogMocks.save).toHaveBeenCalled());
    expect(backendMocks.exportSessionDocument).not.toHaveBeenCalled();

    dialogMocks.save.mockResolvedValueOnce("/out/a.docx");
    backendMocks.exportSessionDocument.mockRejectedValue(
      Object.assign(new Error("Belgenin metni dışa aktarma sınırını aşıyor"), { code: "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED" }),
    );
    await fireEvent.click(screen.getByText("Dışa Aktar", { selector: "button" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Belgenin metni dışa aktarma sınırını aşıyor"));
  });
});

describe("ExportModal print layout", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    backendMocks.printSessionPdf.mockResolvedValue(null);
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
  });

  async function openPrint() {
    const onsuccess = vi.fn();
    render(ExportModal, { props: { sessionId: "session-1", contentGeneration: 7, maxPages: 6, onsuccess } });
    await fireEvent.click(screen.getByText("Yazdır", { selector: "[role=group] button" }));
    return { onsuccess };
  }

  async function print() {
    await fireEvent.click(screen.getByText("Yazdır", { selector: "button:not([aria-pressed])" }));
  }

  it("prints the pages as they are by default", async () => {
    const { onsuccess } = await openPrint();
    expect(screen.queryByLabelText("Kağıt")).toBeNull();
    await print();
    await waitFor(() => expect(onsuccess).toHaveBeenCalledWith("Yazdırma işlemi başlatıldı!"));
    expect(backendMocks.printSessionPdf).toHaveBeenCalledWith("session-1", 7, null, null);
  });

  it("sends a booklet without an orientation choice", async () => {
    await openPrint();
    await fireEvent.change(screen.getByLabelText("Sayfa Düzeni"), { target: { value: "booklet" } });
    expect(screen.queryByLabelText("Yön")).toBeNull();
    await fireEvent.change(screen.getByLabelText("Kağıt"), { target: { value: "A3" } });
    await fireEvent.change(screen.getByLabelText("Yüzler"), { target: { value: "front" } });
    await print();
    await waitFor(() => expect(backendMocks.printSessionPdf).toHaveBeenCalled());
    expect(backendMocks.printSessionPdf).toHaveBeenCalledWith("session-1", 7, null, {
      paper: "A3",
      orientation: "auto",
      mode: { kind: "booklet", sides: "front", rightBinding: false },
    });
  });

  it("sends pages per sheet and posters with their options", async () => {
    await openPrint();
    await fireEvent.change(screen.getByLabelText("Sayfa Düzeni"), { target: { value: "multiple" } });
    await fireEvent.change(screen.getByLabelText("Tabaka başına sayfa"), { target: { value: "4" } });
    await fireEvent.change(screen.getByLabelText("Yön"), { target: { value: "landscape" } });
    await fireEvent.input(screen.getByLabelText(/Sayfa Aralığı/), { target: { value: "2-3" } });
    await print();
    await waitFor(() => expect(backendMocks.printSessionPdf).toHaveBeenCalledTimes(1));
    expect(backendMocks.printSessionPdf).toHaveBeenLastCalledWith("session-1", 7, [1, 2], {
      paper: "A4",
      orientation: "landscape",
      mode: { kind: "multiple", perSheet: 4, order: "horizontal", border: false },
    });
  });

  it("converts the poster overlap from millimetres to points", async () => {
    await openPrint();
    await fireEvent.change(screen.getByLabelText("Sayfa Düzeni"), { target: { value: "poster" } });
    await fireEvent.input(screen.getByLabelText("Bindirme (mm)"), { target: { value: "25.4" } });
    await print();
    await waitFor(() => expect(backendMocks.printSessionPdf).toHaveBeenCalled());
    const layout = backendMocks.printSessionPdf.mock.calls[0][3];
    expect(layout.mode).toEqual({ kind: "poster", percent: 200, overlapPoints: 72, cutMarks: true });
  });
});
