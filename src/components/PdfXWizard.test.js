import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { tick } from "svelte";
import { save } from "@tauri-apps/plugin-dialog";
import { exportPdfXAction } from "../services/actions.js";
import PdfXWizard from "./PdfXWizard.svelte";

vi.mock("../services/actions.js", () => ({
  exportPdfXAction: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));

vi.mock("../featureRegistry.js", () => ({
  getRuntimeFeatureContext: vi.fn(() => ({
    build: "development",
    platform: "macos",
  })),
  getFeaturePresentation: vi.fn(() => ({
    runnable: true,
    visible: false,
  })),
}));

function createDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function renderReadyWizard(props = {}) {
  return render(PdfXWizard, {
    props: {
      sessionId: "pdfx-session",
      contentGeneration: 3,
      isDocumentReady: true,
      ...props,
    },
  });
}

beforeEach(() => {
  exportPdfXAction.mockReset();
  save.mockReset();
  exportPdfXAction.mockResolvedValue(undefined);
  save.mockResolvedValue("/tmp/output.pdf");
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("PdfXWizard session contract", () => {
  it("doğrulanmış aktif session'ı PDF/X export action'ına gönderir", async () => {
    const { getByRole } = renderReadyWizard();

    await fireEvent.click(getByRole("button", { name: "Dışa Aktar" }));

    await waitFor(() => {
      expect(exportPdfXAction).toHaveBeenCalledWith(
        "pdfx-session",
        3,
        "/tmp/output.pdf",
        "PDF/X-4",
        "FOGRA39",
        expect.any(Object),
      );
    });
  });

  it("App belgeyi hazır olarak doğrulamadığında IPC veya save dialogu açmaz", async () => {
    const { getByRole } = renderReadyWizard({ isDocumentReady: false });
    const exportButton = getByRole("button", { name: "Dışa Aktar" });

    expect(exportButton.disabled).toBe(true);
    await fireEvent.click(exportButton);

    expect(save).not.toHaveBeenCalled();
    expect(exportPdfXAction).not.toHaveBeenCalled();
  });

  it("boş session kimliğini readiness true olsa bile savunmalı olarak reddeder", async () => {
    const { getByRole } = renderReadyWizard({ sessionId: "" });
    const exportButton = getByRole("button", { name: "Dışa Aktar" });

    expect(exportButton.disabled).toBe(true);
    await fireEvent.click(exportButton);

    expect(save).not.toHaveBeenCalled();
    expect(exportPdfXAction).not.toHaveBeenCalled();
  });

  it("session snapshot'ını native save dialogundan önce alır", async () => {
    const saveDialog = createDeferred();
    save.mockReturnValue(saveDialog.promise);

    const { rerender, getByRole } = renderReadyWizard({
      sessionId: "pdf-a",
      contentGeneration: 4,
    });

    await fireEvent.click(getByRole("button", { name: "Dışa Aktar" }));
    expect(save).toHaveBeenCalledOnce();

    await rerender({
      sessionId: "pdf-b",
      contentGeneration: 5,
      isDocumentReady: true,
    });
    await tick();

    saveDialog.resolve("/tmp/pdf-a-x4.pdf");

    await waitFor(() => {
      expect(exportPdfXAction).toHaveBeenCalledWith(
        "pdf-a",
        4,
        "/tmp/pdf-a-x4.pdf",
        "PDF/X-4",
        "FOGRA39",
        expect.any(Object),
      );
    });
  });

  it("close event'ini korur", async () => {
    const onClose = vi.fn();
    const { getByRole } = renderReadyWizard({ onclose: onClose });

    await fireEvent.click(getByRole("button", { name: "İptal" }));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
