import { readAppConfig, writeAppConfig } from './services/backend/platform.js';

const MAX_RECENT_FILES = 20;
// Belge başına görünüm durumu (sayfa, zoom) için üst sınır; en uzun süredir güncellenmeyen düşer.
export const MAX_DOCUMENT_STATES = 200;

let configCache = {
  recentFiles: [],
  documentStates: {}
};

let isInitialized = false;
// Yazımlar sıraya girer ve her biri yazıldığı anki önbelleği taşır: daha eski bir anlık
// görüntü, sonra başlayan bir yazımın üzerine düşemez. Atomiklik Rust tarafındadır.
let saveQueue = Promise.resolve();

export async function initConfig() {
  if (isInitialized) return;
  try {
    // Rust geçerli bir JSON nesnesi ya da null döndürür; okunamayan dosya kenara alınmıştır.
    const text = await readAppConfig();
    if (text) {
      configCache = { ...configCache, ...JSON.parse(text) };
    }
    trimDocumentStates();
    isInitialized = true;
  } catch (error) {
    console.error("Config init error:", error);
  }
}

export function saveConfig() {
  saveQueue = saveQueue
    .then(() => writeAppConfig(JSON.stringify(configCache, null, 2)))
    .catch((error) => console.error("Config save error:", error));
  return saveQueue;
}

export function getConfigRecentFiles() {
  return configCache.recentFiles || [];
}

export async function addRecentFile(filePath) {
  if (!configCache.recentFiles) configCache.recentFiles = [];
  configCache.recentFiles = configCache.recentFiles.filter(f => f !== filePath);
  configCache.recentFiles.unshift(filePath);
  if (configCache.recentFiles.length > MAX_RECENT_FILES) {
    configCache.recentFiles.pop();
  }
  await saveConfig();
}

export function getDocumentState(filePath) {
  if (!configCache.documentStates) return null;
  return configCache.documentStates[filePath] || null;
}

export async function saveDocumentState(filePath, state) {
  if (!configCache.documentStates) configCache.documentStates = {};
  const previous = configCache.documentStates[filePath];
  // Silip yeniden eklemek girişi sona taşır; nesne sırası böylece güncellenme sırasıdır.
  // Yollar tamsayı biçiminde olmadığından JavaScript bu sırayı korur.
  delete configCache.documentStates[filePath];
  configCache.documentStates[filePath] = { ...previous, ...state };
  trimDocumentStates();
  await saveConfig();
}

function trimDocumentStates() {
  const states = configCache.documentStates;
  if (!states || typeof states !== "object" || Array.isArray(states)) {
    configCache.documentStates = {};
    return;
  }
  const paths = Object.keys(states);
  for (const path of paths.slice(0, Math.max(0, paths.length - MAX_DOCUMENT_STATES))) {
    delete states[path];
  }
}
