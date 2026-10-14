<script>
  import OutlineItem from "./OutlineItem.svelte";

  let { node, onJump } = $props();

  let expanded = $state(false);
</script>

<div class="outline-node">
  <div class="outline-header">
    {#if node.children.length > 0}
      <button
        type="button"
        class="toggle"
        aria-expanded={expanded}
        aria-label={expanded ? "Daralt" : "Genişlet"}
        onclick={() => { expanded = !expanded; }}
      >
        {expanded ? '▼' : '▶'}
      </button>
    {/if}
    <button type="button" class="outline-link" onclick={() => { if(node.children.length === 0 || onJump) onJump(node.page_index); expanded = !expanded; }}>
      <span class="title">{node.title}</span>
      <span class="page-num">{node.page_index + 1}</span>
    </button>
  </div>
  
  {#if expanded && node.children.length > 0}
    <div class="children">
      {#each node.children as child}
        <OutlineItem node={child} {onJump} />
      {/each}
    </div>
  {/if}
</div>

<style>
  .outline-node {
    font-size: 14px;
    color: var(--text-main, #d1d5db);
    user-select: none;
  }
  .outline-header {
    display: flex;
    align-items: center;
    border-radius: 4px;
    transition: background-color 0.2s;
  }
  .outline-header:hover {
    background-color: rgba(255, 255, 255, 0.1);
  }
  .outline-header button {
    padding: 6px 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .outline-header .outline-link {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: center;
    padding-right: 8px;
    text-align: left;
  }
  .outline-header > button:first-child {
    padding-left: 8px;
  }
  .toggle {
    display: inline-block;
    width: 16px;
    font-size: 10px;
    color: #9ca3af;
    text-align: center;
    margin-right: 4px;
  }
  .title {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .page-num {
    font-size: 12px;
    color: #9ca3af;
    margin-left: 8px;
  }
  .children {
    padding-left: 16px;
    border-left: 1px solid rgba(255,255,255,0.1);
    margin-left: 8px;
  }
</style>
