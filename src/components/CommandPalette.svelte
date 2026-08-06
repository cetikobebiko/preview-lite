<script>
  import { tick } from "svelte";
  import {
    filterCommands,
    moveCommandSelection,
    selectCommand,
  } from "../commandRegistry.js";
  import { FeatureUnavailableError } from "../featureActionGuard.js";
  import { getRuntimeFeatureContext } from "../featureRegistry.js";

  let {
    isOpen = $bindable(false),
    context = null,
    onselect = () => {},
    onclose = () => {},
  } = $props();

  const runtimeFeatureContext = getRuntimeFeatureContext();

  let searchQuery = $state("");
  let selectedIndex = $state(0);
  let inputRef = $state(null);
  let unavailableMessage = $state("");

  let featureContext = $derived(context || runtimeFeatureContext);
  let filteredCommands = $derived(filterCommands(searchQuery, featureContext));

  $effect(() => {
    if (isOpen) {
      searchQuery = "";
      selectedIndex = 0;
      unavailableMessage = "";
      tick().then(() => {
        if (inputRef) inputRef.focus();
      });
    }
  });

  function handleKeyDown(e) {
    if (!isOpen) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex = moveCommandSelection(selectedIndex, "down", filteredCommands);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex = moveCommandSelection(selectedIndex, "up", filteredCommands);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filteredCommands[selectedIndex]) {
        void executeCommand(filteredCommands[selectedIndex]);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  async function executeCommand(command) {
    unavailableMessage = "";
    try {
      await selectCommand(command, featureContext, (id) => {
        if (typeof onselect === "function") {
          onselect({ detail: id });
        }
      });
      close();
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        unavailableMessage = command?.presentation?.warning || error.message;
        return;
      }
      throw error;
    }
  }

  function close() {
    isOpen = false;
    if (typeof onclose === "function") {
      onclose();
    }
  }
</script>

<svelte:window onkeydown={handleKeyDown} />

{#if isOpen}
  <!-- Overlay Backdrop -->
  <div
    class="cmd-overlay"
    onclick={close}
    onkeydown={(e) => e.key === 'Escape' && close()}
    role="presentation"
    tabindex="-1"
  >
    <div class="cmd-dialog" onclick={(e) => e.stopPropagation()} role="presentation">
      <div class="cmd-header">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8"></circle>
          <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
        <input
          bind:this={inputRef}
          type="text"
          class="cmd-input"
          placeholder="Komut veya işlem arayın... (Cmd + K)"
          bind:value={searchQuery}
        />
        <span class="cmd-badge">ESC</span>
      </div>

      <div class="cmd-list">
        {#each filteredCommands as cmd, index}
          <button
            class="cmd-item {index === selectedIndex ? 'active' : ''}"
            class:unavailable={!cmd.presentation.runnable}
            onclick={() => executeCommand(cmd)}
            onmouseenter={() => (selectedIndex = index)}
            disabled={!cmd.presentation.runnable}
            aria-describedby={cmd.presentation.warning ? `command-warning-${cmd.id}` : undefined}
          >
            <span class="cmd-icon">{cmd.icon}</span>
            <div class="cmd-text">
              <span class="cmd-title">{cmd.title}</span>
              <span class="cmd-category">
                {cmd.category}
                {#if cmd.presentation.badge}
                  · {cmd.presentation.badge}
                {/if}
              </span>
              {#if cmd.presentation.warning}
                <span class="cmd-warning" id={`command-warning-${cmd.id}`}>
                  {cmd.presentation.warning}
                </span>
              {/if}
            </div>
            {#if cmd.shortcut}
              <span class="cmd-shortcut">{cmd.shortcut}</span>
            {/if}
          </button>
        {:else}
          <div class="cmd-empty">Komut bulunamadı</div>
        {/each}
      </div>

      {#if unavailableMessage}
        <div class="cmd-unavailable-message" role="status">
          {unavailableMessage}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .cmd-overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(4px);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 15vh;
    z-index: 9999;
  }

  .cmd-dialog {
    background: var(--bg-surface-elevated, #1e1e2e);
    border: 1px solid var(--border-raw, #313244);
    border-radius: 12px;
    width: 540px;
    max-width: 90vw;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.4);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .cmd-header {
    display: flex;
    align-items: center;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-raw, #313244);
    gap: 12px;
    color: var(--text-muted, #a6adc8);
  }

  .cmd-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--text-main, #cdd6f4);
    font-size: 15px;
  }

  .cmd-badge {
    font-size: 11px;
    background: var(--bg-surface, #181825);
    padding: 2px 6px;
    border-radius: 4px;
    color: var(--text-muted, #a6adc8);
    border: 1px solid var(--border-raw, #313244);
  }

  .cmd-list {
    max-height: 320px;
    overflow-y: auto;
    padding: 8px;
  }

  .cmd-item {
    width: 100%;
    display: flex;
    align-items: center;
    padding: 10px 12px;
    border-radius: 8px;
    background: transparent;
    border: none;
    color: var(--text-main, #cdd6f4);
    cursor: pointer;
    text-align: left;
    gap: 12px;
    transition: background 0.15s ease;
  }

  .cmd-item:hover,
  .cmd-item.active {
    background: var(--bg-surface-hover, #313244);
  }

  .cmd-item.unavailable {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .cmd-icon {
    font-size: 16px;
    width: 24px;
    display: flex;
    justify-content: center;
  }

  .cmd-text {
    flex: 1;
    display: flex;
    flex-direction: column;
  }

  .cmd-title {
    font-size: 13px;
    font-weight: 500;
  }

  .cmd-category {
    font-size: 11px;
    color: var(--text-muted, #a6adc8);
  }

  .cmd-warning,
  .cmd-unavailable-message {
    color: var(--accent-warning, #d97706);
    font-size: 10px;
    line-height: 1.35;
    margin-top: 4px;
  }

  .cmd-unavailable-message {
    margin: 0;
    padding: 8px 14px 12px;
    border-top: 1px solid var(--border-raw);
  }

  .cmd-empty {
    padding: 24px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
