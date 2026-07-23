<script>
  import ThumbnailItem from "./ThumbnailItem.svelte";
  import OutlineItem from "./OutlineItem.svelte";

  export let filePath = "";
  export let pageOrder = [];
  export let currentPageIndex = 0;
  export let outlineList = [];
  export let onSelectPage = (index) => {};
  export let onReorderPages = (newOrder) => {};
  export let onDeletePage = (index) => {};
  export let onMergePdf = () => {};
  export let onSplitPdf = () => {};

  let activeTab = "pages"; // 'pages' | 'outline'
  let draggedIndex = null;

  // Virtual List States
  let sidebarContentRef;
  let scrollTop = 0;
  let containerHeight = 800;
  const ITEM_HEIGHT = 218;

  $: totalHeight = pageOrder.length * ITEM_HEIGHT;
  $: startIndex = Math.max(0, Math.floor(scrollTop / ITEM_HEIGHT) - 3);
  $: endIndex = Math.min(pageOrder.length - 1, Math.floor((scrollTop + containerHeight) / ITEM_HEIGHT) + 3);

  $: visiblePages = pageOrder
      .map((originalPageIndex, displayIndex) => ({ originalPageIndex, displayIndex }))
      .slice(startIndex, endIndex + 1);

  let prevCurrentPageIndex = currentPageIndex;
  $: if (currentPageIndex !== prevCurrentPageIndex) {
    prevCurrentPageIndex = currentPageIndex;
    if (sidebarContentRef) {
      const topOffset = currentPageIndex * ITEM_HEIGHT;
      const bottomOffset = topOffset + ITEM_HEIGHT;
      if (topOffset < scrollTop || bottomOffset > scrollTop + containerHeight) {
        sidebarContentRef.scrollTo({
          top: Math.max(0, topOffset - (containerHeight / 2) + (ITEM_HEIGHT / 2)),
          behavior: 'smooth'
        });
      }
    }
  }

  function handleScroll(e) {
    scrollTop = e.target.scrollTop;
  }

  function handleDragStart(e, index) {
    draggedIndex = index;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", index);
  }

  function handleDragOver(e, index) {
    e.preventDefault();
  }

  function handleDrop(e, dropIndex) {
    e.preventDefault();
    if (draggedIndex !== null && draggedIndex !== dropIndex) {
      const updatedOrder = [...pageOrder];
      const [movedItem] = updatedOrder.splice(draggedIndex, 1);
      updatedOrder.splice(dropIndex, 0, movedItem);
      
      onReorderPages(updatedOrder);
      draggedIndex = null;
    }
  }
</script>

<aside class="sidebar">
  <!-- Raw Telemetry Tab Switcher Header -->
  <div class="sidebar-tabs">
    <button
      class="tab-btn {activeTab === 'pages' ? 'active' : ''}"
      on:click={() => (activeTab = "pages")}
    >
      [01. SAYFALAR ({pageOrder.length})]
    </button>
    <button
      class="tab-btn {activeTab === 'outline' ? 'active' : ''}"
      on:click={() => (activeTab = "outline")}
    >
      [02. İÇİNDEKİLER ({outlineList.length})]
    </button>
  </div>

  {#if activeTab === "pages"}
    <div class="sidebar-header">
      <div style="display: flex; gap: 4px; width: 100%;">
        <button class="sidebar-action-btn" on:click={onMergePdf} title="Başka PDF ile Birleştir">
          + BİRLEŞTİR
        </button>
        <button class="sidebar-action-btn" on:click={onSplitPdf} title="Seçili Sayfaları Yeni PDF Olarak Böl">
          / BÖL
        </button>
      </div>
    </div>
    
    <div class="sidebar-content" bind:this={sidebarContentRef} on:scroll={handleScroll} bind:clientHeight={containerHeight}>
      <div style="height: {totalHeight}px; position: relative; width: 100%;">
        {#each visiblePages as { originalPageIndex, displayIndex } (displayIndex)}
          <div style="position: absolute; top: {displayIndex * ITEM_HEIGHT}px; left: 0; width: 100%; padding-bottom: 8px;">
            <ThumbnailItem
              {filePath}
              pageIndex={originalPageIndex}
              {displayIndex}
              isActive={displayIndex === currentPageIndex}
              onSelect={onSelectPage}
              onDelete={onDeletePage}
              onDragStart={handleDragStart}
              onDragOver={handleDragOver}
              onDrop={handleDrop}
            />
          </div>
        {/each}
      </div>
    </div>
  {:else}
    <!-- Outline / Table of Contents Tree View -->
    <div class="outline-content">
      {#if outlineList.length === 0}
        <div class="outline-empty">
          <span>// YER İMİ BULUNAMADI</span>
        </div>
      {:else}
        {#each outlineList as item}
          <OutlineItem {item} {currentPageIndex} {onSelectPage} />
        {/each}
      {/if}
    </div>
  {/if}
</aside>

<style>
  .sidebar {
    width: 220px;
    height: 100%;
    background: var(--bg-surface);
    border-right: 1px solid var(--border-raw);
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    user-select: none;
    font-family: var(--font-mono);
  }

  .sidebar-tabs {
    display: flex;
    background: var(--bg-primary);
    border-bottom: 1px solid var(--border-raw);
  }

  .tab-btn {
    flex: 1;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 700;
    padding: 8px 4px;
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: center;
  }

  .tab-btn.active {
    border-bottom-color: var(--accent-industrial);
    color: var(--accent-industrial);
    background: var(--bg-surface);
  }

  .sidebar-header {
    padding: 6px 8px;
    border-bottom: 1px solid var(--border-raw);
    background: var(--bg-panel);
  }

  .sidebar-action-btn {
    flex: 1;
    background: var(--bg-surface);
    color: var(--text-primary);
    border: 1px solid var(--border-raw);
    padding: 4px 6px;
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 700;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
  }

  .sidebar-action-btn:hover {
    border-color: var(--accent-industrial);
    color: var(--accent-industrial);
  }

  .sidebar-content {
    flex: 1;
    overflow-y: auto !important;
    height: 100%;
    padding: 8px 6px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    scrollbar-width: thin;
    scrollbar-color: #333333 #141414;
  }

  .outline-content {
    flex: 1;
    overflow-y: auto !important;
    height: 100%;
    padding: 6px 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    scrollbar-width: thin;
    scrollbar-color: #333333 #141414;
  }

  /* WebKit / Tauri (macOS/Linux/Windows) Zorlaması */
  :global(.sidebar-content::-webkit-scrollbar),
  :global(.outline-content::-webkit-scrollbar) {
    width: 8px !important;
    height: 8px !important;
    display: block !important;
  }

  :global(.sidebar-content::-webkit-scrollbar-track),
  :global(.outline-content::-webkit-scrollbar-track) {
    background: #141414 !important;
    border-radius: 0px !important;
  }

  :global(.sidebar-content::-webkit-scrollbar-thumb),
  :global(.outline-content::-webkit-scrollbar-thumb) {
    background: #333333 !important;
    border-radius: 0px !important;
  }

  :global(.sidebar-content::-webkit-scrollbar-thumb:hover),
  :global(.outline-content::-webkit-scrollbar-thumb:hover) {
    background: #ff5500 !important;
  }

  :global(.sidebar-content::-webkit-scrollbar-thumb:active),
  :global(.outline-content::-webkit-scrollbar-thumb:active) {
    background: #ff7722 !important;
  }

  .outline-empty {
    padding: 40px 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 11px;
    font-family: var(--font-mono);
  }
</style>
