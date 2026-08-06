<script>
    import { createEventDispatcher } from 'svelte';
    import FeatureNotice from './FeatureNotice.svelte';
    import { FeatureUnavailableError, guardFeatureAction } from '../featureActionGuard.js';
    import { getFeaturePresentation, getRuntimeFeatureContext } from '../featureRegistry.js';
    export let scale = 1.0;
    export let pageIndex = 0;
    
    const dispatch = createEventDispatcher();
    const featureContext = getRuntimeFeatureContext();
    const featurePresentation = getFeaturePresentation('reading_order_editing', featureContext);
    let unavailableMessage = "";
    
    // In a real PDF parser, we would extract the physical bounding boxes for each MCID (Marked Content ID)
    // linked to the StructElem. For demonstration, we generate mock blocks representing the reading order flow.
    let blocks = [
        { id: 1, x: 50, y: 50, w: 400, h: 60, type: 'H1', text: 'Main Title', order: 1 },
        { id: 2, x: 50, y: 130, w: 400, h: 100, type: 'P', text: 'Introduction paragraph...', order: 2 },
        { id: 3, x: 50, y: 250, w: 200, h: 150, type: 'Figure', text: '[Image]', order: 3 },
        { id: 4, x: 270, y: 250, w: 180, h: 150, type: 'P', text: 'Text beside image', order: 4 }
    ];
    
    let draggingBlock = null;
    
    function handleDragStart(e, block) {
        draggingBlock = block;
        e.dataTransfer.effectAllowed = 'move';
        e.dataTransfer.setData('text/plain', block.id.toString());
    }
    
    async function handleDrop(e, targetBlock) {
        e.preventDefault();
        unavailableMessage = "";
        try {
            return await guardFeatureAction('reading_order_editing', featureContext, () => {
                if (!draggingBlock || draggingBlock.id === targetBlock.id) return;
                const tempOrder = targetBlock.order;
                targetBlock.order = draggingBlock.order;
                draggingBlock.order = tempOrder;
                blocks = [...blocks].sort((a, b) => a.order - b.order);
                dispatch('reorder', {
                    blocks: blocks.map(b => ({ id: b.id, newOrder: b.order }))
                });
                draggingBlock = null;
            });
        } catch (error) {
            if (error instanceof FeatureUnavailableError) {
                unavailableMessage = error.presentation?.warning || error.message;
                return;
            }
            throw error;
        }
    }
    
    function handleDragOver(e) {
        e.preventDefault();
        e.dataTransfer.dropEffect = 'move';
    }

</script>

<div class="absolute inset-0 pointer-events-none z-30 opacity-75">
    <FeatureNotice featureId="reading_order_editing" compact />
    {#if unavailableMessage}
        <p class="warning">{unavailableMessage}</p>
    {/if}
    {#each blocks as block}
        <div 
            class="absolute border-2 border-blue-500 bg-blue-500/10 pointer-events-auto cursor-move flex items-start p-1 transition-all"
            style="left: {block.x * scale}px; top: {block.y * scale}px; width: {block.w * scale}px; height: {block.h * scale}px;"
            draggable={featurePresentation.runnable}
            on:dragstart={(e) => handleDragStart(e, block)}
            on:dragover={handleDragOver}
            on:drop={(e) => handleDrop(e, block)}
        >
            <div class="bg-blue-600 text-white rounded-full w-6 h-6 flex items-center justify-center font-bold text-xs shadow-md -ml-3 -mt-3">
                {block.order}
            </div>
            <div class="text-[10px] bg-blue-500/80 text-white px-1 ml-1 rounded">
                {block.type}
            </div>
        </div>
    {/each}
</div>

<style>
    /* Styling for the drag-drop visualization */
    div[draggable="true"]:hover {
        background-color: rgba(59, 130, 246, 0.2);
    }
</style>
