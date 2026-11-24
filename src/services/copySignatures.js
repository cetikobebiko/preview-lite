import * as sessionBackend from "./backend/session.js";

// Kopya üreten işlemler (güvenlik kopyası, bölme, birleştirme, toplu işlem) belgeyi baştan
// yazar ya da yalnız sayfaları yeni bir belgeye taşır; açık belgedeki dijital imzalar bu
// çıktılarda geçerli olmaz. Açık belge değişmez, bu yüzden onay sorulmaz: işlemden önce bir
// not, işlemden sonra backend'in raporuna (`signaturesInvalidated`) dayanan bir cümle gösterilir.
// Birleştirmede eklenen dosyaların imzalarını yalnız backend bilir; son söz onun raporudur.

const BEFORE = Object.freeze({
  security:
    "Bu belge dijital imza içeriyor. Kopya baştan yazılır ve imzalar kopyada geçersiz olur; açık belgedeki imzalar etkilenmez.",
  split:
    "Bu belge dijital imza içeriyor. Bölünen dosyalara yalnız sayfalar kopyalanır; imzalar bu dosyalarda geçerli olmaz. Açık belgedeki imzalar etkilenmez.",
  merge:
    "Bu belge dijital imza içeriyor. Birleştirilen dosyaya yalnız sayfalar kopyalanır; imzalar çıktıda geçerli olmaz. Açık belgedeki imzalar etkilenmez.",
});

const AFTER = Object.freeze({
  security: "Belgedeki dijital imzalar kopyada geçersizdir.",
  split: "Belgedeki dijital imzalar bölünen dosyalarda geçerli değildir.",
  merge: "Kaynak belgelerdeki dijital imzalar birleştirilen dosyada geçerli değildir.",
});

/** Pre-run note for `kind` ("security" | "split" | "merge"). */
export function copySignatureNote(kind) {
  return BEFORE[kind];
}

/** Appends the signature-loss sentence when the backend report says signatures were lost. */
export function withSignatureLoss(message, report, kind) {
  return report?.signaturesInvalidated ? `${message} ${AFTER[kind]}` : message;
}

/**
 * The open document's signature count for the pre-run note; 0 when it cannot be read
 * (the backend report still tells the user afterwards).
 */
export async function readSignatureCount(sessionId, contentGeneration, lookup = sessionBackend.getSessionFormInfo) {
  if (!sessionId) return 0;
  try {
    const info = await lookup(sessionId, contentGeneration);
    return info?.signature_count > 0 ? info.signature_count : 0;
  } catch {
    return 0;
  }
}
