<script>
  import XIcon from "@lucide/svelte/icons/x";

  // Right-docked inspector. Every open panel stays mounted so its request
  // lifecycle is unchanged; when several are open, tabs pick the visible one
  // and a newly opened panel comes to the front.
  let { panels, onclose, content } = $props();

  let activeId = $state(null);
  let knownIds = [];

  $effect.pre(() => {
    const ids = panels.map((panel) => panel.id);
    const opened = ids.filter((id) => !knownIds.includes(id));
    knownIds = ids;
    if (opened.length > 0) activeId = opened.at(-1);
    else if (!ids.includes(activeId)) activeId = ids.at(-1) ?? null;
  });
</script>

<aside class="inspector flex w-[320px] shrink-0 flex-col overflow-hidden border-l bg-sidebar text-sidebar-foreground" aria-label="Denetçi">
  {#if panels.length > 1}
    <div class="flex shrink-0 gap-0.5 overflow-x-auto border-b px-2 py-1.5" role="tablist" aria-label="Açık paneller">
      {#each panels as panel (panel.id)}
        <button
          type="button"
          role="tab"
          aria-selected={panel.id === activeId}
          class="shrink-0 rounded-md px-2 py-1 text-xs font-medium whitespace-nowrap text-muted-foreground hover:bg-muted hover:text-foreground aria-selected:bg-accent aria-selected:text-foreground"
          onclick={() => (activeId = panel.id)}
        >{panel.title}</button>
      {/each}
    </div>
  {/if}

  {#each panels as panel (panel.id)}
    <section class="flex min-h-0 flex-1 flex-col" hidden={panel.id !== activeId} aria-label={panel.title}>
      <header class="flex h-11 shrink-0 items-center gap-1 border-b pr-2 pl-4">
        <h2 class="min-w-0 flex-1 truncate text-[13px] font-semibold">{panel.title}</h2>
        {@render panel.actions?.()}
        <button
          type="button"
          class="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label="{panel.title} panelini kapat"
          onclick={() => onclose(panel.id)}
        ><XIcon class="size-4" /></button>
      </header>
      <div class="inspector-body min-h-0 flex-1 overflow-y-auto">
        {@render content(panel.id)}
      </div>
    </section>
  {/each}
</aside>
