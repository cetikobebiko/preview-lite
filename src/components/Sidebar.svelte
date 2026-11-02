<script>
  import { getSessionPdfOutline } from '../services/backend/session.js';
  import OutlineItem from './OutlineItem.svelte';
  import Thumbnail from './Thumbnail.svelte';

  let {
    sessionId = "",
    contentGeneration = 0,
    pageOrder = [],
    pageLabels = [],
    currentPageIndex = 0,
    onJump = () => {},
    onJumpPhysical = () => {},
    onEditOutline = null,
  } = $props();

  let activeTab = $state('thumbnails');
  const tabClass = "rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs";
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

<aside class="sidebar flex h-full w-[208px] shrink-0 flex-col overflow-hidden border-r bg-sidebar text-sidebar-foreground">
  <div class="grid shrink-0 grid-cols-2 gap-0.5 rounded-md bg-black/5 p-0.5 m-2 dark:bg-white/10" role="group" aria-label="Kenar çubuğu görünümü">
    <button type="button" class={tabClass} aria-pressed={activeTab === 'thumbnails'} onclick={() => activeTab = 'thumbnails'}>
      Sayfalar
    </button>
    <button type="button" class={tabClass} aria-pressed={activeTab === 'outline'} onclick={() => activeTab = 'outline'}>
      İçindekiler
    </button>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto px-3 pt-1 pb-4">
    {#if activeTab === 'outline'}
      <div class="flex flex-col gap-0.5">
        {#if onEditOutline}
          <button type="button" class="mb-1 self-end rounded px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-sidebar-accent hover:text-foreground" onclick={() => onEditOutline()}>Düzenle</button>
        {/if}
        {#if loadingOutline}
          <div class="mt-6 text-center text-xs text-muted-foreground">Yükleniyor…</div>
        {:else if outlineNodes.length === 0}
          <div class="mt-6 text-center text-xs text-muted-foreground">Bu belgede içindekiler tablosu bulunamadı.</div>
        {:else}
          {#each outlineNodes as node, index (`${index}:${node.page_index}:${node.title}`)}
            <OutlineItem {node} onJump={onJumpPhysical} />
          {/each}
        {/if}
      </div>
    {:else if activeTab === 'thumbnails'}
      <div class="flex flex-col items-center gap-2">
        {#each pageOrder as physicalIndex, displayIndex (physicalIndex)}
          <Thumbnail
            {sessionId}
            {contentGeneration}
            pageIndex={physicalIndex}
            pageNumber={displayIndex + 1}
            pageLabel={pageLabels[physicalIndex] ?? ""}
            isActive={currentPageIndex === displayIndex}
            onSelect={() => onJump(displayIndex)}
          />
        {/each}
      </div>
    {/if}
  </div>
</aside>
