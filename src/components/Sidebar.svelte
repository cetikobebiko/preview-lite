<script>
  import { getSessionPdfOutline } from '../services/backend/session.js';
  import OutlineItem from './OutlineItem.svelte';
  import Thumbnail from './Thumbnail.svelte';

  let {
    sessionId = "",
    contentGeneration = 0,
    pageCount = 0,
    currentPageIndex = 0,
    onJump = () => {},
  } = $props();

  let activeTab = $state('thumbnails');
  let outlineNodes = $state([]);
  let loadingOutline = $state(false);
  let loadingOutlineKey = $state("");
  let loadedOutlineKey = $state("");
  let outlineRequestSequence = 0;

  $effect(() => {
    const key = `${sessionId}:${contentGeneration}`;
    if (
      !sessionId ||
      activeTab !== "outline" ||
      loadingOutlineKey === key ||
      loadedOutlineKey === key
    ) return;
    void loadOutline(sessionId, contentGeneration, key);
  });

  async function loadOutline(requestSessionId, requestGeneration, key) {
    const requestSequence = ++outlineRequestSequence;
    loadingOutline = true;
    loadingOutlineKey = key;
    try {
      const result = await getSessionPdfOutline(
        requestSessionId,
        requestGeneration,
      );
      if (
        requestSequence !== outlineRequestSequence ||
        requestSessionId !== sessionId ||
        requestGeneration !== contentGeneration
      ) return;
      outlineNodes = result;
      loadedOutlineKey = key;
    } catch (e) {
      if (requestSequence === outlineRequestSequence) {
        console.error("Outline yüklenemedi:", e);
      }
    } finally {
      if (requestSequence === outlineRequestSequence) {
        loadingOutline = false;
        loadingOutlineKey = "";
      }
    }
  }
</script>

<div class="sidebar">
  <div class="tabs">
    <button class:active={activeTab === 'thumbnails'} onclick={() => activeTab = 'thumbnails'}>
      Sayfalar
    </button>
    <button class:active={activeTab === 'outline'} onclick={() => activeTab = 'outline'}>
      İçindekiler
    </button>
  </div>

  <div class="content">
    {#if activeTab === 'outline'}
      <div class="outline-container">
        {#if loadingOutline}
          <div class="loading">Yükleniyor...</div>
        {:else if outlineNodes.length === 0}
          <div class="empty">Bu belgede içindekiler tablosu bulunamadı.</div>
        {:else}
          {#each outlineNodes as node, index (`${index}:${node.page_index}:${node.title}`)}
            <OutlineItem {node} {onJump} />
          {/each}
        {/if}
      </div>
    {:else if activeTab === 'thumbnails'}
      <div class="thumbnails-container">
        {#each Array(pageCount) as _, i (i)}
          <Thumbnail
            {sessionId}
            {contentGeneration}
            pageIndex={i}
            isActive={currentPageIndex === i}
            onSelect={() => onJump(i)}
          />
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .sidebar {
    width: 280px;
    height: 100vh;
    background: var(--bg-surface, #18181b);
    border-right: 1px solid var(--border-raw, #3f3f46);
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    overflow: hidden;
    font-family: var(--font-mono);
  }
  .tabs {
    display: flex;
    border-bottom: 1px solid var(--border-raw, #3f3f46);
    flex-shrink: 0;
    background: var(--bg-panel, #202023);
  }
  .tabs button {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-muted, #a1a1aa);
    padding: 12px;
    cursor: pointer;
    font-size: 13px;
    font-family: var(--font-mono);
    font-weight: 600;
    transition: all 0.2s ease;
  }
  .tabs button:hover {
    color: var(--text-primary, #f4f4f5);
    background: rgba(255, 255, 255, 0.05);
  }
  .tabs button.active {
    color: var(--accent-industrial, #ff5500);
    border-bottom: 2px solid var(--accent-industrial, #ff5500);
    background: var(--bg-surface, #18181b);
  }
  .content {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
  }
  .outline-container {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .thumbnails-container {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(100px, 1fr));
    gap: 16px;
    align-items: start;
  }
  .loading, .empty {
    color: var(--text-muted, #a1a1aa);
    text-align: center;
    margin-top: 24px;
    font-size: 13px;
  }
</style>
