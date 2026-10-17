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

export function createSignatureLossGuard({ getFormInfo, confirm }) {
  const cleared = new Set();

  return async function ensureSignatureLossAccepted(sessionId, generation, operation) {
    if (cleared.has(sessionId)) return;
    const info = await getFormInfo(sessionId, generation);
    if (info?.signature_count > 0 && !(await confirm(signatureLossMessage(operation)))) {
      throw new SignatureLossDeclinedError();
    }
    cleared.add(sessionId);
  };
}

export const ensureSignatureLossAccepted = createSignatureLossGuard({
  getFormInfo: (sessionId, generation) => sessionBackend.getSessionFormInfo(sessionId, generation),
  confirm: (message) => confirmDialog(message, { title: "İmzalı belge", kind: "warning" }),
});
