import { SvelteMap } from "svelte/reactivity";

// Form alanı taslakları (docs/engineering/form-filling-plan.md, Faz 2).
//
// Taslaklar bellektedir; `formCommitController` onları belgeye yazar (Faz 3). Anahtar tam nitelikli
// alan adıdır: aynı adlı widget'lar (ör. radio grubu, birden çok sayfadaki aynı alan)
// tek taslağı paylaşır. Oturum değişince taslaklar silinir; içerik generation'ı
// değişince ada göre taşınır. Alan adları belgeden geldiği için düz nesne yerine Map
// kullanılır ("constructor" gibi adlar prototype ile çakışmaz).
//
// Taslak değer biçimleri; `target` yazılacak widget'tır (radio için seçilen düğme):
//   { kind: "text", value: string, target }
//   { kind: "checkbox", checked: boolean, target }
//   { kind: "radio", target }
//   { kind: "choice", selected: number[], target }
// target = { pageIndex: number, annotIndex: number }

export const FORM_DRAFTS_CONTEXT = Symbol("form-drafts");

export function createFormDraftState() {
  const drafts = new SvelteMap();
  let sessionId = $state("");

  return {
    bindSession(nextSessionId) {
      if (sessionId === nextSessionId) return;
      sessionId = nextSessionId;
      drafts.clear();
    },

    has(fieldName) {
      return drafts.has(fieldName);
    },

    get(fieldName) {
      return drafts.get(fieldName);
    },

    set(fieldName, draft) {
      drafts.set(fieldName, draft);
    },

    discard(fieldName) {
      drafts.delete(fieldName);
    },

    discardAll() {
      drafts.clear();
    },

    get sessionId() {
      return sessionId;
    },

    get count() {
      return drafts.size;
    },

    // Canlı taslak nesneleri; denetleyici commit sonrası değişmemiş olanları bunlarla tanır.
    entries() {
      return Array.from(drafts.entries());
    },

    // Ayrık kopya.
    snapshot() {
      return Array.from(drafts, ([fieldName, draft]) => [fieldName, structuredClone(draft)]);
    },
  };
}
