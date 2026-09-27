import { writable } from 'svelte/store';
import { guardFeatureAction } from './featureActionGuard.js';
import { getRuntimeFeatureContext } from './featureRegistry.js';
import {
  addBatchJob as addBatchJobBackend,
  cancelPluginExecution as cancelPluginExecutionBackend,
  getPluginExecutionStatus as getPluginExecutionStatusBackend,
  grantPluginConsent as grantPluginConsentBackend,
  inspectPluginPackage as inspectPluginPackageBackend,
  installPluginPackage as installPluginPackageBackend,
  listInstalledPlugins as listInstalledPluginsBackend,
  setPluginEnabled as setPluginEnabledBackend,
  startPluginExecution as startPluginExecutionBackend,
  uninstallPlugin as uninstallPluginBackend,
} from './services/backend/plugins.js';

// Yüklü eklentilerin listesi
export const installedPlugins = writable([]);
export const activePlugins = installedPlugins; // Alias for backward compatibility

// En son incelenen aday paket (Inspect/Review)
export const inspectionCandidate = writable(null);

// Aktif yürütmelerin durum haritası: { [executionId]: PluginExecutionStatus }
export const activeExecutions = writable({});

// Dinamik Slotlar (Eklentilerin UI enjeksiyonları)
export const dynamicSlots = writable({
  toolbar: [],
  contextMenu: [],
});

export async function refreshInstalledPlugins() {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const plugins = await listInstalledPluginsBackend();
      installedPlugins.set(plugins);
      return plugins;
    },
  );
}

export async function inspectPlugin(packagePath) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const result = await inspectPluginPackageBackend(packagePath);
      const candidateWithMeta = {
        ...result,
        package_path: packagePath,
      };
      inspectionCandidate.set(candidateWithMeta);
      return candidateWithMeta;
    },
  );
}

export async function grantConsent(pluginId, expectedPackageDigest, expectedPermissions) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const result = await grantPluginConsentBackend(pluginId, expectedPackageDigest, expectedPermissions);
      await refreshInstalledPlugins();
      return result;
    },
  );
}

export async function installPlugin(packagePath, expectedDigest) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const result = await installPluginPackageBackend(packagePath, expectedDigest);
      inspectionCandidate.set(null);
      await refreshInstalledPlugins();
      return result;
    },
  );
}

export async function uninstallPlugin(pluginId) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      await uninstallPluginBackend(pluginId);
      await refreshInstalledPlugins();
    },
  );
}

export async function togglePluginEnabled(pluginId, enabled) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const result = await setPluginEnabledBackend(pluginId, enabled);
      await refreshInstalledPlugins();
      return result;
    },
  );
}

export async function startPluginHook(pluginId, hookName, payloadJson = "{}") {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const executionId = crypto.randomUUID();
      const startResult = await startPluginExecutionBackend(
        pluginId,
        hookName,
        payloadJson,
        executionId,
      );

      activeExecutions.update((prev) => ({
        ...prev,
        [executionId]: {
          execution_id: executionId,
          plugin_id: pluginId,
          hook_name: hookName,
          state: "RUNNING",
          result_json: null,
          error: null,
        },
      }));

      return startResult;
    },
  );
}

export async function cancelPlugin(executionId) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      await cancelPluginExecutionBackend(executionId);
    },
  );
}

export async function pollExecutionStatus(executionId) {
  return await guardFeatureAction(
    'plugin_system',
    getRuntimeFeatureContext(),
    async () => {
      const status = await getPluginExecutionStatusBackend(executionId);
      activeExecutions.update((prev) => ({
        ...prev,
        [executionId]: status,
      }));
      return status;
    },
  );
}

export async function addBatchJob(jobType, filePath, parameters = "{}") {
  return await guardFeatureAction(
    'batch_processing',
    getRuntimeFeatureContext(),
    () => addBatchJobBackend(jobType, filePath, parameters),
  );
}
