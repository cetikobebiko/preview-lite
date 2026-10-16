<script>
  import { tick } from "svelte";
  import SearchIcon from "@lucide/svelte/icons/search";
  import ChevronUpIcon from "@lucide/svelte/icons/chevron-up";
  import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
  import XIcon from "@lucide/svelte/icons/x";

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
  <div class="search-bar absolute top-3 right-4 z-40 flex items-center gap-1 rounded-lg border bg-popover p-1 pl-2 text-popover-foreground shadow-md" role="search">
    <button
      type="button"
      class="rounded-md px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground hover:bg-muted hover:text-foreground aria-pressed:bg-primary/10 aria-pressed:text-primary"
      aria-pressed={isSemanticMode}
      onclick={() => (isSemanticMode = !isSemanticMode)}
      title="Arama Modunu Değiştir"
    >
      {isSemanticMode ? 'Anlamsal' : 'Tam eşleşme'}
    </button>
    <div class="relative">
      <SearchIcon class="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
      <input
        bind:this={inputRef}
        type="text"
        aria-label="Belgede ara"
        class="h-7 w-52 rounded-md border border-input bg-background pr-2 pl-7 text-[13px] outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
        placeholder={isSemanticMode ? "Yapay zeka ile anlamsal arama..." : "Belgede ara"}
        value={query}
        oninput={handleSearchInput}
        onkeydown={handleKeyDown}
      />
    </div>

    <span class="min-w-14 px-1 text-center text-[11px] whitespace-nowrap tabular-nums text-muted-foreground" role="status">
      {#if isSearching}
        <span class="search-spinner inline-block size-3 animate-spin rounded-full border-2 border-border border-t-primary align-middle" aria-label="Aranıyor"></span>
      {:else if error}
        <span class="text-destructive">Arama hatası</span>
      {:else if query.trim() && totalMatches > 0}
        {currentMatchIndex + 1} / {totalMatches}
      {:else if query.trim()}
        Eşleşme yok
      {/if}
    </span>

    <button type="button" class="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40" onclick={handlePrev} disabled={totalMatches === 0} title="Önceki Eşleşme (Shift + Enter)" aria-label="Önceki eşleşme"><ChevronUpIcon class="size-4" /></button>
    <button type="button" class="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40" onclick={handleNext} disabled={totalMatches === 0} title="Sonraki Eşleşme (Enter)" aria-label="Sonraki eşleşme"><ChevronDownIcon class="size-4" /></button>
    <button type="button" class="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40" onclick={handleClose} title="Aramayı Kapat (Esc)" aria-label="Aramayı kapat"><XIcon class="size-4" /></button>
  </div>
{/if}
