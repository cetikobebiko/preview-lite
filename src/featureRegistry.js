const ALL_DESKTOP_PLATFORMS = ["macos", "windows", "linux"];

export const FEATURE_STATUSES = Object.freeze([
  "stable",
  "experimental",
  "mock",
  "disabled",
  "platform_limited",
]);

const REQUIRED_FIELDS = Object.freeze([
  "id",
  "label",
  "status",
  "enabled",
  "warning",
  "platform",
  "documentation",
]);

const RAW_FEATURES = [
  {
    id: "command_palette",
    label: "Command Palette",
    status: "experimental",
    enabled: true,
    warning: "Hızlı komut paleti arayüzü.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#active-frontend-only-features",
  },
  {
    id: "pdf_viewer",
    label: "PDF viewer",
    status: "experimental",
    enabled: true,
    warning: "Temel görüntüleyici yolu mevcut olsa da ürün seviyesinde stable onayı verilmemiştir.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#viewer--render",
  },
  {
    id: "search",
    label: "Search",
    status: "experimental",
    enabled: true,
    warning: "Arama PDFium metin geometrisini kullanır; sonuç doğruluğu belge yapısına bağlıdır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#search--text-layer",
  },
  {
    id: "annotation",
    label: "Annotation",
    status: "experimental",
    enabled: true,
    warning: "Annotation iş akışı deneysel durumdadır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#annotation",
  },
  {
    id: "page_manipulation",
    label: "Page manipulation",
    status: "experimental",
    enabled: true,
    warning: "Desteklenen sayfa işlemleri deneysel durumdadır; iskele alt eylemler ayrıca kapatılmıştır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#page-manipulation",
  },
  {
    id: "page_merge_direct",
    label: "Direct page merge command",
    status: "disabled",
    enabled: false,
    warning: "Bu doğrudan birleştirme giriş noktasının payload sözleşmesi uyumsuzdur ve çalıştırılmaz.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#page-manipulation",
  },
  {
    id: "page_insert_blank",
    label: "Insert blank page",
    status: "disabled",
    enabled: false,
    warning: "Boş sayfa ekleme bu sürümde özellik bayrağıyla devre dışıdır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#page-manipulation",
  },
  {
    id: "ocr",
    label: "OCR",
    status: "experimental",
    enabled: true,
    warning: "OCR kullanılabilirliği kurulu motor ve dile göre değişebilir.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#ocr",
  },
  {
    id: "macos_vision_ocr",
    label: "Apple Vision OCR",
    status: "platform_limited",
    enabled: true,
    warning: "Yalnız macOS üzerinde kullanılabilir; runtime davranışı tüm sürümlerde doğrulanmamıştır.",
    platform: ["macos"],
    documentation: "docs/engineering/feature-matrix.md#ocr",
  },
  {
    id: "local_ai",
    label: "Yapay Zeka (AI)",
    status: "experimental",
    enabled: true,
    warning: "Belge içeriği yapılandırılan AI endpoint’ine gönderilebilir. Yalnızca loopback endpoint kullanıldığında trafik cihaz içinde kalır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#ai--rag",
  },
  {
    id: "sensitive_data_scan",
    label: "Hassas veri tespiti",
    status: "experimental",
    enabled: true,
    warning: "Eşleşmeler otomatik tahmindir; hassas verilerin tamamını bulduğu garanti edilmez.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#redaction",
  },
  {
    id: "visual_redaction_mask",
    label: "Visual redaction mask",
    status: "mock",
    enabled: false,
    warning: "Yalnız görsel maske taslağıdır; alttaki PDF içeriğini güvenli biçimde kaldırmaz ve gerçek işlem yapılmaz.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#redaction",
  },
  {
    id: "redaction",
    label: "Secure redaction",
    status: "disabled",
    enabled: false,
    warning: "Secure redaction pipeline ve kullanıcı workflow'u tamamlanmıştır; release platform certification tamamlanana kadar özellik kapalıdır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#redaction",
  },
  {
    id: "sanitization",
    label: "Sanitization",
    status: "disabled",
    enabled: false,
    warning: "Sanitization inspection, verified execution ve frontend review workflow'u uygulanmıştır; 8C-7 platform/security aktivasyon kararı tamamlanana kadar özellik kapalıdır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#sanitization",
  },
  {
    id: "pades_signing",
    label: "PAdES signing",
    status: "disabled",
    enabled: false,
    warning: "Kriptografik PDF imzalama henüz uygulanmamıştır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#signature--pades",
  },
  {
    id: "signature_verification",
    label: "Signature verification",
    status: "disabled",
    enabled: false,
    warning: "PDF imzaları kriptografik olarak doğrulanmamaktadır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#signature--pades",
  },
  {
    id: "ltv",
    label: "LTV",
    status: "disabled",
    enabled: false,
    warning: "DSS, OCSP/CRL ve archive timestamp işlemleri uygulanmamıştır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#signature--pades",
  },
  {
    id: "signature_settings",
    label: "Signature settings / TSA / trust store",
    status: "disabled",
    enabled: false,
    warning: "TSA ve güven deposu yapılandırması henüz uygulanmamıştır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#signature--pades",
  },
  {
    id: "basic_accessibility_audit",
    label: "Basic accessibility audit",
    status: "experimental",
    enabled: true,
    warning: "Uygulanan makine kontrolleri sınırlıdır; sonuç PDF/UA uygunluğu veya erişilebilirlik garantisi değildir.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#accessibility--pdf-ua",
  },
  {
    id: "pdf_ua_machine_validation",
    label: "PDF/UA machine validation",
    status: "disabled",
    enabled: false,
    warning: "Tam profil doğrulaması ve harici validator entegrasyonu uygulanmamıştır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/pdf-ua-validation-strategy.md",
  },
  {
    id: "pdf_auto_tag",
    label: "PDF Auto-Tag",
    status: "disabled",
    enabled: false,
    warning: "Otomatik etiketleme eylemi uygulanmamıştır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#accessibility--pdf-ua",
  },
  {
    id: "reading_order_editing",
    label: "Reading-order editing",
    status: "disabled",
    enabled: false,
    warning: "Okuma sırası düzenlemesi uygulanmamıştır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#accessibility--pdf-ua",
  },
  {
    id: "pdf_x_export",
    label: "PDF/X export",
    status: "disabled",
    enabled: false,
    warning: "Gerçek PDF/X conversion ve validation uygulanmamıştır; production export yüzeyi yoktur.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#prepress--pdf-x",
  },
  {
    id: "print_resource_inspection",
    label: "Print resource inspection",
    status: "experimental",
    enabled: true,
    warning: "Yalnız listelenen PDF kaynaklarını ve dictionary yapılarını inceler. Kullanım, baskı sonucu veya PDF/X uygunluğu garanti edilmez.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#prepress--pdf-x",
  },
  {
    id: "rgb_derived_print_preview",
    label: "RGB-derived print preview",
    status: "experimental",
    enabled: true,
    warning: "Ekran RGB render'ından matematiksel olarak türetilir. ICC tabanlı separasyon, gerçek mürekkep ölçümü veya RIP çıktısı değildir.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#prepress--pdf-x",
  },
  {
    id: "plugin_system",
    label: "Plugin system",
    status: "experimental",
    enabled: false,
    warning: "Eklenti manifest izinleri tam olarak uygulanmamıştır; eklenti yükleme ve script çalıştırma kapalıdır.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#plugins",
  },
  {
    id: "batch_processing",
    label: "Batch processing",
    status: "disabled",
    enabled: false,
    warning: "Toplu iş işleme bu sürümde devre dışıdır; backend FEATURE_UNAVAILABLE döndürür.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#batch-processing",
  },
  {
    id: "form_editing",
    label: "Form editing",
    status: "experimental",
    enabled: true,
    warning: "Form alanları okunur ve taslak olarak doldurulabilir; değişiklikler henüz belgeye kaydedilmez.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/form-filling-plan.md",
  },
  {
    id: "searchable_pdf_export",
    label: "Searchable PDF export",
    status: "disabled",
    enabled: false,
    warning: "Aranabilir PDF dışa aktarma backend tarafından desteklenmemektedir.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#ocr",
  },
  {
    id: "macos_continuity_camera",
    label: "macOS Continuity Camera",
    status: "disabled",
    enabled: false,
    warning: "macOS Continuity Camera eylemi geçici olarak devre dışıdır.",
    platform: ["macos"],
    documentation: "docs/engineering/feature-matrix.md#platform-specific-features",
  },
  {
    id: "macos_share",
    label: "macOS Share",
    status: "platform_limited",
    enabled: true,
    warning: "Yalnız macOS üzerinde kullanılabilir; runtime davranışı tüm sürümlerde doğrulanmamıştır.",
    platform: ["macos"],
    documentation: "docs/engineering/feature-matrix.md#platform-specific-features",
  },
  {
    id: "text_to_speech",
    label: "Text to speech",
    status: "platform_limited",
    enabled: true,
    warning: "Yalnız desteklenen masaüstü platformlarında kullanılabilir; ses ve dil desteği sisteme bağlıdır.",
    platform: ["macos", "windows"],
    documentation: "docs/engineering/feature-matrix.md#platform-specific-features",
  },
  {
    id: "markdown_notes_export",
    label: "Markdown notes export",
    status: "mock",
    enabled: false,
    warning: "Markdown dosyası yazılmamaktadır; geliştirici görünümünde de gerçek işlem yapılmaz.",
    platform: ALL_DESKTOP_PLATFORMS,
    documentation: "docs/engineering/feature-matrix.md#active-frontend-only-features",
  },
];

function isNonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0;
}

export function validateFeatureRegistryRecords(records) {
  if (!Array.isArray(records)) {
    throw new TypeError("Feature registry records must be an array.");
  }

  const ids = new Set();
  for (const record of records) {
    if (!record || typeof record !== "object" || Array.isArray(record)) {
      throw new TypeError("Each feature registry record must be an object.");
    }

    for (const field of REQUIRED_FIELDS) {
      if (!Object.prototype.hasOwnProperty.call(record, field)) {
        throw new TypeError(`Feature record is missing required field: ${field}`);
      }
    }

    if (!isNonEmptyString(record.id) || !/^[a-z0-9_]+$/.test(record.id)) {
      throw new TypeError("Feature id must be a non-empty snake_case string.");
    }
    if (ids.has(record.id)) {
      throw new TypeError(`Duplicate feature id: ${record.id}`);
    }
    ids.add(record.id);

    if (!isNonEmptyString(record.label)) {
      throw new TypeError(`Feature ${record.id} has an invalid label.`);
    }
    if (!FEATURE_STATUSES.includes(record.status)) {
      throw new TypeError(`Feature ${record.id} has an invalid status.`);
    }
    if (typeof record.enabled !== "boolean") {
      throw new TypeError(`Feature ${record.id} has an invalid enabled value.`);
    }
    if (typeof record.warning !== "string") {
      throw new TypeError(`Feature ${record.id} has an invalid warning.`);
    }
    if (
      !Array.isArray(record.platform) ||
      record.platform.length === 0 ||
      record.platform.some((platform) => !ALL_DESKTOP_PLATFORMS.includes(platform)) ||
      new Set(record.platform).size !== record.platform.length
    ) {
      throw new TypeError(`Feature ${record.id} has an invalid platform list.`);
    }
    if (!isNonEmptyString(record.documentation)) {
      throw new TypeError(`Feature ${record.id} has invalid documentation metadata.`);
    }
  }

  return true;
}

