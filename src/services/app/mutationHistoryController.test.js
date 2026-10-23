import { describe, expect, it, vi } from "vitest";
import {
  createMutationHistoryController,
  historyOperationLabel,
} from "./mutationHistoryController.js";

vi.mock("../backend/index.js", () => ({
  undoSessionMutation: vi.fn(),
  redoSessionMutation: vi.fn(),
}));

function setup(active, { saving = false } = {}) {
  const documentSession = {
    active,
    isSaveInProgress: vi.fn(() => saving),
  };
  const restore = { undo: vi.fn(), redo: vi.fn() };
  const applyMutationSnapshot = vi.fn(async () => {});
  const onToast = vi.fn();
  const onError = vi.fn();
  const controller = createMutationHistoryController({
    documentSession,
    applyMutationSnapshot,
    onToast,
    onError,
    restore,
  });
  return { controller, restore, applyMutationSnapshot, onToast, onError };
}

const session = (overrides = {}) => ({
  id: "s1",
  contentGeneration: 4,
  undoOperation: "rotate_page",
  redoOperation: null,
  ...overrides,
});

describe("mutation history controller", () => {
  it("undoes through the backend, applies the snapshot like a mutation and names the step", async () => {
    const { controller, restore, applyMutationSnapshot, onToast } = setup(session());
    const snapshot = { id: "s1", contentGeneration: 5 };
    restore.undo.mockResolvedValueOnce(snapshot);

    await expect(controller.undo()).resolves.toBe(true);

    expect(restore.undo).toHaveBeenCalledWith("s1", 4);
    expect(applyMutationSnapshot).toHaveBeenCalledWith(snapshot);
    expect(onToast).toHaveBeenCalledWith("Geri alındı: Sayfa döndürme");
  });

  it("redoes with the redo operation's name", async () => {
    const { controller, restore, onToast } = setup(session({ undoOperation: null, redoOperation: "add_annotation" }));
    restore.redo.mockResolvedValueOnce({ id: "s1", contentGeneration: 5 });
    await expect(controller.redo()).resolves.toBe(true);
    expect(restore.redo).toHaveBeenCalledWith("s1", 4);
    expect(onToast).toHaveBeenCalledWith("Yinelendi: Açıklama ekleme");
  });

  it("does not call the backend when there is nothing to restore", async () => {
    const { controller, restore, onToast } = setup(session({ undoOperation: null }));
    await expect(controller.undo()).resolves.toBe(false);
    await expect(controller.redo()).resolves.toBe(false);
    expect(restore.undo).not.toHaveBeenCalled();
    expect(restore.redo).not.toHaveBeenCalled();
    expect(onToast).toHaveBeenCalledWith("Geri alınacak adım yok.");
    expect(onToast).toHaveBeenCalledWith("Yinelenecek adım yok.");
  });

  it("waits for a running save and for its own previous request", async () => {
    const saving = setup(session(), { saving: true });
    await expect(saving.controller.undo()).resolves.toBe(false);
    expect(saving.restore.undo).not.toHaveBeenCalled();

    const { controller, restore } = setup(session());
    let release;
    restore.undo.mockReturnValueOnce(new Promise((resolve) => { release = resolve; }));
    const first = controller.undo();
    await expect(controller.undo()).resolves.toBe(false);
    release({ id: "s1", contentGeneration: 5 });
    await expect(first).resolves.toBe(true);
    expect(restore.undo).toHaveBeenCalledTimes(1);
  });

  it("reports a backend failure without applying anything", async () => {
    const { controller, restore, applyMutationSnapshot, onError } = setup(session());
    restore.undo.mockRejectedValueOnce(new Error("Geçmiş dosyası kayıtlı içerikle eşleşmiyor"));
    await expect(controller.undo()).resolves.toBe(false);
    expect(applyMutationSnapshot).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith("Geri alma başarısız: Geçmiş dosyası kayıtlı içerikle eşleşmiyor");
  });

  it("ignores requests without an active PDF session", async () => {
    const { controller, restore } = setup(null);
    await expect(controller.undo()).resolves.toBe(false);
    expect(restore.undo).not.toHaveBeenCalled();
  });

  it("names every backend operation and falls back for unknown ones", () => {
    for (const operation of [
      "rotate_page", "delete_pages", "reorder_pages", "extract_pages", "insert_blank_page",
      "add_annotation", "delete_annotation", "fill_form_fields", "author_form_fields",
    ]) {
      expect(historyOperationLabel(operation)).not.toBe("Son değişiklik");
    }
    expect(historyOperationLabel("something_new")).toBe("Son değişiklik");
  });
});
