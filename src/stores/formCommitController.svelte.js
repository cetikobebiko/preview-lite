import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { fillFormFields, getFormInfo } from "../pdfState.js";

// Form taslaklarını belgeye yazan paylaşılan denetleyici (form-filling-plan.md, Faz 3).
//
// Taslak store'u başına tek denetleyici vardır; tüm sayfaların FormLayer'ları aynı kuyruğu
// kullanır. İstekler kısa bir gecikmeyle toplanır ve tüm taslaklar tek mutasyonla gider.
// Uçuşta bir commit varken gelen istek, o bitince yeni generation ile yeniden çalışır.
// Başarıda yalnız gönderildiğinden beri değişmemiş taslaklar silinir; hatada taslaklar
// korunur.

const COMMIT_DELAY_MS = 350;

const controllers = new WeakMap();

// Form yazımı artımlıdır: imzalı baytlar korunur ve imzalar kendi revizyonları için geçerli
// kalır, ama görüntüleyiciler belgeyi imzadan sonra değiştirilmiş gösterir. DocMDP P=1 ile
// sertifikalanmış belgede her değişiklik sertifikayı bozar.
export function signatureWarning(info) {
  return info?.signatures_forbid_changes
    ? "Bu belge değişikliğe kapalı olarak sertifikalanmış. Form alanlarını kaydetmek sertifikayı geçersiz kılar. Devam edilsin mi?"
    : "Bu belge dijital imza içeriyor. Değişiklikler imzalı içeriğin sonuna eklenir; imzalar korunur, ancak belge imzadan sonra değiştirilmiş olarak görünür. Devam edilsin mi?";
}

const defaultDependencies = Object.freeze({
  fill: fillFormFields,
  getFormInfo,
  confirmSignatureInvalidation: (info) =>
    confirmDialog(signatureWarning(info), { title: "İmzalı belge", kind: "warning" }),
  delay: (callback) => setTimeout(callback, COMMIT_DELAY_MS),
  cancelDelay: (handle) => clearTimeout(handle),
});

export function formCommitControllerFor(drafts, dependencies = defaultDependencies) {
  let controller = controllers.get(drafts);
  if (!controller) {
    controller = createFormCommitController(drafts, dependencies);
    controllers.set(drafts, controller);
  }
  return controller;
}

export function draftToEdit(fieldName, draft) {
  const base = {
    pageIndex: draft.target.pageIndex,
    annotIndex: draft.target.annotIndex,
    fieldName,
    kind: draft.kind,
  };
  switch (draft.kind) {
    case "text":
      return { ...base, text: draft.value };
    case "checkbox":
      return { ...base, checked: draft.checked };
    case "radio":
      return base;
    case "choice":
      return { ...base, selected: [...draft.selected] };
    default:
      throw new TypeError(`Unknown form draft kind: ${draft.kind}`);
  }
}

export function createFormCommitController(drafts, dependencies = defaultDependencies) {
  const state = $state({ status: "idle", error: "" });
  let request = null;
  let timer = null;
  let running = false;
  let rerun = false;
  // İmza denetiminden geçmiş oturum: imzası yok ya da kullanıcı uyarıyı kabul etti.
  let signaturesClearedFor = "";

  async function signaturesAllowed(sessionId, contentGeneration) {
    if (signaturesClearedFor === sessionId) return true;
    const info = await dependencies.getFormInfo(sessionId, contentGeneration);
    if (info?.signature_count && !(await dependencies.confirmSignatureInvalidation(info))) {
      return false;
    }
    signaturesClearedFor = sessionId;
    return true;
  }

  async function run() {
    if (running) {
      rerun = true;
      return;
    }
    const current = request;
    const pending = drafts.entries();
    if (!current?.sessionId || pending.length === 0 || drafts.sessionId !== current.sessionId) {
      return;
    }
    running = true;
    state.status = "saving";
    state.error = "";
    try {
      if (!(await signaturesAllowed(current.sessionId, current.contentGeneration))) {
        state.status = "error";
        state.error = "İmzalı belge kaydedilmedi; taslaklar korunuyor.";
        return;
      }
      const snapshot = await dependencies.fill(
        current.sessionId,
        current.contentGeneration,
        pending.map(([fieldName, draft]) => draftToEdit(fieldName, draft)),
      );
      for (const [fieldName, draft] of pending) {
        if (drafts.get(fieldName) === draft) drafts.discard(fieldName);
      }
      // The write moved the session to a new generation; a request queued meanwhile still
      // carries the generation its page saw before the write.
      const generation = snapshot?.contentGeneration;
      if (
        Number.isSafeInteger(generation) &&
        request?.sessionId === current.sessionId &&
        generation > request.contentGeneration
      ) {
        request = { ...request, contentGeneration: generation };
      }
      state.status = "idle";
      current.onsessionSnapshot?.(snapshot);
    } catch (error) {
      state.status = "error";
      state.error = error?.message || String(error || "Form alanları kaydedilemedi.");
      console.error("Form alanları kaydedilemedi", error);
    } finally {
      running = false;
      if (rerun) {
        rerun = false;
        void run();
      }
    }
  }

  return {
    // `contentGeneration`, isteği yapan katmanın gördüğü son generation'dır.
    request(next) {
      const sameSession = request?.sessionId === next.sessionId;
      request = {
        ...next,
        contentGeneration:
          sameSession && request.contentGeneration > next.contentGeneration
            ? request.contentGeneration
            : next.contentGeneration,
      };
      if (timer !== null) dependencies.cancelDelay(timer);
      timer = dependencies.delay(() => {
        timer = null;
        void run();
      });
    },

    get status() {
      return state.status;
    },

    get error() {
      return state.error;
    },
  };
}
