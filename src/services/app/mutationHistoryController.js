import { redoSessionMutation, undoSessionMutation } from "../backend/index.js";

// Backend işlem kodları (MutationRecipe::operation_name) → kullanıcıya gösterilen ad.
const OPERATION_LABELS = Object.freeze({
  rotate_page: "Sayfa döndürme",
  delete_pages: "Sayfa silme",
  reorder_pages: "Sayfa sıralaması",
  extract_pages: "Sayfa ayıklama",
  insert_blank_page: "Boş sayfa ekleme",
  add_annotation: "Açıklama ekleme",
  delete_annotation: "Açıklama silme",
  fill_form_fields: "Form doldurma",
  author_form_fields: "Form alanı düzenleme",
});

export function historyOperationLabel(operation) {
  return OPERATION_LABELS[operation] ?? "Son değişiklik";
}

const DIRECTIONS = Object.freeze({
  undo: Object.freeze({
    operation: (snapshot) => snapshot.undoOperation,
    empty: "Geri alınacak adım yok.",
    done: "Geri alındı",
    failed: "Geri alma",
  }),
  redo: Object.freeze({
    operation: (snapshot) => snapshot.redoOperation,
    empty: "Yinelenecek adım yok.",
    done: "Yinelendi",
    failed: "Yineleme",
  }),
});

/**
 * Aktif PDF oturumunda undo/redo. Geçmişin sahibi backend'dir
 * (docs/engineering/undo-redo-and-incremental-save-design.md §5); burası yalnız isteği
 * gönderir, dönen snapshot'ı mutasyonlarla aynı yoldan uygular ve kullanıcıya söyler.
 * Aynı anda tek istek çalışır; kayıt sürerken istek gönderilmez.
 */
export function createMutationHistoryController({
  documentSession,
  applyMutationSnapshot,
  onToast,
  onError,
  restore = { undo: undoSessionMutation, redo: redoSessionMutation },
}) {
  let inFlight = false;

  async function run(direction) {
    const active = documentSession.active;
    if (!active || inFlight || documentSession.isSaveInProgress(active.id)) return false;
    const copy = DIRECTIONS[direction];
    const operation = copy.operation(active);
    if (!operation) {
      onToast(copy.empty);
      return false;
    }
    inFlight = true;
    try {
      const snapshot = await restore[direction](active.id, active.contentGeneration);
      await applyMutationSnapshot(snapshot);
      onToast(`${copy.done}: ${historyOperationLabel(operation)}`);
      return true;
    } catch (error) {
      onError(`${copy.failed} başarısız: ${error?.message ?? error}`);
      return false;
    } finally {
      inFlight = false;
    }
  }

  return Object.freeze({
    undo: () => run("undo"),
    redo: () => run("redo"),
  });
}
