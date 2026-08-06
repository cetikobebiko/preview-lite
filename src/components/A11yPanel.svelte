<script>
    import { createEventDispatcher } from 'svelte';
    import {
        getSessionStructureTree,
        validateSessionPdfUa,
    } from '../services/backend/session.js';
    import TagTreeExplorer from './TagTreeExplorer.svelte';
    import FeatureNotice from './FeatureNotice.svelte';
    import { FeatureUnavailableError, guardFeatureAction } from '../featureActionGuard.js';
    import { getFeaturePresentation, getRuntimeFeatureContext } from '../featureRegistry.js';
    
    let {
        sessionId = "",
        contentGeneration = 0,
    } = $props();
    
    const dispatch = createEventDispatcher();
    const featureContext = getRuntimeFeatureContext();
    const autoTagPresentation = getFeaturePresentation('pdf_auto_tag', featureContext);
    const readingOrderPresentation = getFeaturePresentation('reading_order_editing', featureContext);
    
    let activeTab = $state('validate');
    let isLoading = $state(false);
    let errorMsg = $state("");
    
    // Validate State
    let report = $state(null);
    
    // Tree State
    let tagTree = $state(null);
    let selectedNode = $state(null);
    let editAltText = $state("");
    let editLang = $state("");
    
    // Preview State
    let previewText = $state("");
    let requestSequence = 0;
    
    $effect(() => {
        const requestSessionId = sessionId;
        const requestGeneration = contentGeneration;
        const sequence = ++requestSequence;
        report = null;
        tagTree = null;
        selectedNode = null;
        previewText = "";
        errorMsg = "";
        if (!requestSessionId) {
            isLoading = false;
            return;
        }
        void refreshSessionData(
            requestSessionId,
            requestGeneration,
            sequence,
        );
    });
    
    async function refreshSessionData(
        requestSessionId,
        requestGeneration,
        sequence,
    ) {
        isLoading = true;
        const [validationResult, treeResult] = await Promise.allSettled([
            validateSessionPdfUa(requestSessionId, requestGeneration),
            getSessionStructureTree(requestSessionId, requestGeneration),
        ]);
        if (
            sequence !== requestSequence ||
            requestSessionId !== sessionId ||
            requestGeneration !== contentGeneration
        ) return;

        if (validationResult.status === "fulfilled") {
            report = validationResult.value;
        } else {
            errorMsg = validationResult.reason?.toString?.() ?? String(validationResult.reason);
        }
        if (treeResult.status === "fulfilled") {
            tagTree = treeResult.value;
            generatePreviewText(tagTree);
        } else {
            console.error("Tree load error:", treeResult.reason);
            tagTree = null;
        }
        isLoading = false;
    }
    
    function generatePreviewText(node, depth = 0) {
        if (!node) return;
        if (depth === 0) {
            previewText = "--- Screen Reader Preview ---\n\n";
        }
        
        let prefix = "  ".repeat(depth);
        let content = "";
        
        if (node.tag_type !== 'DocumentRoot') {
            content += `${prefix}[${node.tag_type}] `;
            if (node.alt_text) content += `Alt: "${node.alt_text}" `;
            if (node.language) content += `(Lang: ${node.language}) `;
            content += "\n";
            previewText += content;
        }
        
        if (node.children) {
            for (let child of node.children) {
                generatePreviewText(child, depth + 1);
            }
        }
    }
    
    function handleNodeSelected(event) {
        selectedNode = event.detail;
        editAltText = selectedNode.alt_text || "";
        editLang = selectedNode.language || "";
    }
    
    async function handleNodeReorder() {
        try {
            await guardFeatureAction('reading_order_editing', featureContext, () => {});
        } catch (error) {
            if (error instanceof FeatureUnavailableError) {
                errorMsg = error.presentation?.warning || error.message;
                return;
            }
            throw error;
        }
    }

</script>

