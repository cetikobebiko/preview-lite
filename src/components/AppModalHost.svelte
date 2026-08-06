<script>
  import CommandPalette from "./CommandPalette.svelte";
  import AiSidebar from "./AiSidebar.svelte";
  import AiContextMenu from "./AiContextMenu.svelte";
  import AiSettingsModal from "./AiSettingsModal.svelte";
  import PluginManagerModal from "./PluginManagerModal.svelte";
  import DeveloperConsole from "./DeveloperConsole.svelte";
  import SignaturePanel from "./SignaturePanel.svelte";
  import SignatureWizardModal from "./SignatureWizardModal.svelte";
  import SignatureSettingsModal from "./SignatureSettingsModal.svelte";
  import SanitizationDialog from "./SanitizationDialog.svelte";

  let { ui, featureUi, sessionId, contentGeneration, displayPath, onCommandSelect, onSanitize } = $props();
</script>

{#if featureUi("signature_verification").visible}
  <SignaturePanel isOpen={ui.panels.signature} {sessionId} {contentGeneration} onclose={() => ui.setPanel("signature", false)} />
{/if}
{#if featureUi("pades_signing").visible}
  <SignatureWizardModal
    isOpen={ui.isModalOpen("signature_wizard")}
    {displayPath}
    {sessionId}
    {contentGeneration}
    on:close={() => ui.closeModal("signature_wizard")}
  />
{/if}
{#if featureUi("signature_settings").visible}
  <SignatureSettingsModal isOpen={ui.isModalOpen("signature_settings")} on:close={() => ui.closeModal("signature_settings")} />
{/if}

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
  <SanitizationDialog show={ui.isModalOpen("sanitization")} on:close={() => ui.closeModal("sanitization")} on:confirm={onSanitize} />
{/if}
{#if ui.isModalOpen("plugin_manager") && featureUi("plugin_system").visible}
  <PluginManagerModal on:close={() => ui.closeModal("plugin_manager")} />
{/if}
{#if ui.isModalOpen("developer_console") && featureUi("plugin_system").visible}
  <DeveloperConsole on:close={() => ui.closeModal("developer_console")} />
{/if}
{#if featureUi("local_ai").visible}
  <AiContextMenu />
{/if}
