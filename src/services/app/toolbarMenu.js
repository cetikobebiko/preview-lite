// Builds the toolbar's overflow menu. Feature gates decide visibility and
// runnability exactly as the old HUD buttons did; only the presentation moved.

const BADGE_LABELS = {
  EXPERIMENTAL: "Deneysel",
  MOCK: "Örnek",
  PLATFORM_LIMITED: "Sınırlı",
  DISABLED: "Kapalı",
};

export const badgeLabel = (badge) => (badge ? BADGE_LABELS[badge] ?? badge : "");

export function buildToolbarMenu({ isPdf, hasDocument, viewer, ui, featureUi, onFeatureAction, actions }) {
  const feature = (featureId, label, run, extra = {}, visible = featureUi(featureId).visible) => {
    if (!visible) return null;
    const state = featureUi(featureId);
    return {
      id: featureId,
      label,
      badge: badgeLabel(state.badge),
      hint: state.warning || "",
      disabled: !state.runnable,
      onSelect: () => onFeatureAction(featureId, run),
      ...extra,
    };
  };
  const item = (id, label, onSelect, extra = {}) => ({ id, label, badge: "", hint: "", disabled: false, onSelect, ...extra });
  const panel = (name) => ({ checked: Boolean(ui.panels[name]) });

  const sections = [];
  if (isPdf) {
    sections.push({
      id: "navigation",
      items: [
        item("go-back", "Geri git", actions.onGoBack, { disabled: viewer.historyStack.length === 0 }),
        item("go-forward", "İleri git", actions.onGoForward, { disabled: viewer.forwardStack.length === 0 }),
      ],
    });
  }
  if (hasDocument) {
    sections.push({
      id: "view",
      label: "Görünüm",
      items: [
        isPdf && item("single-page", "Tek sayfa", actions.onToggleSinglePage, { checked: viewer.isSinglePageMode }),
        isPdf && item("book-mode", "Kitap görünümü", actions.onToggleBookMode, { checked: viewer.isBookMode }),
        isPdf && viewer.isBookMode && item("cover-mode", "Kapak sayfası ayrı", () => (viewer.coverMode = !viewer.coverMode), { checked: viewer.coverMode }),
        item("smart-dark", "Akıllı koyu mod", actions.onToggleDarkMode, { checked: viewer.isSmartDarkMode }),
        item("reading-ruler", "Okuma cetveli", () => ui.togglePanel("readingRuler"), panel("readingRuler")),
        item("bionic", "Bionic okuma…", actions.onOpenBionic),
      ],
    });
    sections.push({
      id: "tools",
      label: "Araçlar",
      items: [
        feature("macos_vision_ocr", "Metni tanı (OCR)", actions.onRunOcr),
        viewer.isSpeaking
          ? featureUi("text_to_speech").visible
            && item("text_to_speech", "Okumayı durdur", actions.onStopSpeaking, { disabled: !featureUi("text_to_speech").runnable })
          : feature("text_to_speech", "Sesli oku", actions.onSpeak),
        feature("local_ai", "Yapay zekâ asistanı", () => ui.openModal("ai_assistant")),
      ],
    });
  }
  if (isPdf) {
    sections.push({
      id: "inspect",
      label: "İnceleme",
      items: [
        feature("basic_accessibility_audit", "Erişilebilirlik denetimi", () => ui.togglePanel("a11y"), panel("a11y")),
        // Shown when either prepress feature is visible, gated by print_resource_inspection.
        feature("print_resource_inspection", "Baskı kaynakları", () => ui.togglePanel("prepress"), panel("prepress"),
          featureUi("print_resource_inspection").visible || featureUi("rgb_derived_print_preview").visible),
        feature("sensitive_data_scan", "Hassas veri taraması", () => ui.togglePanel("sensitiveDataScan"), panel("sensitiveDataScan")),
        feature("document_compare", "Belgeyi karşılaştır…", () => ui.togglePanel("documentCompare"), panel("documentCompare")),
      ],
    });
  }
  sections.push({
    id: "document",
    label: "Belge",
    items: [
      item("open", "Aç…", actions.onSelectFile, { shortcut: "⌘O" }),
      feature("page_manipulation", "Sayfaları düzenle…", () => ui.openModal("page_organizer"), { id: "page_organizer" }),
      feature("page_manipulation", "Birleştir / Böl…", () => ui.openModal("merge_split"), { id: "merge_split" }),
    ],
  });
  sections.push({
    id: "app",
    items: [
      feature("plugin_system", "Eklentiler…", () => ui.openModal("plugin_manager")),
      feature("command_palette", "Komut paleti…", () => ui.openModal("command_palette")),
    ],
  });

  return sections
    .map((section) => ({ ...section, items: section.items.filter(Boolean) }))
    .filter((section) => section.items.length > 0);
}
