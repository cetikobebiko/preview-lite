import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const dialogMocks = vi.hoisted(() => ({
  ask: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
}));
const pdfStateMocks = vi.hoisted(() => ({
  mergeSessionPdfs: vi.fn(),
  splitSessionPdf: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
vi.mock("../pdfState.js", () => pdfStateMocks);
vi.mock("../featureRegistry.js", async (importOriginal) => {
  const original = await importOriginal();
  return {
    ...original,
    getFeaturePresentation: (...args) => ({ ...original.getFeaturePresentation(...args), runnable: true }),
  };
});

import MergeSplitModal from "./MergeSplitModal.svelte";

describe("MergeSplitModal split ranges", () => {
  const originalAnimate = Element.prototype.animate;
  const originalAlert = window.alert;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    window.alert = vi.fn();
    dialogMocks.open.mockResolvedValue("/out");
    pdfStateMocks.splitSessionPdf.mockResolvedValue(["/out/a.pdf"]);
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
    window.alert = originalAlert;
  });

  async function submitSplit(text, extraProps = {}) {
    const onclose = vi.fn();
    const onsuccess = vi.fn();
    render(MergeSplitModal, { props: { currentFilePath: "/doc.pdf", maxPages: 10, sessionId: "session-1", contentGeneration: 4, onclose, onsuccess, ...extraProps } });
    await fireEvent.click(screen.getByText("Böl", { selector: "button" }));
    await fireEvent.input(screen.getByLabelText(/Bölünecek Aralıklar/), { target: { value: text } });
    await fireEvent.click(screen.getByText("Böl ve Kaydet", { selector: "button" }));
    return { onclose, onsuccess };
  }

  it.each([
    ["5-3", '"5-3" ters aralık'],
    ["1-3-5", '"1-3-5" geçersiz'],
    ["2.5", '"2.5" geçersiz'],
    ["3a", '"3a" geçersiz'],
    [", ,", "1. parça boş"],
    ["11", '"11" sınır dışında'],
    ["", "Sayfa aralığı boş."],
  ])("shows %j inline without calling the backend", async (text, message) => {
    await submitSplit(text);

    expect(screen.getByRole("alert").textContent).toContain(message);
    expect(window.alert).not.toHaveBeenCalled();
    expect(dialogMocks.open).not.toHaveBeenCalled();
    expect(pdfStateMocks.splitSessionPdf).not.toHaveBeenCalled();
  });

  it("sends 0-based inclusive ranges in input order", async () => {
    const { onclose, onsuccess } = await submitSplit(" 8-10 , 1 - 3, 5 ");

    await waitFor(() => expect(onclose).toHaveBeenCalled());
    expect(onsuccess).toHaveBeenCalledWith("PDF başarıyla bölündü ve klasöre kaydedildi!");
    expect(window.alert).not.toHaveBeenCalled();
    expect(pdfStateMocks.splitSessionPdf).toHaveBeenCalledWith("session-1", 4, [[7, 9], [0, 2], [4, 4]], "/out", false);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("clears the previous error on a valid retry", async () => {
    await submitSplit("5-3");
    expect(screen.getByRole("alert")).toBeTruthy();

    await fireEvent.input(screen.getByLabelText(/Bölünecek Aralıklar/), { target: { value: "3-5" } });
    await fireEvent.click(screen.getByText("Böl ve Kaydet", { selector: "button" }));

    await waitFor(() => expect(pdfStateMocks.splitSessionPdf).toHaveBeenCalledWith("session-1", 4, [[2, 4]], "/out", false));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("maps displayed page numbers to physical ranges", async () => {
    // Display 1-3 shows physical 4-6, display 4-6 shows physical 1-3.
    const { onclose } = await submitSplit("1-3, 4-6", { maxPages: 6, pageOrder: [3, 4, 5, 0, 1, 2] });

    await waitFor(() => expect(onclose).toHaveBeenCalled());
    expect(pdfStateMocks.splitSessionPdf).toHaveBeenCalledWith("session-1", 4, [[3, 5], [0, 2]], "/out", false);
  });

  it("rejects a displayed range that is not contiguous in the file", async () => {
    await submitSplit("1-2", { maxPages: 3, pageOrder: [1, 0, 2] });

    expect(screen.getByRole("alert").textContent).toContain('"1-2" ekranda ardışık ama belgede ardışık değil');
    expect(pdfStateMocks.splitSessionPdf).not.toHaveBeenCalled();
  });

  it("shows backend split failures inline", async () => {
    pdfStateMocks.splitSessionPdf.mockRejectedValueOnce(Object.assign(new Error("Split range aralık dışında"), { code: "SESSION_JOB_INVALID_SPLIT_RANGES" }));
    const { onclose } = await submitSplit("1-3");

    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Bölme hatası: Split range aralık dışında"));
    expect(window.alert).not.toHaveBeenCalled();
    expect(onclose).not.toHaveBeenCalled();
  });

  it("keeps the modal open when the user declines overwriting", async () => {
    pdfStateMocks.splitSessionPdf.mockRejectedValueOnce(Object.assign(new Error("conflict"), {
      code: "SESSION_JOB_OUTPUT_CONFLICT",
      jobOutcome: { status: "failed", error: { code: "SESSION_JOB_OUTPUT_CONFLICT", conflictingNames: ["doc_sayfa_1-3.pdf"] } },
    }));
    dialogMocks.ask.mockResolvedValue(false);
    const { onclose, onsuccess } = await submitSplit("1-3");

    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("mevcut dosyalara dokunulmadı"));
    expect(onclose).not.toHaveBeenCalled();
    expect(onsuccess).not.toHaveBeenCalled();
  });

  it("clears the error when switching tabs", async () => {
    await submitSplit("5-3");
    await fireEvent.click(screen.getByText("Birleştir", { selector: "button" }));

    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("MergeSplitModal merge feedback", () => {
  const originalAnimate = Element.prototype.animate;
  const originalAlert = window.alert;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    window.alert = vi.fn();
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
    window.alert = originalAlert;
  });

  function renderMerge() {
    const onclose = vi.fn();
    const onsuccess = vi.fn();
    render(MergeSplitModal, { props: { currentFilePath: "/doc.pdf", maxPages: 10, sessionId: "session-1", contentGeneration: 4, onclose, onsuccess } });
    return { onclose, onsuccess };
  }

  it("asks for a second file inline", async () => {
    renderMerge();
    await fireEvent.click(screen.getByText("Birleştir ve Kaydet", { selector: "button" }));

    expect(screen.getByRole("alert").textContent).toBe("En az iki dosya seçmelisiniz.");
    expect(window.alert).not.toHaveBeenCalled();
    expect(dialogMocks.save).not.toHaveBeenCalled();
  });

  it("reports success through onsuccess and backend failures inline", async () => {
    dialogMocks.open.mockResolvedValue(["/other.pdf"]);
    dialogMocks.save.mockResolvedValue("/merged.pdf");
    pdfStateMocks.mergeSessionPdfs.mockRejectedValueOnce(new Error("disk dolu")).mockResolvedValueOnce(undefined);
    const { onclose, onsuccess } = renderMerge();

    await fireEvent.click(screen.getByText("+ Dosya Ekle", { selector: "button" }));
    await waitFor(() => expect(screen.getByText("other.pdf")).toBeTruthy());
    await fireEvent.click(screen.getByText("Birleştir ve Kaydet", { selector: "button" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Birleştirme hatası: disk dolu"));
    expect(onclose).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByText("Birleştir ve Kaydet", { selector: "button" }));
    await waitFor(() => expect(onsuccess).toHaveBeenCalledWith("Dosyalar başarıyla birleştirildi!"));
    expect(pdfStateMocks.mergeSessionPdfs).toHaveBeenLastCalledWith("session-1", 4, ["/other.pdf"], "/merged.pdf");
    expect(onclose).toHaveBeenCalled();
    expect(window.alert).not.toHaveBeenCalled();
  });
});
