// Görsel dışa aktarım hata kodlarını kullanıcıya gösterilecek metne çevirir.
// Backend mesajı ayrıntıyı (sayfa, piksel boyutu, önerilen en yüksek DPI)
// taşır; burada yalnız kısa bir başlık eklenir.
const EXPORT_ERROR_TITLES = Object.freeze({
  SESSION_JOB_RENDER_LIMIT_EXCEEDED: "Seçilen çözünürlük bu sayfa için çok yüksek.",
  SESSION_JOB_INVALID_EXPORT_DPI: "Geçersiz çözünürlük.",
  SESSION_JOB_INVALID_PAGE: "Seçilen sayfa belgede bulunamadı.",
  SESSION_JOB_OUTPUT_TARGET_UNSUPPORTED: "Hedef klasördeki aynı adlı öğeler dosya olmadığı için değiştirilemez.",
  SESSION_JOB_OUTPUT_ROLLBACK_FAILED: "Dışa aktarma yarıda kaldı ve tamamen geri alınamadı.",
});

const FALLBACK_MESSAGE = "Dışa aktarma başarısız oldu.";

export function exportErrorMessage(error) {
  const code = error?.code ?? error?.cause?.code;
  // BackendCommandError, worker güvenlik hatalarının ön ekini userMessage'da taşır.
  const raw = error?.userMessage ?? error?.message ?? error;
  const detail = typeof raw === "string" ? raw.trim() : "";
  const title = EXPORT_ERROR_TITLES[code];
  if (!title) return detail || FALLBACK_MESSAGE;
  return detail ? `${title} ${detail}` : title;
}