<div class="h-full flex flex-col bg-[#1e1e1e] text-white border-l border-white/10 w-80 shadow-2xl z-50">
    <div class="p-4 border-b border-white/10 flex justify-between items-center bg-[#252526]">
        <h2 class="text-lg font-semibold flex items-center gap-2">
            <svg class="w-5 h-5 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"></path><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"></path></svg>
            Accessibility
        </h2>
        <button
            class="p-1 hover:bg-white/10 rounded"
            onclick={() => dispatch('close')}
            aria-label="Erişilebilirlik panelini kapat"
        >
            <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path></svg>
        </button>
    </div>
    
    <div class="flex border-b border-white/10 text-sm">
        <button class="flex-1 py-2 text-center {activeTab === 'validate' ? 'bg-blue-600 font-bold' : 'hover:bg-white/5'}" onclick={() => activeTab = 'validate'}>Check</button>
        <button class="flex-1 py-2 text-center {activeTab === 'tree' ? 'bg-blue-600 font-bold' : 'hover:bg-white/5'}" onclick={() => activeTab = 'tree'}>Tags</button>
        <button class="flex-1 py-2 text-center {activeTab === 'preview' ? 'bg-blue-600 font-bold' : 'hover:bg-white/5'}" onclick={() => activeTab = 'preview'}>Preview</button>
    </div>
    
    <div class="flex-1 overflow-y-auto p-4 custom-scrollbar">
        <FeatureNotice featureId="pdf_ua_validation" />
        {#if isLoading}
            <div class="text-center text-gray-400 py-4">Processing...</div>
        {/if}
        
        {#if errorMsg}
            <div class="bg-red-500/20 text-red-400 p-3 rounded mb-4 text-sm border border-red-500/30">
                {errorMsg}
            </div>
        {/if}
        
        {#if activeTab === 'validate'}
            {#if report}
                <div class="mb-6 flex flex-col items-center">
                    <div class="text-4xl font-bold {report.score > 80 ? 'text-green-400' : report.score > 50 ? 'text-yellow-400' : 'text-red-400'}">
                        {report.score}/100
                    </div>
                    <div class="text-sm text-gray-400 mt-1">Heuristic PDF/UA estimate — compliance validation değildir</div>
                </div>
                
                <h3 class="font-semibold mb-2 text-sm text-gray-300 uppercase tracking-wider">Report</h3>
                <div class="space-y-2">
                    {#each report.issues as issue, index (`${index}:${issue}`)}
                        <div class="p-3 bg-white/5 rounded border border-white/10 text-sm flex gap-2">
                            <span class="text-blue-400 mt-0.5">•</span>
                            <span>{issue}</span>
                        </div>
                    {/each}
                </div>
                
                {#if !report.is_tagged}
                    <FeatureNotice featureId="pdf_auto_tag" compact />
                    <button
                        class="mt-4 w-full py-2 bg-blue-600 hover:bg-blue-500 rounded font-medium text-sm transition-colors shadow-lg"
                        disabled={!autoTagPresentation.runnable}
                    >
                        Auto-Tag kullanılamıyor · DISABLED
                    </button>
                {/if}
            {:else if !isLoading}
                <div class="text-center text-gray-400 mt-10">No report available. Open a PDF to analyze.</div>
            {/if}
            
        {:else if activeTab === 'tree'}
            {#if tagTree}
                <div class="bg-[#181818] rounded border border-white/10 p-2 mb-4 overflow-x-auto">
                    <FeatureNotice featureId="reading_order_editing" compact />
                    <TagTreeExplorer
                        node={tagTree}
                        isRoot={true}
                        canReorder={readingOrderPresentation.runnable}
                        on:nodeSelected={handleNodeSelected}
                        on:nodeReorder={handleNodeReorder}
                    />
                </div>
                
                {#if selectedNode}
                    <div class="bg-[#252526] p-3 rounded border border-white/10">
                        <div class="flex justify-between items-center mb-2">
                            <h3 class="font-semibold text-sm">Edit Tag: {selectedNode.tag_type}</h3>
                            <span class="text-xs text-gray-500">#{selectedNode.id}</span>
                        </div>
                        
                        <div class="space-y-3">
                            <div>
                                <label for="a11y-alt-text" class="block text-xs text-gray-400 mb-1">Alternative Text (/Alt)</label>
                                <textarea id="a11y-alt-text" class="w-full bg-[#1e1e1e] border border-white/20 rounded p-2 text-sm focus:border-blue-500 focus:outline-none opacity-50 cursor-not-allowed" rows="2" bind:value={editAltText} placeholder="Enter alt text..." disabled readonly></textarea>
                            </div>
                            <div>
                                <label for="a11y-language" class="block text-xs text-gray-400 mb-1">Language (/Lang)</label>
                                <input id="a11y-language" type="text" class="w-full bg-[#1e1e1e] border border-white/20 rounded p-2 text-sm focus:border-blue-500 focus:outline-none opacity-50 cursor-not-allowed" bind:value={editLang} placeholder="e.g. en-US, tr-TR" disabled readonly>
                            </div>
                            <button class="w-full py-1.5 bg-green-600/50 cursor-not-allowed rounded text-sm font-medium transition-colors" disabled>
                                Save Changes (Not Supported)
                            </button>
                        </div>
                    </div>
                {:else}
                    <div class="text-center text-gray-500 text-sm mt-4 italic">
                        Select a tag from the tree to edit its properties.
                    </div>
                {/if}
            {:else if !isLoading}
                <div class="text-center text-gray-400 mt-10">Document is not tagged. Cannot view Structure Tree.</div>
            {/if}
            
        {:else if activeTab === 'preview'}
             <div class="bg-[#181818] rounded border border-white/10 p-3 h-full">
                <pre class="text-xs text-green-400 font-mono whitespace-pre-wrap overflow-y-auto h-full">{previewText}</pre>
            </div>
        {/if}
    </div>
</div>

<style>
    .custom-scrollbar::-webkit-scrollbar {
        width: 6px;
    }
    .custom-scrollbar::-webkit-scrollbar-track {
        background: transparent;
    }
    .custom-scrollbar::-webkit-scrollbar-thumb {
        background: rgba(255, 255, 255, 0.2);
        border-radius: 3px;
    }
    .custom-scrollbar::-webkit-scrollbar-thumb:hover {
        background: rgba(255, 255, 255, 0.3);
    }
</style>
