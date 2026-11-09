import { derived } from "svelte/store";
import { installedPlugins, pluginBackendGate } from "../../pluginStore.js";

// Command palette entries for plugins: one "Eklenti: <ad>" command per enabled,
// consented plugin whose manifest declares on_custom_command. Ids carry the plugin id
// after this prefix; featureRuntime routes them to the hook dispatcher.
export const PLUGIN_COMMAND_PREFIX = "plugin-command:";

export function pluginCommandsFor(plugins, gate) {
  if (!gate?.checked || gate.enabled !== true) return [];
  return (Array.isArray(plugins) ? plugins : [])
    .filter((plugin) => (
      plugin?.enabled === true
      && plugin?.consent_granted === true
      && Array.isArray(plugin.hooks)
      && plugin.hooks.includes("on_custom_command")
    ))
    .map((plugin) => Object.freeze({
      id: `${PLUGIN_COMMAND_PREFIX}${plugin.id}`,
      title: `Eklenti: ${plugin.name || plugin.id}`,
      icon: "🧩",
      category: "Eklentiler",
      featureId: "plugin_system",
      pluginId: plugin.id,
    }));
}

export function pluginIdFromCommandId(commandId) {
  return typeof commandId === "string" && commandId.startsWith(PLUGIN_COMMAND_PREFIX)
    ? commandId.slice(PLUGIN_COMMAND_PREFIX.length) || null
    : null;
}

/** Palette commands, updated whenever the installed list or the backend gate changes. */
export const pluginPaletteCommands = derived(
  [installedPlugins, pluginBackendGate],
  ([$plugins, $gate]) => pluginCommandsFor($plugins, $gate),
);
