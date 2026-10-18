import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import * as sessionBackend from "./backend/session.js";

// İmzalı belgede oturum mutasyonlarından önce onay ister. İki tür vardır:
//
// - Tam yeniden yazım (sayfa sıralaması): belge baştan kaydedilir; imzaların `/ByteRange`'i
//   artık imzalanan baytları göstermez ve imzalar bozulur (`ensureSignatureLossAccepted`).
// - Artımlı yazım (döndürme, silme, boş sayfa, annotation ekleme/silme): değişiklik belgenin
//   sonuna eklenir ve imzalı baytlar korunur; backend artımlı yazamazsa imzalı belgeyi baştan
//   yazmak yerine hata verir. Yine de imza panelleri belgeyi imzadan sonra değiştirilmiş
//   gösterir; kullanıcı bunu bilerek devam eder (`ensureSignedIncrementalEditAccepted`).
//
// Form yazımı da artımlıdır; kendi uyarıları formCommitController ve form tasarımındadır.
//
// Karar oturum başına ve guard başına hatırlanır: imzasız belge ya da kullanıcı bir kez kabul
// ettiyse aynı guard aynı oturumda tekrar sormaz. Artımlı bildirimi kabul etmek, tam yeniden
// yazım uyarısını atlatmaz. Reddedilirse `SignatureLossDeclinedError` atılır; çağıranlar bunu
// hata olarak göstermemelidir.

export const SIGNATURE_LOSS_DECLINED = "SIGNATURE_LOSS_DECLINED";

export class SignatureLossDeclinedError extends Error {
  constructor() {
    super("İmzaları korumak için işlem iptal edildi.");
    this.name = "SignatureLossDeclinedError";
    this.code = SIGNATURE_LOSS_DECLINED;
  }
}

export function isSignatureLossDeclined(error) {
  return error?.code === SIGNATURE_LOSS_DECLINED;
}

export function signatureLossMessage(operation) {
  return `Bu belge dijital imza içeriyor. ${operation} belgeyi yeniden yazar ve mevcut imzaları geçersiz kılar. Devam edilsin mi?`;
}

// Artımlı yazılan sayfa ve annotation mutasyonları imzalı baytları korur; değişikliğin imzanın
// izin verdiği türden olup olmadığı imzaya (DocMDP) bağlıdır, bu yüzden metin bunu iddia etmez.
export function signedIncrementalEditMessage(operation) {
  return `Bu belge dijital imza içeriyor. ${operation} belgenin sonuna eklenir ve imzalı içerik korunur; ancak imza panelleri belgeyi imzadan sonra değiştirilmiş olarak gösterir. Devam edilsin mi?`;
}

// Form alanı tasarımı artımlı yazar ve imzalı baytları korur; ama alan eklemek, silmek ya
// da değiştirmek imzanın (DocMDP) izin verdiği değişikliklerden değildir.
export function formAuthoringSignatureMessage() {
  return "Bu belge dijital imza içeriyor. Alan değişiklikleri belgenin sonuna eklenir ve imzalı içerik korunur, ancak alan eklemek, silmek veya değiştirmek imzanın izin verdiği değişikliklerden değildir; imza panelleri belgeyi değiştirilmiş gösterir. Devam edilsin mi?";
}

export function createSignatureLossGuard({ getFormInfo, confirm, message = signatureLossMessage }) {
  const cleared = new Set();

  return async function ensureSignatureLossAccepted(sessionId, generation, operation) {
    if (cleared.has(sessionId)) return;
    const info = await getFormInfo(sessionId, generation);
    if (info?.signature_count > 0 && !(await confirm(message(operation)))) {
      throw new SignatureLossDeclinedError();
    }
    cleared.add(sessionId);
  };
}

const defaultLookup = (sessionId, generation) =>
  sessionBackend.getSessionFormInfo(sessionId, generation);
const defaultConfirm = (text) => confirmDialog(text, { title: "İmzalı belge", kind: "warning" });

export const ensureSignatureLossAccepted = createSignatureLossGuard({
  getFormInfo: defaultLookup,
  confirm: defaultConfirm,
});

export const ensureSignedFormAuthoringAccepted = createSignatureLossGuard({
  getFormInfo: defaultLookup,
  confirm: defaultConfirm,
  message: formAuthoringSignatureMessage,
});

export const ensureSignedIncrementalEditAccepted = createSignatureLossGuard({
  getFormInfo: defaultLookup,
  confirm: defaultConfirm,
  message: signedIncrementalEditMessage,
});
