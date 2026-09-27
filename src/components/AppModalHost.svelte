<script>
  import CommandPalette from "./CommandPalette.svelte";
  import AiSidebar from "./AiSidebar.svelte";
  import AiContextMenu from "./AiContextMenu.svelte";
  import AiSettingsModal from "./AiSettingsModal.svelte";
  import PluginManagerModal from "./PluginManagerModal.svelte";
  import SanitizationPanel from "./SanitizationPanel.svelte";

  let {
    ui, featureUi, sessionId, contentGeneration, displayPath, sanitizationState,
    sanitization, onCommandSelect,
  } = $props();
</script>


{#if featureUi("command_palette").visible}
  <CommandPalette isOpen={ui.isModalOpen("command_palette")} onselect={onCommandSelect} onclose={() => ui.closeModal("command_palette")} />
{/if}

{#if ui.isModalOpen("ai_assistant") && featureUi("local_ai").visible}
  <AiSidebar {sessionId} {contentGeneration} on:openSettings={() => ui.openModal("ai_settings")} on:close={() => ui.closeModal("ai_assistant")} />
{/if}
{#if ui.isModalOpen("ai_settings") && featureUi("local_ai").visible}
  <AiSettingsModal on:close={() => ui.closeModal("ai_settings")} />
{/if}
{#if featureUi("sanitization").visible}
  <SanitizationPanel
    show={ui.isModalOpen("sanitization")}
    state={sanitizationState}
    runnable={featureUi("sanitization").runnable}
    onInspect={sanitization.inspect}
    onPolicyChange={sanitization.updatePolicy}
    onAcknowledgementChange={sanitization.setAcknowledgement}
    onExecute={sanitization.execute}
    onCancel={sanitization.cancel}
    onReset={sanitization.reset}
    onClose={() => ui.closeModal("sanitization")}
  />
{/if}
{#if ui.isModalOpen("plugin_manager") && featureUi("plugin_system").visible}
  <PluginManagerModal on:close={() => ui.closeModal("plugin_manager")} />
{/if}
{#if featureUi("local_ai").visible}
  <AiContextMenu />
{/if}
