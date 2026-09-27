import { callBackend } from "./client.js";

export const inspectPluginPackage = (packagePath) =>
  callBackend("inspectPluginPackage", { packagePath });

export const installPluginPackage = (packagePath, expectedDigest) =>
  callBackend("installPluginPackage", { packagePath, expectedDigest });

export const grantPluginConsent = (pluginId, expectedPackageDigest, expectedPermissions) =>
  callBackend("grantPluginConsent", { pluginId, expectedPackageDigest, expectedPermissions });

export const uninstallPlugin = (pluginId) =>
  callBackend("uninstallPlugin", { pluginId });

export const setPluginEnabled = (pluginId, enabled) =>
  callBackend("setPluginEnabled", { pluginId, enabled });

export const listInstalledPlugins = () =>
  callBackend("listInstalledPlugins");

export const startPluginExecution = (pluginId, hookName, payloadJson = "{}", executionId = "") =>
  callBackend("startPluginExecution", { pluginId, hookName, payloadJson, executionId });

export const cancelPluginExecution = (executionId) =>
  callBackend("cancelPluginExecution", { executionId });

export const getPluginExecutionStatus = (executionId) =>
  callBackend("getPluginExecutionStatus", { executionId });

export const addBatchJob = (jobType, filePath, parameters = "{}") =>
  callBackend("addBatchJob", { jobType, filePath, parameters });
