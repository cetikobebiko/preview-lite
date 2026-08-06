import { initConfig, getConfigRecentFiles } from "../../configManager.js";
import { listen } from "../platform/events.js";

/** Owns startup restore plus native/browser drag listener setup and teardown. */
export function createApplicationLifecycleController({
  viewer,
  ui,
  onOpenFile,
  onToast,
  onError,
  isDocumentOpen,
  initializeConfig = initConfig,
  getRecentFiles = getConfigRecentFiles,
  listenToEvent = listen,
  browserWindow = window,
}) {
  let interval = null;
  let unlistenDrop = null;
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
      if (savedViewer.scale !== undefined) viewer.scale = savedViewer.scale;
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
    void listenToEvent("tauri://drag-drop", (event) => {
      const path = event.payload?.paths?.[0];
      if (path && isCurrent(epoch)) { void onOpenFile(path); onToast("Sürüklenen dosya açıldı."); }
    }).then((dispose) => {
      if (!isCurrent(epoch)) dispose();
      else unlistenDrop = dispose;
    }).catch((error) => {
      if (isCurrent(epoch)) onError("Dosya bırakma dinleyicisi kurulamadı", error);
    });
    browserWindow.addEventListener("dragover", onDragOver);
    browserWindow.addEventListener("dragleave", onDragLeave);
    browserWindow.addEventListener("drop", onDrop);
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

  function onDragOver(event) { event.preventDefault(); ui.isDraggingFileOver = true; }
  function onDragLeave(event) { if (event.clientX <= 0 || event.clientY <= 0 || event.clientX >= browserWindow.innerWidth || event.clientY >= browserWindow.innerHeight) ui.isDraggingFileOver = false; }
  function onDrop(event) {
    event.preventDefault(); ui.isDraggingFileOver = false;
    const file = event.dataTransfer?.files?.[0];
    const path = file?.path || file?.name;
    if (path) { void onOpenFile(path); onToast(`Dosya yüklendi: ${file.name || "Yeni Belge"}`); }
  }
  function stop() {
    stopped = true;
    running = false;
    lifecycleEpoch += 1;
    unlistenDrop?.(); unlistenDrop = null;
    browserWindow.removeEventListener("dragover", onDragOver); browserWindow.removeEventListener("dragleave", onDragLeave); browserWindow.removeEventListener("drop", onDrop);
    if (interval !== null) clearInterval(interval); interval = null;
  }
  return Object.freeze({ start, stop, saveViewerSettings });
}
