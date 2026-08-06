import { BaseDirectory, readTextFile, writeTextFile, exists, mkdir } from '@tauri-apps/plugin-fs';

const CONFIG_FILE = 'app_config.json';

let configCache = {
  recentFiles: [],
  documentStates: {}
};

let isInitialized = false;

export async function initConfig() {
  if (isInitialized) return;
  try {
    const dirExists = await exists('', { baseDir: BaseDirectory.AppData });
    if (!dirExists) {
      await mkdir('', { baseDir: BaseDirectory.AppData, recursive: true });
    }
    
    const hasConfig = await exists(CONFIG_FILE, { baseDir: BaseDirectory.AppData });
    if (hasConfig) {
      const text = await readTextFile(CONFIG_FILE, { baseDir: BaseDirectory.AppData });
      try {
        configCache = { ...configCache, ...JSON.parse(text) };
      } catch (e) {
        console.error("Config parse error, using defaults");
      }
    } else {
      await saveConfig();
    }
    isInitialized = true;
  } catch (error) {
    console.error("Config init error:", error);
  }
}

export async function saveConfig() {
  try {
    await writeTextFile(CONFIG_FILE, JSON.stringify(configCache, null, 2), { baseDir: BaseDirectory.AppData });
  } catch (error) {
    console.error("Config save error:", error);
  }
}

export function getConfigRecentFiles() {
  return configCache.recentFiles || [];
}

export async function addRecentFile(filePath) {
  if (!configCache.recentFiles) configCache.recentFiles = [];
  configCache.recentFiles = configCache.recentFiles.filter(f => f !== filePath);
  configCache.recentFiles.unshift(filePath);
  if (configCache.recentFiles.length > 20) {
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
  configCache.documentStates[filePath] = {
    ...configCache.documentStates[filePath],
    ...state
  };
  await saveConfig();
}
