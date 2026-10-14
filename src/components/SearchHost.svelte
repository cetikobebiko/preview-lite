<script>
  import SearchBar from "./SearchBar.svelte";
  let { ui, searchState, featureUi, onSearch, onNext, onPrevious } = $props();
</script>

{#if featureUi("search").visible}
  <SearchBar isOpen={ui.panels.search} query={searchState.query} totalMatches={searchState.totalMatches} currentMatchIndex={searchState.currentMatchIndex} isSearching={searchState.isSearching} error={searchState.error} onsearch={onSearch} onnext={onNext} onprev={onPrevious} onclose={() => ui.setPanel("search", false)} />
{/if}
