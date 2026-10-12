import { detectScannedPages, getAvailableOcrLanguages } from "./pdfState.js";

// Uygulama düzeyi OCR keşif cache'i.
//
// - Dil listesi: başarılı sonuç uygulama ömrü boyunca tutulur. Hata sonucu da
//   cache'lenir (fallback ["eng"]); OCR_FAILURE_RETRY_AFTER_MS dolduktan sonraki
//   ilk istekte bir kez yeniden denenir. Eşzamanlı istekler tek çağrıyı paylaşır.
// - Taranmış sayfa tespiti: (sessionId, contentGeneration) başına tek session job.
//   Hata sonucu aynı bekleme süresiyle cache'lenir; sayfa "taranmamış" sayılır
//   (eski sayfa başına davranıştaki `false` fallback'iyle aynı).
export const OCR_FAILURE_RETRY_AFTER_MS = 60_000;
export const OCR_FALLBACK_LANGUAGES = Object.freeze(["eng"]);
const MAX_CACHED_SESSIONS = 8;

let languageEntry = null;
const scannedPagesBySession = new Map();

function failureExpired(entry, now) {
  return entry.failedAt !== null && now - entry.failedAt >= OCR_FAILURE_RETRY_AFTER_MS;
}

export function getOcrLanguages() {
  const now = Date.now();
  if (languageEntry && !failureExpired(languageEntry, now)) {
    return languageEntry.promise;
  }
  const entry = { promise: null, failedAt: null };
  entry.promise = Promise.resolve()
    .then(() => getAvailableOcrLanguages())
    .then((langs) => {
      if (!Array.isArray(langs) || langs.some((lang) => typeof lang !== "string")) {
        throw new Error("OCR dil listesi beklenmeyen biçimde döndü");
      }
      return langs;
    })
    .catch((error) => {
      entry.failedAt = Date.now();
      console.error("OCR dilleri okunamadı:", error);
      return [...OCR_FALLBACK_LANGUAGES];
    });
  languageEntry = entry;
  return entry.promise;
}

function scannedPagesFor(sessionId, contentGeneration) {
  const now = Date.now();
  const cached = scannedPagesBySession.get(sessionId);
  if (
    cached &&
    cached.contentGeneration === contentGeneration &&
    !failureExpired(cached, now)
  ) {
    // LRU sırası için yeniden ekle.
    scannedPagesBySession.delete(sessionId);
    scannedPagesBySession.set(sessionId, cached);
    return cached.promise;
  }

  // Session başına yalnız son generation tutulur; eski generation sonucu atılır.
  scannedPagesBySession.delete(sessionId);
  const entry = { contentGeneration, promise: null, failedAt: null };
  entry.promise = Promise.resolve()
    .then(() => detectScannedPages(sessionId, contentGeneration))
    .then((flags) => {
      if (!Array.isArray(flags) || flags.some((flag) => typeof flag !== "boolean")) {
        throw new Error("Taranmış sayfa tespiti beklenmeyen biçimde döndü");
      }
      return flags;
    })
    .catch((error) => {
      entry.failedAt = Date.now();
      console.error("Scanned page tespiti hatası:", error);
      return null;
    });
  scannedPagesBySession.set(sessionId, entry);
  while (scannedPagesBySession.size > MAX_CACHED_SESSIONS) {
    scannedPagesBySession.delete(scannedPagesBySession.keys().next().value);
  }
  return entry.promise;
}

export async function isPageScanned(sessionId, contentGeneration, pageIndex) {
  if (!sessionId) return false;
  const flags = await scannedPagesFor(sessionId, contentGeneration);
  return flags?.[pageIndex] === true;
}

export function resetOcrDocumentStateForTests() {
  languageEntry = null;
  scannedPagesBySession.clear();
}
