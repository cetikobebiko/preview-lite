<script>
  import { createEventDispatcher } from "svelte";

  export let isOpen = false;
  export let text = "";

  const dispatch = createEventDispatcher();

  function close() {
    dispatch("close");
  }

  function convertToBionic(rawText) {
    if (!rawText) return "Metin yüklenmedi...";
    
    return rawText
      .split("\n")
      .map((line) => {
        return line
          .split(" ")
          .map((word) => {
            if (word.length <= 1) return `<b>${word}</b>`;
            const mid = Math.ceil(word.length * 0.4);
            const boldPart = word.slice(0, mid);
            const restPart = word.slice(mid);
            return `<b>${boldPart}</b>${restPart}`;
          })
          .join(" ");
      })
      .join("<br/>");
  }

  $: formattedBionicText = convertToBionic(text);
</script>

{#if isOpen}
  <div class="bionic-overlay" on:click={close} role="presentation" tabindex="-1">
    <div class="bionic-dialog" on:click|stopPropagation role="presentation">
      <div class="bionic-header">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-size: 20px;">🧠</span>
          <div>
            <h3 class="bionic-title">Biyonik Okuma Katmanı (Bionic Reading)</h3>
            <p class="bionic-subtitle">İlk harfleri vurgulayarak hızlı okuma ve odaklanmayı artırır</p>
          </div>
        </div>
        <button class="close-btn" on:click={close}>✕</button>
      </div>

      <div class="bionic-body">
        <div class="bionic-text-content">
          {@html formattedBionicText}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .bionic-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--bg-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 180;
    animation: fadeIn 0.15s ease-out;
  }

  .bionic-dialog {
    width: 680px;
    height: 560px;
    max-width: 92vw;
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .bionic-header {
    padding: 14px 18px;
    border-bottom: 1px solid var(--border-raw);
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--bg-panel);
  }

  .bionic-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .bionic-subtitle {
    font-size: 11px;
    color: var(--text-muted);
  }

  .close-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    font-size: 16px;
    cursor: pointer;
  }

  .bionic-body {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    line-height: 1.8;
    font-size: 15px;
    color: var(--text-primary);
  }

  .bionic-text-content :global(b) {
    color: var(--text-primary);
    font-weight: 700;
  }
</style>
