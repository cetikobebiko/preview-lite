import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const batchMocks = vi.hoisted(() => ({
  scanBatchInputs: vi.fn(),
  startBatch: vi.fn(),
  cancelBatch: vi.fn(),
  getBatchStatus: vi.fn(),
  revealBatchOutputDir: vi.fn(),
}));
const dialogMocks = vi.hoisted(() => ({ open: vi.fn(), ask: vi.fn() }));
vi.mock("../services/backend/batch.js", () => batchMocks);
vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import BatchProcessingModal from "./BatchProcessingModal.svelte";

const file = (index, state = "pending", extra = {}) => ({
  index, inputPath: `/girdi/belge${index}.pdf`, state, outputs: [], reason: null, ...extra,
});
const counts = (completed = 0, skipped = 0, failed = 0, cancelled = 0) => ({ completed, skipped, failed, cancelled });
const status = (extra = {}) => ({
  batchId: "b1", state: "running", operation: "export_images", outputDir: "/çıktı", total: 2,
  lastSequence: 1, counts: counts(), files: [file(0, "running"), file(1)], warnings: [], reason: null, ...extra,
});

function fakeTracker({ existing = null, started = status() } = {}) {
  const tracker = {
    onChange: () => {},
    attach: vi.fn(async (id) => {
      const next = id ? started : existing;
      tracker.onChange(next);
      return next;
    }),
    dispose: vi.fn(),
    push: (next) => tracker.onChange(next),
  };
  const create = vi.fn(({ onChange }) => {
    tracker.onChange = onChange;
    return tracker;
  });
  return { tracker, create };
}

const SCAN = {
  files: [{ path: "/klasör/a.pdf", size: 2048 }, { path: "/klasör/b.pdf", size: 3 * 1024 * 1024 }],
  rejected: [{ path: "/klasör/bağ.pdf", code: "BATCH_INPUT_SYMLINK", message: "Sembolik bağlar toplu işleme alınmaz" }],
  truncated: false,
};

