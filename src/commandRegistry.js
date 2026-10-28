import { FeatureUnavailableError, guardFeatureAction } from "./featureActionGuard.js";
import { getFeature, getFeaturePresentation } from "./featureRegistry.js";

export const COMMANDS = Object.freeze([
  { id: "open-metadata", title: "Belge Meta Verilerini Düzenle (Title, Author, Keywords)", icon: "🏷️", category: "Güvenlik & Meta", featureId: null },
  { id: "open-page-labels", title: "Sayfa Etiketlerini Düzenle (i, ii, 1, 2…)", icon: "🔢", category: "Düzenleme", featureId: "page_manipulation" },
  { id: "open-outline-editor", title: "Yer İmlerini Düzenle", icon: "🔖", category: "Düzenleme", featureId: "page_manipulation" },
  { id: "open-page-marks", title: "Üstbilgi / Altbilgi, Bates, Filigran ve Arka Plan", icon: "🖋️", category: "Düzenleme", featureId: "page_manipulation" },
  { id: "export-page-image", title: "Aktif Sayfayı Yüksek Çözünürlüklü SVG / Görsel Olarak Aktar", icon: "🎨", category: "Dışa Aktar", featureId: null },
  { id: "open-bionic", title: "Biyonik Okuma Katmanı (Bionic Reading)", icon: "🧠", category: "Okuma Modları", featureId: null },
  { id: "toggle-ruler", title: "Odak Cetvelini Aç / Gizle (Line Focus)", icon: "📏", category: "Okuma Modları", featureId: null },
  { id: "open-analytics", title: "Oturum ve Okuma İstatistikleri", icon: "📊", category: "İstatistik", featureId: null },
  { id: "camera-scan", title: "iPhone Süreklilik Kamerası ile Belge Tara & Ekle", icon: "📱", category: "Donanım", featureId: "macos_continuity_camera" },
  { id: "speak-page", title: "Sayfayı Sesli Oku", icon: "🔊", category: "Ses", featureId: "text_to_speech" },
  { id: "stop-speak", title: "Sesli Okumayı Durdur", icon: "⏹️", category: "Ses", featureId: "text_to_speech" },
  { id: "share-macos", title: "macOS Paylaşım Menüsü (Share Sheet)", icon: "📤", category: "Paylaşım", featureId: "macos_share" },
  { id: "open-ai-assistant", title: "Yapay Zeka Asistanı / Belge Özetle", icon: "🤖", category: "Yapay Zeka", featureId: "local_ai" },
  { id: "run-ocr-page", title: "Taranmış Sayfaya OCR Uygula (Apple Vision)", icon: "👁️", category: "OCR", featureId: "macos_vision_ocr" },
  { id: "toggle-dark-mode", title: "Akıllı Gece Modu (Smart Dark Mode)", icon: "🌙", category: "Görünüm", featureId: "pdf_viewer" },
  { id: "toggle-book-mode", title: "Kitap Modu (Çift Sayfa)", icon: "📖", category: "Görünüm", featureId: "pdf_viewer" },
  { id: "toggle-split-view", title: "Çift Görünüm / Split View", icon: "📑", category: "Görünüm", featureId: "pdf_viewer" },
  { id: "toggle-sidebar", title: "Sol Paneli Göster / Gizle", icon: "🖼️", category: "Arayüz", featureId: "pdf_viewer" },
  { id: "open-search", title: "Belge İçi Metin Arama (Cmd+F)", icon: "🔍", category: "Gezinti", featureId: "search" },
  { id: "export-notes-md", title: "Notları Markdown (.md) Olarak Dışa Aktar", icon: "📝", category: "Not Alma", featureId: "markdown_notes_export" },
  { id: "undo-mutation", title: "Son Değişikliği Geri Al (Cmd+Z)", icon: "↩️", category: "Düzenleme", featureId: "undo_redo" },
  { id: "redo-mutation", title: "Geri Alınan Değişikliği Yinele (Shift+Cmd+Z)", icon: "↪️", category: "Düzenleme", featureId: "undo_redo" },
  { id: "split-pdf", title: "PDF Sayfalarını Böl / Seçili Sayfaları Dışa Aktar", icon: "✂️", category: "Düzenleme", featureId: "page_manipulation" },
  { id: "merge-pdf", title: "Başka Bir PDF ile Birleştir", icon: "➕", category: "Düzenleme", featureId: "page_merge_direct" },
  { id: "reset-zoom", title: "Yakınlaştırmayı Sıfırla (%100)", icon: "🎯", category: "Görünüm", featureId: "pdf_viewer" },
].map((command) => Object.freeze(command)));

export function validateCommandFeatureMappings(commands = COMMANDS) {
  for (const command of commands) {
    if (!command || typeof command.id !== "string" || !command.id) {
      throw new TypeError("Command id is required.");
    }
    if (!Object.prototype.hasOwnProperty.call(command, "featureId")) {
      throw new TypeError(`Command ${command.id} is missing featureId.`);
    }
    if (command.featureId !== null && !getFeature(command.featureId)) {
      throw new TypeError(`Command ${command.id} references unknown feature ${command.featureId}.`);
    }
  }
  return true;
}

validateCommandFeatureMappings();

export function getCommandById(commandId) {
  return COMMANDS.find((command) => command.id === commandId) || null;
}

export function getCommandPresentation(command, context = {}) {
  if (!command) {
    return Object.freeze({
      visible: false,
      runnable: false,
      status: "disabled",
      badge: "DISABLED",
      warning: "Bilinmeyen komut.",
      reason: "Bilinmeyen komut.",
    });
  }
  if (command.featureId === null) {
    return Object.freeze({
      visible: true,
      runnable: true,
      status: "stable",
      badge: "",
      warning: "",
      reason: "",
    });
  }
  return getFeaturePresentation(command.featureId, context);
}

export function filterCommands(query = "", context = {}, commands = COMMANDS) {
  const normalizedQuery = String(query).trim().toLocaleLowerCase("tr");
  return commands
    .map((command) => ({
      ...command,
      presentation: getCommandPresentation(command, context),
    }))
    .filter((command) => command.presentation.visible)
    .filter((command) => (
      !normalizedQuery ||
      command.title.toLocaleLowerCase("tr").includes(normalizedQuery) ||
      command.category.toLocaleLowerCase("tr").includes(normalizedQuery)
    ));
}

export function moveCommandSelection(currentIndex, direction, commands) {
  if (!Array.isArray(commands) || commands.length === 0) return 0;
  const delta = direction === "up" ? -1 : 1;
  return (currentIndex + delta + commands.length) % commands.length;
}

export async function selectCommand(command, context, onSelect) {
  if (!command) {
    throw new FeatureUnavailableError("unknown_command", getCommandPresentation(null, context));
  }
  if (typeof onSelect !== "function") {
    throw new TypeError("Command select callback must be a function.");
  }
  if (command.featureId === null) {
    return await onSelect(command.id);
  }
  return await guardFeatureAction(command.featureId, context, () => onSelect(command.id));
}
