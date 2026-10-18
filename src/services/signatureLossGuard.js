import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import * as sessionBackend from "./backend/session.js";

// Tam yeniden yazım yapan oturum mutasyonlarından (döndürme, silme, sıralama, boş sayfa,
// annotation ekleme/silme) önce imzalı belgede onay ister. Bu mutasyonlar belgeyi baştan
// kaydeder; imzaların `/ByteRange`'i artık imzalanan baytları göstermez ve imzalar bozulur.
// Form yazımı artımlıdır ve imzaları korur; kendi uyarısı formCommitController'dadır.
//
// Karar oturum başına hatırlanır: imzasız belge ya da kullanıcı bir kez kabul ettiyse aynı
// oturumda tekrar sorulmaz. Reddedilirse `SignatureLossDeclinedError` atılır; çağıranlar bunu
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
