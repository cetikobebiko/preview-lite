import { callBackend } from "./client.js";

export const getActivePlugins = () => callBackend("getActivePlugins");

export const loadPlugin = (manifestPath) =>
  callBackend("loadPlugin", { manifestPath });

export const unloadPlugin = (pluginId) =>
  callBackend("unloadPlugin", { pluginId });

export const triggerPluginHook = (pluginId, hookName, payload = "") =>
  callBackend("triggerPluginHook", { pluginId, hookName, payload });

export const runScriptDirect = (scriptContent) =>
  callBackend("runScriptDirect", { scriptContent });

export const addBatchJob = (jobType, filePath, parameters = "{}") =>
  callBackend("addBatchJob", { jobType, filePath, parameters });
