import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const pdfStateMocks = vi.hoisted(() => ({ startSearchablePdfExport: vi.fn() }));
const ocrStateMocks = vi.hoisted(() => ({ getOcrLanguages: vi.fn() }));
const sessionMocks = vi.hoisted(() => ({ getSessionFormInfo: vi.fn() }));
const dialogMocks = vi.hoisted(() => ({ save: vi.fn() }));

vi.mock("../pdfState.js", () => pdfStateMocks);
vi.mock("../ocrDocumentState.js", () => ocrStateMocks);
vi.mock("../services/backend/session.js", () => sessionMocks);
vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);

import SearchablePdfModal from "./SearchablePdfModal.svelte";

const REPORT = {
  processedPages: [0, 1],
  skippedPages: [{ pageIndex: 2, reason: "has_text", detail: null }],
  wordsWritten: 42,
  wordsSkipped: 0,
  signaturesInvalidated: false,
};

function deferredJob() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { handle: { jobId: "job", promise, cancel: vi.fn(() => Promise.resolve()) }, resolve, reject };
}

const saveButton = () => screen.getByRole("button", { name: "Kopyayı kaydet…" });

describe("SearchablePdfModal", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    pdfStateMocks.startSearchablePdfExport.mockReset();
    ocrStateMocks.getOcrLanguages.mockReset().mockResolvedValue(["eng", "osd", "tur"]);
    sessionMocks.getSessionFormInfo.mockReset().mockResolvedValue({ signature_count: 0 });
    dialogMocks.save.mockReset().mockResolvedValue("/tmp/rapor_aranabilir.pdf");
  });

  afterEach(() => {
    cleanup();
    Element.prototype.animate = originalAnimate;
  });

  it("selects Turkish, saves every scanned page and reports the result", async () => {
    const job = deferredJob();
    pdfStateMocks.startSearchablePdfExport.mockReturnValue(job.handle);
    const onsaved = vi.fn();
    const onopen = vi.fn();
    const onclose = vi.fn();
    render(SearchablePdfModal, { props: { sessionId: "s", contentGeneration: 3, fileName: "rapor.pdf", pageCount: 3, onsaved, onopen, onclose } });

    await waitFor(() => expect(document.getElementById("searchable-lang").value).toBe("tur"));
    expect([...document.getElementById("searchable-lang").options].map((option) => option.value)).toEqual(["eng", "tur"]);
    await fireEvent.click(saveButton());

    await waitFor(() => expect(pdfStateMocks.startSearchablePdfExport).toHaveBeenCalledWith("s", 3, "/tmp/rapor_aranabilir.pdf", "tur", null));
    expect(dialogMocks.save).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: "rapor_aranabilir.pdf" }));
    expect(screen.getByRole("button", { name: "İptal" })).toBeTruthy();
    job.resolve(REPORT);

    expect(await screen.findByText(/2 sayfaya metin katmanı eklendi \(42 kelime\)\. 1 sayfa atlandı\./)).toBeTruthy();
    expect(screen.getByText("Atlandı, zaten metin katmanı var: 3")).toBeTruthy();
    expect(onsaved).toHaveBeenCalledWith(expect.stringMatching(/42 kelime/));
    await fireEvent.click(screen.getByRole("button", { name: "Kopyayı aç" }));
    expect(onclose).toHaveBeenCalled();
    expect(onopen).toHaveBeenCalledWith("/tmp/rapor_aranabilir.pdf");
  });

  it("sends the page range as physical pages and rejects an invalid range before saving", async () => {
    const job = deferredJob();
    pdfStateMocks.startSearchablePdfExport.mockReturnValue(job.handle);
    render(SearchablePdfModal, { props: { sessionId: "s", contentGeneration: 1, pageCount: 4, pageOrder: [3, 2, 1, 0] } });
    await waitFor(() => expect(saveButton().disabled).toBe(false));
    await fireEvent.click(screen.getByLabelText("Sayfa aralığı", { selector: "input[type=radio]" }));

    await fireEvent.input(screen.getByRole("textbox", { name: "Sayfa aralığı" }), { target: { value: "9" } });
    await fireEvent.click(saveButton());
    expect((await screen.findByRole("alert")).textContent).toMatch(/sınır dışında/);
    expect(dialogMocks.save).not.toHaveBeenCalled();

    await fireEvent.input(screen.getByRole("textbox", { name: "Sayfa aralığı" }), { target: { value: "1-2" } });
    await fireEvent.click(saveButton());
    await waitFor(() => expect(pdfStateMocks.startSearchablePdfExport).toHaveBeenCalledWith("s", 1, "/tmp/rapor_aranabilir.pdf", "tur", [3, 2]));
    job.resolve(REPORT);
  });

  it("cancels the running job and says nothing was written", async () => {
    const job = deferredJob();
    job.handle.cancel.mockImplementation(() => {
      job.reject(Object.assign(new Error("Session job iptal edildi"), { code: "SESSION_JOB_CANCELLED" }));
      return Promise.resolve();
    });
    pdfStateMocks.startSearchablePdfExport.mockReturnValue(job.handle);
    const onsaved = vi.fn();
    render(SearchablePdfModal, { props: { sessionId: "s", contentGeneration: 1, pageCount: 2, onsaved } });
    await waitFor(() => expect(saveButton().disabled).toBe(false));
    await fireEvent.click(saveButton());
    await fireEvent.click(await screen.findByRole("button", { name: "İptal" }));

    expect((await screen.findByRole("alert")).textContent).toBe("İşlem iptal edildi; hiçbir dosya yazılmadı.");
    expect(job.handle.cancel).toHaveBeenCalledTimes(1);
    expect(onsaved).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "İptal" })).toBeNull();
  });

  it("cancels and hides the result when the document changes while running", async () => {
    const job = deferredJob();
    pdfStateMocks.startSearchablePdfExport.mockReturnValue(job.handle);
    const onsaved = vi.fn();
    const view = render(SearchablePdfModal, { props: { sessionId: "s", contentGeneration: 1, pageCount: 2, onsaved } });
    await waitFor(() => expect(saveButton().disabled).toBe(false));
    await fireEvent.click(saveButton());
    await waitFor(() => expect(pdfStateMocks.startSearchablePdfExport).toHaveBeenCalled());

    await view.rerender({ sessionId: "s", contentGeneration: 2, pageCount: 2, onsaved });
    await waitFor(() => expect(job.handle.cancel).toHaveBeenCalled());
    job.resolve(REPORT); // a late success from the old document

    expect(await screen.findByText("Belge iş sürerken değişti; sonuç gösterilmiyor.")).toBeTruthy();
    expect(screen.queryByText(/metin katmanı eklendi/)).toBeNull();
    expect(onsaved).not.toHaveBeenCalled();
  });

  it("explains how to install Tesseract when it is missing and does not run", async () => {
    ocrStateMocks.getOcrLanguages.mockRejectedValue(new Error("Tesseract bulunamadı"));
    render(SearchablePdfModal, { props: { sessionId: "s", contentGeneration: 1, pageCount: 2 } });
    expect((await screen.findByRole("alert")).textContent).toMatch(/Tesseract bulunamadı.*brew install tesseract tesseract-lang/);
    expect(saveButton().disabled).toBe(true);
  });

  it("warns that signatures become invalid in the copy", async () => {
    sessionMocks.getSessionFormInfo.mockResolvedValue({ signature_count: 2 });
    render(SearchablePdfModal, { props: { sessionId: "s", contentGeneration: 1, pageCount: 2 } });
    expect((await screen.findByRole("note")).textContent).toMatch(/imzalar kopyada geçersiz/);
  });

  it("shows the backend's language guidance", async () => {
    pdfStateMocks.startSearchablePdfExport.mockReturnValue({
      jobId: "job",
      promise: Promise.reject(Object.assign(new Error("Tesseract dil verisi kurulu değil: tur."), { code: "OCR_LANGUAGE_MISSING" })),
      cancel: vi.fn(),
    });
    render(SearchablePdfModal, { props: { sessionId: "s", contentGeneration: 1, pageCount: 2 } });
    await waitFor(() => expect(saveButton().disabled).toBe(false));
    await fireEvent.click(saveButton());
    expect((await screen.findByRole("alert")).textContent).toMatch(/kurulu değil: tur\. Kurulum:/);
  });
});
