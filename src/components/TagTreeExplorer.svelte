<script>
  import { createEventDispatcher } from "svelte";
  import TagTreeExplorer from "./TagTreeExplorer.svelte";

  let { node, isRoot = false, level = 0 } = $props();
  const dispatch = createEventDispatcher();
  let isExpanded = $state(true);

  function selectNode() {
    dispatch("nodeSelected", node);
  }
</script>

<div class="tag-tree-node select-none" style:margin-left={isRoot ? "0" : "16px"}>
  <div class="flex items-center py-1 px-2 rounded hover:bg-white/10 transition-colors cursor-pointer border border-transparent" onclick={selectNode} role="button" tabindex="0" onkeydown={(event) => event.key === "Enter" && selectNode()}>
    {#if node.children?.length > 0}
      <button class="w-4 h-4 mr-1 text-gray-400 hover:text-white" onclick={(event) => { event.stopPropagation(); isExpanded = !isExpanded; }} aria-label={isExpanded ? "Collapse node" : "Expand node"}>
        {isExpanded ? "▼" : "▶"}
      </button>
    {:else}
      <span class="w-4 h-4 mr-1"></span>
    {/if}
    <span class="px-1.5 py-0.5 text-xs font-mono rounded bg-blue-500/20 text-blue-400 mr-2 border border-blue-500/30">
      {node.rawType ?? node.kind}
    </span>
    <span class="text-sm text-gray-300 truncate flex-1">
      {#if node.altTextPreview}<span class="text-green-400 italic text-xs mr-1">Alt: {node.altTextPreview}</span>{/if}
      {#if node.languagePreview}<span class="text-yellow-400 text-xs mr-1">[{node.languagePreview}]</span>{/if}
      <span class="text-gray-500 text-xs">#{node.id}</span>
    </span>
  </div>

  {#if isExpanded && node.children?.length > 0}
    <div class="border-l border-white/10 ml-2">
      {#each node.children as child (child.id)}
        <TagTreeExplorer node={child} level={level + 1} on:nodeSelected={(event) => dispatch("nodeSelected", event.detail)} />
      {/each}
    </div>
  {/if}
</div>

<style>
  .tag-tree-node { position: relative; }
</style>
