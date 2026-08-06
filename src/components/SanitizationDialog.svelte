<script>
    import { createEventDispatcher } from 'svelte';
    import FeatureNotice from './FeatureNotice.svelte';
    import { FeatureUnavailableError, guardFeatureAction } from '../featureActionGuard.js';
    import { getFeaturePresentation, getRuntimeFeatureContext } from '../featureRegistry.js';
    const dispatch = createEventDispatcher();
    const featureContext = getRuntimeFeatureContext();
    const featurePresentation = getFeaturePresentation('sanitization', featureContext);
    
    export let show = false;
    let unavailableMessage = "";
    
    let options = {
        remove_metadata: true,
        remove_annotations: false,
        remove_javascript: true,
        remove_embedded_files: true,
        remove_hidden_layers: true
    };
    
    function close() {
        show = false;
        dispatch('close');
    }
    
    async function confirm() {
        unavailableMessage = "";
        try {
            return await guardFeatureAction('sanitization', featureContext, () => {
                dispatch('confirm', options);
                show = false;
            });
        } catch (error) {
            if (error instanceof FeatureUnavailableError) {
                unavailableMessage = error.presentation?.warning || error.message;
                return;
            }
            throw error;
        }
    }
</script>

{#if show}
<div class="modal-backdrop" on:click|self={close}>
    <div class="modal-content">
        <div class="modal-header">
            <div class="icon-warning">
                <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
            </div>
            <h2>Belge Temizleme ve Arındırma (Sanitization)</h2>
        </div>
        
        <div class="modal-body">
            <FeatureNotice featureId="sanitization" />
            <div class="alert">
                <strong>Kullanılamıyor:</strong> Bu sürüm, gizli verilerin PDF nesne ağacından eksiksiz ve güvenli biçimde kaldırıldığını garanti etmez. Temizleme işlemi çalıştırılmaz.
            </div>
            {#if unavailableMessage}
                <p class="alert">{unavailableMessage}</p>
            {/if}
            
            <p>Lütfen temizlenecek ögeleri seçin:</p>
            
            <div class="options-grid">
                <label class="option-card" class:selected={options.remove_metadata}>
                    <input type="checkbox" bind:checked={options.remove_metadata} disabled={!featurePresentation.runnable} />
                    <div class="opt-text">
                        <span class="opt-title">Meta Veriler (XMP, Info)</span>
                        <span class="opt-desc">Yazar, oluşturan, başlık, değişiklik tarihi gibi gizli belge özellikleri.</span>
                    </div>
                </label>
                
                <label class="option-card" class:selected={options.remove_javascript}>
                    <input type="checkbox" bind:checked={options.remove_javascript} disabled={!featurePresentation.runnable} />
                    <div class="opt-text">
                        <span class="opt-title">JavaScript & Aksiyonlar</span>
                        <span class="opt-desc">Zararlı olabilecek gömülü JS kodları ve dış bağlantı (URI) eylemleri.</span>
                    </div>
                </label>
                
                <label class="option-card" class:selected={options.remove_embedded_files}>
                    <input type="checkbox" bind:checked={options.remove_embedded_files} disabled={!featurePresentation.runnable} />
                    <div class="opt-text">
                        <span class="opt-title">Gömülü Dosyalar (Attachments)</span>
                        <span class="opt-desc">PDF içine gizlenmiş başka dosyalar.</span>
                    </div>
                </label>
                
                <label class="option-card" class:selected={options.remove_hidden_layers}>
                    <input type="checkbox" bind:checked={options.remove_hidden_layers} disabled={!featurePresentation.runnable} />
                    <div class="opt-text">
                        <span class="opt-title">Gizli Katmanlar (OCG)</span>
                        <span class="opt-desc">Görünür olmayan vektör veya resim katmanları.</span>
                    </div>
                </label>
                
                <label class="option-card" class:selected={options.remove_annotations}>
                    <input type="checkbox" bind:checked={options.remove_annotations} disabled={!featurePresentation.runnable} />
                    <div class="opt-text">
                        <span class="opt-title">Tüm Açıklamalar (Annotations)</span>
                        <span class="opt-desc">Kullanıcı yorumları, çizimler, form alanları (Dikkat: Belge görünümünü etkileyebilir).</span>
                    </div>
                </label>
            </div>
        </div>
        
        <div class="modal-footer">
            <button class="btn-cancel" on:click={close}>İptal</button>
            <button class="btn-confirm" on:click={confirm} disabled={!featurePresentation.runnable}>Kullanılamıyor · DISABLED</button>
        </div>
    </div>
</div>
{/if}

<style>
    .modal-backdrop {
        position: fixed;
        top: 0; left: 0; right: 0; bottom: 0;
        background: rgba(0,0,0,0.7);
        backdrop-filter: blur(4px);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 1000;
    }
    
    .modal-content {
        background: var(--bg-surface, #1e1e1e);
        width: 600px;
        max-width: 90vw;
        border-radius: 12px;
        border: 1px solid var(--border-color, #333);
        box-shadow: 0 20px 40px rgba(0,0,0,0.5);
        display: flex;
        flex-direction: column;
        color: var(--text-color, #e0e0e0);
    }
    
    .modal-header {
        padding: 24px 24px 16px;
        display: flex;
        align-items: center;
        gap: 16px;
        border-bottom: 1px solid var(--border-color, #333);
    }
    
    .icon-warning {
        color: #f59e0b;
        background: rgba(245, 158, 11, 0.1);
        padding: 12px;
        border-radius: 50%;
        display: flex;
        align-items: center;
        justify-content: center;
    }
    
    h2 {
        margin: 0;
        font-size: 20px;
        font-weight: 600;
    }
    
    .modal-body {
        padding: 24px;
        overflow-y: auto;
        max-height: 60vh;
    }
    
    .alert {
        background: rgba(239, 68, 68, 0.1);
        border-left: 4px solid #ef4444;
        padding: 12px 16px;
        border-radius: 4px;
        margin-bottom: 24px;
        font-size: 14px;
        line-height: 1.5;
        color: #ffb3b3;
    }
    
    .alert strong {
        color: #ef4444;
    }
    
    .options-grid {
        display: grid;
        grid-template-columns: 1fr;
        gap: 12px;
        margin-top: 16px;
    }
    
    .option-card {
        display: flex;
        align-items: flex-start;
        gap: 12px;
        padding: 16px;
        background: rgba(255,255,255,0.03);
        border: 1px solid var(--border-color, #333);
        border-radius: 8px;
        cursor: pointer;
        transition: all 0.2s;
    }
    
    .option-card:hover {
        background: rgba(255,255,255,0.05);
    }
    
    .option-card.selected {
        background: rgba(59, 130, 246, 0.1);
        border-color: #3b82f6;
    }
    
    .option-card input[type="checkbox"] {
        margin-top: 4px;
        width: 18px;
        height: 18px;
        accent-color: #3b82f6;
    }
    
    .opt-text {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    
    .opt-title {
        font-weight: 500;
        font-size: 15px;
        color: white;
    }
    
    .opt-desc {
        font-size: 13px;
        color: #aaa;
        line-height: 1.4;
    }
    
    .modal-footer {
        padding: 16px 24px;
        border-top: 1px solid var(--border-color, #333);
        display: flex;
        justify-content: flex-end;
        gap: 12px;
    }
    
    button {
        padding: 10px 20px;
        border-radius: 6px;
        font-size: 14px;
        font-weight: 500;
        cursor: pointer;
        border: none;
    }
    
    .btn-cancel {
        background: transparent;
        color: #e0e0e0;
        border: 1px solid #555;
    }
    
    .btn-cancel:hover {
        background: rgba(255,255,255,0.05);
    }
    
    .btn-confirm {
        background: #ef4444;
        color: white;
    }
    
    .btn-confirm:hover {
        background: #dc2626;
    }
</style>