function freezeRecord(record) {
  return Object.freeze({
    ...record,
    platform: Object.freeze([...record.platform]),
  });
}

validateFeatureRegistryRecords(RAW_FEATURES);

export const featureRegistry = Object.freeze(
  Object.fromEntries(RAW_FEATURES.map((record) => [record.id, freezeRecord(record)])),
);

function normalizePlatform(platform) {
  const normalized = typeof platform === "string" ? platform.toLowerCase() : "";
  if (
    normalized === "macos" ||
    normalized === "mac" ||
    normalized === "darwin" ||
    normalized.includes("mac")
  ) return "macos";
  if (
    normalized === "windows" ||
    normalized === "win32" ||
    normalized === "win" ||
    normalized.includes("windows")
  ) return "windows";
  if (normalized === "linux" || normalized.includes("linux")) return "linux";
  return "unknown";
}

function detectRuntimePlatform() {
  if (typeof navigator !== "undefined") {
    const navPlatform = normalizePlatform(navigator.userAgentData?.platform || navigator.platform || "");
    if (navPlatform !== "unknown") return navPlatform;
  }
  if (typeof process !== "undefined" && process.platform) {
    return normalizePlatform(process.platform);
  }
  return "unknown";
}

export function getRuntimeFeatureContext(overrides = {}) {
  const explicitBuild = overrides.build;
  const build = explicitBuild === "development" || explicitBuild === "production"
    ? explicitBuild
    : import.meta.env?.DEV
      ? "development"
      : "production";

  return Object.freeze({
    build,
    platform: normalizePlatform(overrides.platform ?? detectRuntimePlatform()),
  });
}