describe("BatchProcessingModal", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    for (const mock of Object.values(batchMocks)) mock.mockReset();
    dialogMocks.open.mockReset();
    batchMocks.scanBatchInputs.mockResolvedValue(SCAN);
    batchMocks.startBatch.mockResolvedValue({ batchId: "b1" });
    batchMocks.cancelBatch.mockResolvedValue(true);
  });

  afterEach(() => {
    cleanup();
    Element.prototype.animate = originalAnimate;
  });

  async function chooseFolder() {
    dialogMocks.open.mockResolvedValueOnce("/klasör");
    await fireEvent.click(await screen.findByRole("button", { name: "Klasör ekle…" }));
    await screen.findByText("a.pdf");
  }

  it("walks files → operation → output and starts a typed batch", async () => {
    const { tracker, create } = fakeTracker();
    render(BatchProcessingModal, { props: { createTracker: create } });

    expect(await screen.findByText("0 PDF seçildi (en fazla 500).")).toBeTruthy();
    expect(screen.getByRole("button", { name: "İleri" }).disabled).toBe(true);

    await chooseFolder();
    expect(batchMocks.scanBatchInputs).toHaveBeenCalledWith(["/klasör"], false);
    expect(screen.getByText("2 PDF seçildi (en fazla 500).")).toBeTruthy();
    expect(screen.getByText("1 dosya alınmadı")).toBeTruthy();

    await fireEvent.click(screen.getByLabelText("Alt klasörleri dahil et"));
    await waitFor(() => expect(batchMocks.scanBatchInputs).toHaveBeenLastCalledWith(["/klasör"], true));

    await fireEvent.click(screen.getByRole("button", { name: "b.pdf dosyasını çıkar" }));
    expect(screen.getByText("1 PDF seçildi (en fazla 500).")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "İleri" }));

    await fireEvent.change(document.getElementById("batch-operation"), { target: { value: "export_images" } });
    await fireEvent.input(document.getElementById("batch-image-dpi"), { target: { value: "10" } });
    expect(await screen.findByText("Çözünürlük 36–1200 DPI aralığında olmalı.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "İleri" }).disabled).toBe(true);
    await fireEvent.input(document.getElementById("batch-image-dpi"), { target: { value: "150" } });
    await fireEvent.click(screen.getByRole("button", { name: "İleri" }));

    expect(screen.getByText("Bir çıktı klasörü seçin.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Başlat" }).disabled).toBe(true);
    dialogMocks.open.mockResolvedValueOnce("/çıktı");
    await fireEvent.click(screen.getByRole("button", { name: "Klasör seç…" }));
    await screen.findByText("/çıktı");
    await fireEvent.change(document.getElementById("batch-conflict"), { target: { value: "overwrite" } });
    expect(screen.getByText(/Girdi dosyalarının ve bu toplu işlemin kendi çıktılarının üzerine hiçbir zaman yazılmaz/)).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Başlat" }));
    await waitFor(() => expect(batchMocks.startBatch).toHaveBeenCalledOnce());
    expect(batchMocks.startBatch).toHaveBeenCalledWith({
      inputs: ["/klasör/a.pdf"],
      operation: { kind: "export_images", format: "png", dpi: 150 },
      outputDir: "/çıktı",
      conflictPolicy: "overwrite",
    });
    expect(tracker.attach).toHaveBeenLastCalledWith("b1");
    expect(await screen.findByText("Sayfaları görsel olarak dışa aktar: 0 / 2")).toBeTruthy();
    expect(screen.getByText("İşleniyor: belge0.pdf")).toBeTruthy();
  });

  it("sends the security op and forgets the passwords after starting", async () => {
    const { create } = fakeTracker({ started: status({ operation: "security" }) });
    render(BatchProcessingModal, { props: { createTracker: create } });
    await chooseFolder();
    await fireEvent.click(screen.getByRole("button", { name: "İleri" }));
    await fireEvent.change(document.getElementById("batch-operation"), { target: { value: "security" } });
    await fireEvent.input(document.getElementById("batch-security-user"), { target: { value: "gizli-1234" } });
    expect(await screen.findByText("Açma parolaları uyuşmuyor.")).toBeTruthy();
    await fireEvent.input(document.getElementById("batch-security-user-confirm"), { target: { value: "gizli-1234" } });
    await fireEvent.click(screen.getByRole("button", { name: "İleri" }));
    dialogMocks.open.mockResolvedValueOnce("/çıktı");
    await fireEvent.click(screen.getByRole("button", { name: "Klasör seç…" }));
    await screen.findByText("/çıktı");
    await fireEvent.click(screen.getByRole("button", { name: "Başlat" }));
    await waitFor(() => expect(batchMocks.startBatch).toHaveBeenCalledOnce());
    const [request] = batchMocks.startBatch.mock.calls[0];
    expect(request.operation.kind).toBe("security");
    expect(request.operation.op).toMatchObject({ op: "protect", settings: { userPassword: "gizli-1234", method: "aes256" } });
    expect(request.operation.op.settings).not.toHaveProperty("userConfirm");
    await screen.findByText(/Parola korumalı ya da korumasız kopya: 0 \/ 2/);
    expect(document.body.innerHTML).not.toContain("gizli-1234");
  });

  it("shows progress, cancels, and switches to a filterable report", async () => {
    const { tracker, create } = fakeTracker({ existing: status() });
    render(BatchProcessingModal, { props: { createTracker: create } });
    expect(await screen.findByText("Sayfaları görsel olarak dışa aktar: 0 / 2")).toBeTruthy();

    tracker.push(status({ lastSequence: 2, counts: counts(1), files: [file(0, "completed", { outputs: ["/çıktı/belge0_sayfa_1.png"] }), file(1, "running")] }));
    expect(await screen.findByText("Sayfaları görsel olarak dışa aktar: 1 / 2")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "İptal et" }));
    expect(batchMocks.cancelBatch).toHaveBeenCalledWith("b1");

    tracker.push(status({
      state: "cancelled",
      lastSequence: 3,
      counts: counts(1, 0, 0, 1),
      files: [file(0, "completed", { outputs: ["/çıktı/belge0_sayfa_1.png"] }), file(1, "cancelled")],
    }));
    expect(await screen.findByText("Toplu işlem iptal edildi")).toBeTruthy();
    expect(screen.getByText("→ belge0_sayfa_1.png")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "İptal" }));
    const report = screen.getByRole("list", { name: "Sonuç raporu" });
    expect(report.textContent).toContain("belge1.pdf");
    expect(report.textContent).not.toContain("belge0.pdf");

    await fireEvent.click(screen.getByRole("button", { name: "Çıktı klasörünü aç" }));
    expect(batchMocks.revealBatchOutputDir).toHaveBeenCalledWith("b1");

    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const onToast = vi.fn();
    cleanup();
    render(BatchProcessingModal, { props: { createTracker: fakeTracker({ existing: status({ state: "completed", counts: counts(2), files: [file(0, "completed"), file(1, "completed")] }) }).create, onToast } });
    await fireEvent.click(await screen.findByRole("button", { name: "Raporu kopyala" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledOnce());
    expect(writeText.mock.calls[0][0]).toContain("Tamamlandı: 2, atlandı: 0, hata: 0, iptal: 0");
    expect(onToast).toHaveBeenCalledWith("Toplu işlem raporu panoya kopyalandı.");
  });

  it("restores a running batch on reopen and releases the listener when closed", async () => {
    const { tracker, create } = fakeTracker({ existing: status() });
    const onclose = vi.fn();
    const view = render(BatchProcessingModal, { props: { createTracker: create, onclose } });
    expect(await screen.findByText("İşleniyor: belge0.pdf")).toBeTruthy();
    expect(tracker.attach).toHaveBeenCalledWith(null);
    await fireEvent.click(screen.getByRole("button", { name: "Arka planda sürdür" }));
    expect(onclose).toHaveBeenCalled();
    view.unmount();
    expect(tracker.dispose).toHaveBeenCalledOnce();
  });

  // The flags mirror batch::tests::signature_loss_is_reported_per_file_for_pdf_copies_only
  // (sanitization/signature.pdf → true, minimal.pdf → false).
  it("marks completed files whose signatures are not valid in the output", async () => {
    const { create } = fakeTracker({
      existing: status({
        state: "completed",
        operation: "security",
        counts: counts(2),
        files: [
          file(0, "completed", { inputPath: "/girdi/imzali.pdf", outputs: ["/çıktı/imzali_korumalı.pdf"], signaturesInvalidated: true }),
          file(1, "completed", { inputPath: "/girdi/imzasiz.pdf", outputs: ["/çıktı/imzasiz_korumalı.pdf"], signaturesInvalidated: false }),
        ],
      }),
    });
    render(BatchProcessingModal, { props: { createTracker: create } });
    expect(await screen.findByText("Dijital imzaları çıktıda geçersiz olan dosya: 1")).toBeTruthy();
    const rows = [...screen.getByRole("list", { name: "Sonuç raporu" }).children];
    expect(rows[0].textContent).toContain("Dijital imzalar çıktıda geçersiz.");
    expect(rows[1].textContent).not.toContain("Dijital imzalar");
  });

  it("explains signature loss before optimize and security batches only", async () => {
    render(BatchProcessingModal, { props: { createTracker: fakeTracker().create } });
    await chooseFolder();
    await fireEvent.click(screen.getByRole("button", { name: "İleri" }));
    const note = /dijital imzalar kopyada geçersiz olur/;
    for (const [operation, shown] of [["optimize", true], ["security", true], ["export_text", false], ["export_images", false]]) {
      await fireEvent.change(document.getElementById("batch-operation"), { target: { value: operation } });
      expect(Boolean(screen.queryByText(note)), operation).toBe(shown);
    }
  });

  it("starts a new batch from the report", async () => {
    const { create } = fakeTracker({ existing: status({ state: "completed", counts: counts(2), files: [file(0, "completed"), file(1, "completed")] }) });
    render(BatchProcessingModal, { props: { createTracker: create } });
    await fireEvent.click(await screen.findByRole("button", { name: "Yeni toplu işlem" }));
    expect(screen.getByText("0 PDF seçildi (en fazla 500).")).toBeTruthy();
  });
});
