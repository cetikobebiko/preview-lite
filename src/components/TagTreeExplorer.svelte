<script>
    import { createEventDispatcher } from 'svelte';
    const dispatch = createEventDispatcher();
    
    export let node;
    export let isRoot = false;
    export let level = 0;
    export let canReorder = false;
    
    let isExpanded = true;
    
    function toggleExpand() {
        isExpanded = !isExpanded;
    }
    
    function handleDragStart(event) {
        if (isRoot || !canReorder) return;
        event.dataTransfer.setData('text/plain', node.id);
        event.dataTransfer.effectAllowed = 'move';
        event.target.classList.add('opacity-50');
    }
    
    function handleDragEnd(event) {
        event.target.classList.remove('opacity-50');
    }
    
    function handleDragOver(event) {
        if (isRoot || !canReorder) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'move';
        event.currentTarget.classList.add('bg-blue-500/20');
    }
    
    function handleDragLeave(event) {
        if (isRoot || !canReorder) return;
        event.currentTarget.classList.remove('bg-blue-500/20');
    }
    
    function handleDrop(event) {
        if (isRoot || !canReorder) return;
        event.preventDefault();
        event.currentTarget.classList.remove('bg-blue-500/20');
        const draggedId = event.dataTransfer.getData('text/plain');
        if (draggedId && draggedId !== node.id) {
            dispatch('nodeReorder', { draggedId, targetId: node.id });
        }
    }
    
    function selectNode() {
        dispatch('nodeSelected', node);
    }
    
    function forwardNodeSelected(event) {
        dispatch('nodeSelected', event.detail);
    }
    
    function forwardNodeReorder(event) {
        dispatch('nodeReorder', event.detail);
    }
</script>

<div class="tag-tree-node select-none" style="margin-left: {isRoot ? 0 : 16}px;">
    <div 
        class="flex items-center py-1 px-2 rounded hover:bg-white/10 transition-colors cursor-pointer border border-transparent"
        draggable={!isRoot && canReorder}
        on:dragstart={handleDragStart}
        on:dragend={handleDragEnd}
        on:dragover={handleDragOver}
        on:dragleave={handleDragLeave}
        on:drop={handleDrop}
        on:click|stopPropagation={selectNode}
    >
        {#if node.children && node.children.length > 0}
            <button class="w-4 h-4 mr-1 text-gray-400 hover:text-white" on:click|stopPropagation={toggleExpand}>
                {isExpanded ? '▼' : '▶'}
            </button>
        {:else}
            <div class="w-4 h-4 mr-1"></div>
        {/if}
        
        <span class="px-1.5 py-0.5 text-xs font-mono rounded bg-blue-500/20 text-blue-400 mr-2 border border-blue-500/30">
            {node.tag_type}
        </span>
        
        <span class="text-sm text-gray-300 truncate flex-1">
            {#if node.alt_text}
                <span class="text-green-400 italic text-xs mr-1" title="Alt Text">Alt: {node.alt_text}</span>
            {/if}
            {#if node.language}
                <span class="text-yellow-400 text-xs mr-1" title="Language">[{node.language}]</span>
            {/if}
            <span class="text-gray-500 text-xs">#{node.id}</span>
        </span>
    </div>
    
    {#if isExpanded && node.children && node.children.length > 0}
        <div class="children-container border-l border-white/10 ml-2">
            {#each node.children as child (child.id)}
                <svelte:self 
                    node={child} 
                    level={level + 1}
                    {canReorder}
                    on:nodeSelected={forwardNodeSelected}
                    on:nodeReorder={forwardNodeReorder}
                />
            {/each}
        </div>
    {/if}
</div>

<style>
    .tag-tree-node {
        position: relative;
    }
</style>
