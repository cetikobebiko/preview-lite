import { initConfig, getConfigRecentFiles } from "../../configManager.js";
import { onDragDropEvent } from "../platform/events.js";
import { fileBaseName, isOpenableFilePath } from "./openableFiles.js";
import { normalizeScale } from "./zoomLimits.js";

/**
 * Owns startup restore plus native drag-drop listener setup and teardown.
 *
 * File drops are handled only through Tauri's webview drag-drop events. With the
 * native handler enabled (Tauri's default, pinned in tauri.conf.json) the webview
 * never receives external file drags, so DOM dragover/drop listeners would see
 * nothing useful: WebKit's File has no absolute path, and a window-level dragover
 * listener would also light the file overlay during in-page HTML5 drags.
 */
export function createApplicationLifecycleController({
  viewer,
  ui,
  onOpenFile,
  onToast,
  onError,
  isDocumentOpen,
  initializeConfig = initConfig,
  getRecentFiles = getConfigRecentFiles,
  listenToDragDrop = onDragDropEvent,
}) {
  let interval = null;
  let unlistenDragDrop = null;
  let lifecycleEpoch = 0;
  let activeStart = null;
  let running = false;
  let stopped = true;

  function restoreSettings() {
    try {
      const saved = localStorage.getItem("antigravity_settings");
      if (!saved) return;
      const parsed = JSON.parse(saved);
      const savedViewer = parsed.viewer ?? parsed;
      if (savedViewer.isSmartDarkMode !== undefined) viewer.isSmartDarkMode = savedViewer.isSmartDarkMode;
      if (savedViewer.scale !== undefined) viewer.scale = normalizeScale(savedViewer.scale);
    } catch (error) { onError("Ayarlar okunamadı", error); }
  }

  function saveViewerSettings() {
    try {
      localStorage.setItem("antigravity_settings", JSON.stringify({
        viewer: {
          isSmartDarkMode: viewer.isSmartDarkMode,
          scale: viewer.scale,
        },
      }));
    } catch (error) {
      onError("Ayarlar kaydedilemedi", error);
    }
  }

  function isCurrent(epoch) {
    return !stopped && lifecycleEpoch === epoch;
  }

  function installRuntime(epoch) {
    if (!isCurrent(epoch)) return { status: "stopped" };
    restoreSettings();
    const recent = getRecentFiles()?.[0];
    if (recent) void onOpenFile(recent);
    interval = setInterval(() => { if (isDocumentOpen()) viewer.readingTimeSeconds += 1; }, 1000);
    subscribeDragDrop(epoch);
    running = true;
    return { status: "started" };
  }

  function start() {
    if (running) return Promise.resolve({ status: "started" });
    if (activeStart && !stopped) return activeStart.promise;
    stopped = false;
    const epoch = ++lifecycleEpoch;
    const pending = Promise.resolve()
      .then(() => initializeConfig())
      .then(() => installRuntime(epoch));
    const startRecord = { epoch, promise: pending };
    activeStart = startRecord;
    void pending.finally(() => {
      if (activeStart === startRecord) activeStart = null;
    }).catch(() => {});
    return pending;
  }

  function subscribeDragDrop(epoch) {
    let pending;
    try {
      pending = Promise.resolve(listenToDragDrop((event) => handleDragDrop(epoch, event?.payload)));
    } catch (error) {
      pending = Promise.reject(error);
    }
    pending.then((dispose) => {
      if (!isCurrent(epoch)) dispose();
      else unlistenDragDrop = dispose;
    }).catch((error) => {
      if (isCurrent(epoch)) onError("Dosya bırakma dinleyicisi kurulamadı", error);
    });
  }

  function handleDragDrop(epoch, payload) {
    if (!isCurrent(epoch)) return;
    switch (payload?.type) {
      case "enter":
        // In-page HTML5 drags and non-file drags arrive without openable paths.
        ui.isDraggingFileOver = droppedPaths(payload.paths).some(isOpenableFilePath);
        break;
      case "leave":
        ui.isDraggingFileOver = false;
        break;
      case "drop":
        ui.isDraggingFileOver = false;
        openDroppedPaths(droppedPaths(payload.paths));
        break;
      default:
        // "over" only carries a cursor position.
        break;
    }
  }

  function droppedPaths(paths) {
    return Array.isArray(paths) ? paths.filter((path) => typeof path === "string" && path.length > 0) : [];
  }

  function openDroppedPaths(paths) {
    if (paths.length === 0) return;
    const target = paths.find(isOpenableFilePath);
    if (!target) {
      onToast("Desteklenmeyen dosya türü. PDF veya görsel bırakın.");
      return;
    }
    void onOpenFile(target);
    const ignored = paths.length - 1;
    onToast(ignored > 0
      ? `Sürüklenen dosya açıldı: ${fileBaseName(target)}. Diğer ${ignored} dosya yok sayıldı.`
      : `Sürüklenen dosya açıldı: ${fileBaseName(target)}`);
  }

  function stop() {
    stopped = true;
    running = false;
    lifecycleEpoch += 1;
    unlistenDragDrop?.(); unlistenDragDrop = null;
    ui.isDraggingFileOver = false;
    if (interval !== null) clearInterval(interval); interval = null;
  }
  return Object.freeze({ start, stop, saveViewerSettings });
}