export function getFeature(featureId) {
  return typeof featureId === "string" ? featureRegistry[featureId] || null : null;
}

function unavailableReason(feature, context) {
  if (!feature) return "Bilinmeyen özellik kimliği.";
  if (feature.status === "mock") return feature.warning || "Bu özellik yalnız bir mock taslaktır.";
  if (feature.status === "disabled") return feature.warning || "Bu özellik devre dışıdır.";
  if (!feature.enabled) return feature.warning || "Bu özellik ürün seviyesinde kapalıdır.";
  if (context.platform === "unknown") return "Çalışma platformu güvenli biçimde belirlenemedi.";
  if (!feature.platform.includes(context.platform)) {
    return `${feature.label} bu platformda kullanılamaz.`;
  }
  return "";
}

export function isFeatureRunnable(featureId, context = {}) {
  const feature = getFeature(featureId);
  return isFeatureRecordRunnable(feature, context);
}

export function isFeatureRecordRunnable(feature, context = {}) {
  const runtimeContext = getRuntimeFeatureContext(context);
  return Boolean(
    feature &&
    typeof feature.enabled === "boolean" &&
    FEATURE_STATUSES.includes(feature.status) &&
    Array.isArray(feature.platform) &&
    feature.enabled &&
    feature.status !== "mock" &&
    feature.status !== "disabled" &&
    runtimeContext.platform !== "unknown" &&
    feature.platform.includes(runtimeContext.platform),
  );
}

export function isFeatureVisible(featureId, context = {}) {
  const feature = getFeature(featureId);
  if (!feature) return false;

  const runtimeContext = getRuntimeFeatureContext(context);
  if (
    runtimeContext.platform === "unknown" ||
    !feature.platform.includes(runtimeContext.platform)
  ) {
    return false;
  }
  if (isFeatureRunnable(featureId, runtimeContext)) return true;

  return runtimeContext.build === "development";
}

export function getFeaturePresentation(featureId, context = {}) {
  const feature = getFeature(featureId);
  const runtimeContext = getRuntimeFeatureContext(context);
  const runnable = isFeatureRunnable(featureId, runtimeContext);
  const visible = isFeatureVisible(featureId, runtimeContext);
  const reason = runnable ? "" : unavailableReason(feature, runtimeContext);

  return Object.freeze({
    feature,
    featureId: typeof featureId === "string" ? featureId : "",
    status: feature?.status || "disabled",
    badge: feature?.status === "stable" ? "" : (feature?.status || "disabled").toUpperCase(),
    warning: feature?.warning || reason,
    reason,
    visible,
    runnable,
    context: runtimeContext,
  });
}

// This registry is a frontend presentation and defense-in-depth gate. It is not a
// backend authorization boundary; native commands must enforce their own security.
