import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const dialogMocks = vi.hoisted(() => ({
  ask: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
}));
const backendMocks = vi.hoisted(() => ({
  exportSessionPagesAsImages: vi.fn(),
  exportSessionTextToFile: vi.fn(),
  printSessionPdf: vi.fn(),
}));
const fsMocks = vi.hoisted(() => ({ writeTextFile: vi.fn() }));

vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
vi.mock("@tauri-apps/plugin-fs", () => fsMocks);
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
    expect(fsMocks.writeTextFile).not.toHaveBeenCalled();
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
