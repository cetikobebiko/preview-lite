<script>
  import { tick } from "svelte";

  let {
    isOpen = false,
    query = "",
    totalMatches = 0,
    currentMatchIndex = 0,
    isSearching = false,
    error = "",
    onsearch,
    onsemanticSearch,
    onnext,
    onprev,
    onclose,
  } = $props();

  let inputRef = $state(null);
  let isSemanticMode = $state(false);
  let semanticResults = $state([]);

  $effect(() => {
    if (isOpen) {
      tick().then(() => {
        if (inputRef) inputRef.focus();
      });
    }
  });

  function handleSearchInput(event) {
    const detail = {
      query: event.currentTarget.value,
      isSemanticMode,
    };
    if (typeof onsearch === "function") {
      onsearch({ detail });
    }
  }

  function handleSemanticSearch() {
    if (!query.trim()) return;
    if (typeof onsemanticSearch === "function") {
      onsemanticSearch({ detail: query });
    }
  }

  function handleNext() {
    if (typeof onnext === "function") {
      onnext();
    }
  }

  function handlePrev() {
    if (typeof onprev === "function") {
      onprev();
    }
  }

  function handleClose() {
    if (typeof onclose === "function") {
      onclose();
    }
  }

  function handleKeyDown(e) {
    if (e.key === "Enter") {
      if (isSemanticMode) {
        handleSemanticSearch();
      } else {
        if (e.shiftKey) {
          handlePrev();
        } else {
          handleNext();
        }
      }
    } else if (e.key === "Escape") {
      handleClose();
    }
  }
</script>

{#if isOpen}
  <div class="search-bar">
    <button
      class="mode-toggle {isSemanticMode ? 'semantic' : 'exact'}"
      onclick={() => (isSemanticMode = !isSemanticMode)}
      title="Arama Modunu Değiştir"
    >
      [{isSemanticMode ? 'SEMANTİK' : 'EXACT'}]
    </button>
    <div class="search-input-wrapper">
      <input
        bind:this={inputRef}
        type="text"
        placeholder={isSemanticMode ? "Yapay zeka ile anlamsal arama..." : "BELGE İÇİNDE ARA..."}
        value={query}
        oninput={handleSearchInput}
        onkeydown={handleKeyDown}
      />
    </div>

    {#if isSearching}
      <div class="search-spinner"></div>
    {:else if error}
      <span class="search-count empty">SEARCH ERROR</span>
    {:else if query.trim() && totalMatches > 0}
      <span class="search-count">MATCH: {String(currentMatchIndex + 1).padStart(2, '0')}/{String(totalMatches).padStart(2, '0')}</span>
    {:else if query.trim()}
      <span class="search-count empty">NO MATCH</span>
    {/if}

    <div class="search-actions">
      <button class="search-btn" onclick={handlePrev} disabled={totalMatches === 0} title="Önceki Eşleşme (Shift + Enter)">
        [PREV]
      </button>
      <button class="search-btn" onclick={handleNext} disabled={totalMatches === 0} title="Sonraki Eşleşme (Enter)">
        [NEXT]
      </button>
      <button class="search-btn close-btn" onclick={handleClose} title="Aramayı Kapat (Esc)">
        [ESC]
      </button>
    </div>
  </div>
{/if}

<style>
  .search-bar {
    position: absolute;
    top: 50px;
    right: 16px;
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    padding: 6px 10px;
    display: flex;
    align-items: center;
    gap: 8px;
    z-index: 80;
    font-family: var(--font-mono);
  }

  .search-input-wrapper input {
    background: var(--bg-panel);
    border: 1px solid var(--border-raw);
    outline: none;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 4px 8px;
    width: 160px;
  }

  .search-input-wrapper input:focus {
    border-color: var(--accent-industrial);
  }

  .search-count {
    font-size: 10px;
    font-weight: 700;
    color: var(--accent-industrial);
    white-space: nowrap;
    background: var(--bg-primary);
    padding: 3px 6px;
    border: 1px solid var(--border-raw);
  }

  .search-count.empty {
    color: var(--error-color);
  }

  .search-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    border-left: 1px solid var(--border-raw);
    padding-left: 6px;
  }

  .search-btn {
    background: var(--bg-panel);
    border: 1px solid var(--border-raw);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 700;
    padding: 4px 6px;
    cursor: pointer;
  }

  .search-btn:hover:not(:disabled) {
    border-color: var(--accent-industrial);
    color: var(--accent-industrial);
  }

  .search-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .search-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--border-raw);
    border-top-color: var(--accent-industrial);
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
