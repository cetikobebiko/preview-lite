import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const dialogMocks = vi.hoisted(() => ({ open: vi.fn() }));
const pdfStateMocks = vi.hoisted(() => ({ startDocumentCompare: vi.fn() }));

vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
vi.mock("../pdfState.js", () => pdfStateMocks);
vi.mock("../featureRegistry.js", async (importOriginal) => {
  const original = await importOriginal();
  return {
    ...original,
    getFeaturePresentation: (...args) => ({ ...original.getFeaturePresentation(...args), runnable: true }),
  };
});

import DocumentComparePanel from "./DocumentComparePanel.svelte";
import { createDocumentCompareState } from "../stores/documentCompareState.svelte.js";

const side = (text, pageIndex) => ({ text, textTruncated: false, pageIndex, intrinsicRotation: 0, rects: [], near: null });

describe("DocumentComparePanel", () => {
  let compareState;

  beforeEach(() => {
    vi.clearAllMocks();
    compareState = createDocumentCompareState();
  });

  it("compares the open document with the chosen file and lists the changes", async () => {
    dialogMocks.open.mockResolvedValue("/docs/v1.pdf");
    pdfStateMocks.startDocumentCompare.mockReturnValue({
      jobId: "job",
      cancel: vi.fn(),
      promise: Promise.resolve({
        currentIsOld: false,
        totalChanges: 2,
        oldWords: 10,
        newWords: 11,
        changes: [
          { kind: "replaced", old: side("brown", 0), new: side("red", 0) },
          { kind: "inserted", old: side("", 2), new: side("today", 1) },
        ],
      }),
    });
    const onNavigate = vi.fn();
    render(DocumentComparePanel, { props: { compareState, sessionId: "session-1", contentGeneration: 4, onNavigate } });

    const compare = screen.getByRole("button", { name: "Karşılaştır" });
    expect(compare.disabled).toBe(true);
    await fireEvent.click(screen.getByRole("button", { name: "PDF seç…" }));
    await waitFor(() => expect(screen.getByText("v1.pdf")).toBeTruthy());
    await fireEvent.click(screen.getByLabelText("Yeni sürüm (seçilen dosya eski)"));
    await fireEvent.click(screen.getByRole("button", { name: "Karşılaştır" }));

    expect(pdfStateMocks.startDocumentCompare).toHaveBeenCalledWith("session-1", 4, "/docs/v1.pdf", false);
    await waitFor(() => expect(screen.getByText("2 değişiklik: 1 değişti, 1 eklendi, 0 silindi.")).toBeTruthy());
    expect(screen.getByText("brown")).toBeTruthy();
    expect(screen.getByText("Sayfa 2 · seçilen dosyada 3")).toBeTruthy();

    await fireEvent.click(screen.getByText("today").closest("button"));
    expect(onNavigate).toHaveBeenCalledWith(1);
  });

  it("shows the backend's error and hides results of another document version", async () => {
    compareState.otherPath = "/docs/locked.pdf";
    pdfStateMocks.startDocumentCompare.mockReturnValue({
      jobId: "job",
      cancel: vi.fn(),
      promise: Promise.reject(Object.assign(new Error("Seçilen PDF parola korumalı"), { code: "SESSION_JOB_COMPARE_INPUT_PASSWORD_REQUIRED" })),
    });
    const { rerender } = render(DocumentComparePanel, { props: { compareState, sessionId: "session-1", contentGeneration: 1 } });
    await fireEvent.click(screen.getByRole("button", { name: "Karşılaştır" }));
    await waitFor(() => expect(screen.getByText("Seçilen PDF parola korumalı")).toBeTruthy());

    pdfStateMocks.startDocumentCompare.mockReturnValue({
      jobId: "job",
      cancel: vi.fn(),
      promise: Promise.resolve({ currentIsOld: true, totalChanges: 0, oldWords: 3, newWords: 0, changes: [] }),
    });
    await fireEvent.click(screen.getByRole("button", { name: "Karşılaştır" }));
    await waitFor(() => expect(screen.getByText("Metin farkı bulunamadı.")).toBeTruthy());
    expect(screen.getByText("Yeni sürümde metin bulunamadı; belge taranmış olabilir.")).toBeTruthy();

    await rerender({ compareState, sessionId: "session-1", contentGeneration: 2 });
    expect(screen.queryByText("Metin farkı bulunamadı.")).toBeNull();
  });
});
