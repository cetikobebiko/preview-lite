<script>
  import { BIONIC_EMPTY_MESSAGE, toBionicLines } from "../utils/bionicText.js";

  let { isOpen = false, text = "", source = null, onclose } = $props();

  const bionicLines = $derived(toBionicLines(text));
  // Bionic Reading covers a single page; say which one and how its text was obtained.
  const sourceLabel = $derived(
    source && Number.isInteger(source.pageNumber)
      ? `Yalnız sayfa ${source.pageNumber} · ${source.method === "ocr" ? "OCR ile tanınan metin" : "PDF metin katmanı"}`
      : "",
  );

  function close() {
    onclose?.();
  }
</script>

{#if isOpen}
  <div class="bionic-overlay" onclick={close} role="presentation" tabindex="-1">
    <div class="bionic-dialog" onclick={(event) => event.stopPropagation()} role="presentation">
      <div class="bionic-header">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-size: 20px;">🧠</span>
          <div>
            <h3 class="bionic-title">Biyonik Okuma Katmanı (Bionic Reading)</h3>
            <p class="bionic-subtitle">İlk harfleri vurgulayarak hızlı okuma ve odaklanmayı artırır</p>
            {#if sourceLabel}<p class="bionic-subtitle" data-testid="bionic-source">{sourceLabel}</p>{/if}
          </div>
        </div>
        <button class="close-btn" onclick={close}>✕</button>
      </div>

      <div class="bionic-body">
        <div class="bionic-text-content" data-testid="bionic-text">
          {#if bionicLines.length === 0}{BIONIC_EMPTY_MESSAGE}{:else}{#each bionicLines as words, lineIndex}{#if lineIndex > 0}<br />{/if}{#each words as word, wordIndex}{#if wordIndex > 0}{" "}{/if}<b>{word.bold}</b>{word.rest}{/each}{/each}{/if}
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

  .bionic-text-content b {
    color: var(--text-primary);
    font-weight: 700;
  }
</style>
