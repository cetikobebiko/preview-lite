<script>
  export let item = {};
  export let currentPageIndex = 0;
  export let onSelectPage = () => {};

  let isOpen = true;

  function handleSelect(e) {
    e.stopPropagation();
    onSelectPage(item.page_index);
  }

  function toggleOpen(e) {
    e.stopPropagation();
    isOpen = !isOpen;
  }
</script>

<div class="outline-node">
  <div
    class="outline-item {item.page_index === currentPageIndex ? 'active' : ''}"
    style="padding-left: {4 + (item.level || 0) * 10}px;"
  >
    {#if item.children && item.children.length > 0}
      <button class="toggle-btn" on:click={toggleOpen} title={isOpen ? "Daralt" : "Genişlet"}>
        <span class="toggle-icon">{isOpen ? "-" : "+"}</span>
      </button>
    {:else}
      <span class="bullet">></span>
    {/if}

    <button class="title-btn" on:click={handleSelect} title="{item.title} (Sayfa {item.page_index + 1})">
      <span class="title-text">{item.title}</span>
      <span class="page-badge">[P.{String(item.page_index + 1).padStart(3, '0')}]</span>
    </button>
  </div>

  {#if isOpen && item.children && item.children.length > 0}
    <div class="children-container">
      {#each item.children as child}
        <svelte:self item={child} {currentPageIndex} {onSelectPage} />
      {/each}
    </div>
  {/if}
</div>

<style>
  .outline-node {
    display: flex;
    flex-direction: column;
    width: 100%;
    font-family: var(--font-mono);
  }

  .outline-item {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px;
    border-left: 2px solid transparent;
    border-radius: 0px;
    transition: none;
  }

  .outline-item:hover {
    background: var(--bg-panel);
  }

  .outline-item.active {
    border-left: 2px solid var(--accent-industrial);
    background: var(--bg-panel);
  }

  .toggle-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-mono);
    font-weight: 700;
    font-size: 11px;
  }

  .toggle-btn:hover {
    color: var(--accent-industrial);
  }

  .toggle-icon {
    font-family: var(--font-mono);
  }

  .bullet {
    color: var(--border-focus);
    font-size: 10px;
    width: 12px;
    text-align: center;
    font-family: var(--font-mono);
  }

  .title-btn {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 400;
    text-align: left;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    overflow: hidden;
    padding: 0;
  }

  .outline-item.active .title-btn {
    color: var(--accent-industrial);
    font-weight: 700;
  }

  .title-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }

  .page-badge {
    font-size: 10px;
    color: var(--text-muted);
    font-family: var(--font-mono);
  }

  .children-container {
    display: flex;
    flex-direction: column;
  }
</style>
