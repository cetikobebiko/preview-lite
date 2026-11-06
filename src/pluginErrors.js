import { FeatureUnavailableError } from "./featureActionGuard.js";

// Every PluginErrorCode wire value from src-tauri/src/plugins/error.rs, mapped to a
// user-facing Turkish sentence. pluginErrors.test.js fails if a Rust code is missing.
export const PLUGIN_ERROR_MESSAGES = Object.freeze({
  PLUGIN_FEATURE_DISABLED:
    "Eklenti sistemi bu derlemede arka uçta kapalı. Eklentiler yalnız geliştirme derlemesinde (tauri dev) çalışır.",
  PLUGIN_MANIFEST_TOO_LARGE: "Eklenti manifesti izin verilen boyutu (64 KB) aşıyor.",
  PLUGIN_MANIFEST_INVALID: "Eklenti manifesti veya kanca yükü geçersiz.",
  PLUGIN_MANIFEST_VERSION_UNSUPPORTED: "Eklenti manifest sürümü desteklenmiyor.",
  PLUGIN_MANIFEST_UNKNOWN_FIELD: "Eklenti manifestinde tanınmayan bir alan var.",
  PLUGIN_ID_INVALID: "Eklenti kimliği geçersiz.",
  PLUGIN_VERSION_INVALID: "Eklenti sürümü geçersiz.",
  PLUGIN_PERMISSION_UNKNOWN: "Manifest tanınmayan bir izin istiyor.",
  PLUGIN_HOOK_UNKNOWN: "Bilinmeyen kanca adı.",
  PLUGIN_PATH_INVALID: "Eklenti dosyaları okunamadı veya uygulama veri dizini çözümlenemedi.",
  PLUGIN_PATH_TRAVERSAL: "Eklenti yolu izin verilen dizinin dışına çıkıyor.",
  PLUGIN_SYMLINK_REJECTED: "Eklenti paketinde sembolik bağlantı kabul edilmez.",
  PLUGIN_HARDLINK_REJECTED: "Eklenti paketinde sabit bağlantı (hardlink) kabul edilmez.",
  PLUGIN_FILE_NOT_REGULAR:
    "Eklenti paketi yalnız manifest.json ve main.rhai dosyalarını içermelidir (gizli dosyalar dahil başka dosya olamaz).",
  PLUGIN_SCRIPT_TOO_LARGE: "Eklenti betiği izin verilen boyutu (1 MB) aşıyor.",
  PLUGIN_PACKAGE_DIGEST_MISMATCH:
    "Eklenti karantinaya alındı: dosyaları onaydan sonra değişmiş. Onay geri alındı ve eklenti kapatıldı; kaldırıp yeniden kurun.",
  PLUGIN_ALREADY_INSTALLED: "Bu eklenti zaten kurulu.",
  PLUGIN_INSTALL_CONFLICT: "Bu eklentinin aynı sürümü zaten kurulu.",
  PLUGIN_NOT_INSTALLED: "Eklenti kurulu değil.",
  PLUGIN_NOT_ENABLED: "Eklenti devre dışı; çalıştırmadan önce etkinleştirin.",
  PLUGIN_PERMISSION_DENIED:
    "İzin reddedildi: eklenti onaylanmamış bir host API'ı çağırdı veya izin kümesi manifestle eşleşmiyor.",
  PLUGIN_CONSENT_REQUIRED: "Eklenti için kullanıcı onayı yok; etkinleştirilemez ve çalıştırılamaz.",
  PLUGIN_CONSENT_POLICY_MISMATCH: "Onay politikası sürümü değişti; eklentiyi yeniden onaylayın.",
  PLUGIN_CONSENT_REVOKED: "Eklenti onayı geri alınmış.",
  PLUGIN_REGISTRY_CORRUPT: "Eklenti kayıt dosyası bozuk.",
  PLUGIN_HOOK_NOT_DECLARED: "Bu kanca eklenti manifestinde bildirilmemiş; çalıştırma reddedildi.",
  PLUGIN_PAYLOAD_TOO_LARGE: "Kanca yükü izin verilen sınırı aşıyor.",
  PLUGIN_WORKER_START_FAILED: "Eklenti çalışma süreci başlatılamadı.",
  PLUGIN_WORKER_PROTOCOL_ERROR: "Eklenti çalışma süreciyle iletişim protokolü bozuldu.",
  PLUGIN_EXECUTION_TIMEOUT: "Eklenti zaman sınırını (10 sn) aştı ve durduruldu.",
  PLUGIN_EXECUTION_CANCELLED: "Eklenti yürütmesi iptal edildi.",
  PLUGIN_EXECUTION_BUSY: "Bu eklentinin süren bir yürütmesi var.",
  PLUGIN_EXECUTION_NOT_FOUND: "Yürütme bulunamadı.",
  PLUGIN_RESOURCE_LIMIT_EXCEEDED:
    "Eklenti kaynak sınırını aştı (işlem sayısı, bellek, günlük ya da bildirim sınırı) ve durduruldu.",
  PLUGIN_RESULT_TOO_LARGE: "Eklenti sonucu izin verilen boyutu aşıyor.",
  PLUGIN_RESULT_INVALID: "Eklenti betiği yalnız boş değer veya metin döndürebilir.",
  PLUGIN_WORKER_CRASHED: "Eklenti çalışma süreci beklenmedik biçimde sonlandı.",
  PLUGIN_UNINSTALL_BUSY: "Eklenti çalışırken kaldırılamaz; önce yürütmeyi iptal edin.",
  PLUGIN_SCRIPT_RUNTIME_ERROR: "Eklenti betiği çalışırken hata verdi.",
});

export const PLUGIN_BACKEND_GATE_MISMATCH_MESSAGE =
  "Eklenti sistemi arayüzde açık görünüyor ama arka uç bu derlemede kapalı (üretim derlemesi). Kurulum, etkinleştirme ve çalıştırma engellendi; kapatma ve kaldırma kullanılabilir.";

/** Reads `code` and the raw message from a BackendCommandError, a wire error or an Error. */
function codeAndDetail(error) {
  if (!error || typeof error !== "object") {
    return { code: null, detail: error == null ? "" : String(error) };
  }
  const cause = error.cause && typeof error.cause === "object" ? error.cause : null;
  const code = typeof error.code === "string"
    ? error.code
    : typeof cause?.code === "string" ? cause.code : null;
  const detail = typeof cause?.message === "string"
    ? cause.message
    : typeof error.message === "string" ? error.message : "";
  return { code, detail };
}

/**
 * Turkish, user-facing text for a plugin error. Known codes get a fixed sentence
 * plus the backend detail; unknown input falls back to its own message.
 */
export function describePluginError(error) {
  if (error instanceof FeatureUnavailableError) {
    return error.presentation?.reason || error.presentation?.warning || error.message;
  }
  const { code, detail } = codeAndDetail(error);
  const known = code ? PLUGIN_ERROR_MESSAGES[code] : undefined;
  if (!known) return detail || "Eklenti işlemi başarısız oldu.";
  return detail && detail !== known ? `${known} (${detail})` : known;
}

/** Installed-plugin state for the UI. Quarantine = consent revoked after it was given. */
export function pluginLifecycleState(plugin) {
  if (plugin?.consent_granted) return plugin.enabled ? "enabled" : "disabled";
  return plugin?.reviewed_package_digest ? "quarantined" : "consent_required";
}
