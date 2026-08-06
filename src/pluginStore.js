import { writable } from 'svelte/store';
import { guardFeatureAction } from './featureActionGuard.js';
import { getRuntimeFeatureContext } from './featureRegistry.js';
import {
  addBatchJob as addBatchJobBackend,
  getActivePlugins,
  loadPlugin as loadPluginBackend,
  runScriptDirect as runScriptDirectBackend,
  triggerPluginHook as triggerPluginHookBackend,
  unloadPlugin as unloadPluginBackend,
} from './services/backend/plugins.js';

// Yüklü eklentilerin listesi
export const activePlugins = writable([]);

// Dinamik Slotlar (Eklentilerin UI enjeksiyonları)
// Örn: { "toolbar": [{ id: "btn1", label: "My Btn", hook: "on_my_btn" }] }
export const dynamicSlots = writable({
  toolbar: [],
  contextMenu: [],
});

export async function refreshPlugins() {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const plugins = await getActivePlugins();
      activePlugins.set(plugins);
      return plugins;
    },
  );
}

export async function loadPlugin(manifestPath) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const result = await loadPluginBackend(manifestPath);
      const plugins = await getActivePlugins();
      activePlugins.set(plugins);
      return result;
    },
  );
}

export async function unloadPlugin(pluginId) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      await unloadPluginBackend(pluginId);
      const plugins = await getActivePlugins();
      activePlugins.set(plugins);
    },
  );
}

export async function triggerPluginHook(pluginId, hookName, payload = "") {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    () => triggerPluginHookBackend(pluginId, hookName, payload),
  );
}

export async function runScriptDirect(scriptContent) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    () => runScriptDirectBackend(scriptContent),
  );
}

export async function addBatchJob(jobType, filePath, parameters = "{}") {
  return await guardFeatureAction(
    'batch_processing',
    getRuntimeFeatureContext(),
    () => addBatchJobBackend(jobType, filePath, parameters),
  );
}
