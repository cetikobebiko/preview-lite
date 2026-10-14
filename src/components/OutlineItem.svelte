<script>
  import OutlineItem from "./OutlineItem.svelte";

  let { node, onJump } = $props();

  let expanded = $state(false);
</script>

<div class="outline-node">
  <div class="outline-header" onclick={() => { if(node.children.length === 0 || onJump) onJump(node.page_index); expanded = !expanded; }}>
    {#if node.children.length > 0}
      <span class="toggle" onclick={(event) => { event.stopPropagation(); expanded = !expanded; }}>
        {expanded ? '▼' : '▶'}
      </span>
    {/if}
    <span class="title">{node.title}</span>
    <span class="page-num">{node.page_index + 1}</span>
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
    padding: 6px 8px;
    cursor: pointer;
    border-radius: 4px;
    transition: background-color 0.2s;
  }
  .outline-header:hover {
    background-color: rgba(255, 255, 255, 0.1);
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
