<script>
    import { createEventDispatcher } from 'svelte';
    import FeatureNotice from './FeatureNotice.svelte';
    import { FeatureUnavailableError, guardFeatureAction } from '../featureActionGuard.js';
    import { getFeaturePresentation, getRuntimeFeatureContext } from '../featureRegistry.js';

    export let isOpen = false;
    const dispatch = createEventDispatcher();
    const featureContext = getRuntimeFeatureContext();
    const featurePresentation = getFeaturePresentation('signature_settings', featureContext);
    let unavailableMessage = "";

    // In a real app, these would be loaded from and saved to a persistent store
    // e.g., tauri-plugin-store
    let tsaUrl = "http://timestamp.digicert.com";
    let tsaUsername = "";
    let tsaPassword = "";
    
    let newCertPath = "";
    let trustStore = [
        "DigiCert Assured ID Root CA",
        "GlobalSign Root CA"
    ];

    function close() {
        isOpen = false;
        dispatch('close');
    }

    async function runSettingsAction(callback) {
        unavailableMessage = "";
        try {
            return await guardFeatureAction('signature_settings', featureContext, callback);
        } catch (error) {
            if (error instanceof FeatureUnavailableError) {
                unavailableMessage = error.presentation?.warning || error.message;
                return;
            }
            throw error;
        }
    }

    async function saveSettings() {
        return await runSettingsAction(() => {
            close();
        });
    }

    async function addCert() {
        return await runSettingsAction(() => {
            if (newCertPath) {
                trustStore = [...trustStore, newCertPath];
                newCertPath = "";
            }
        });
    }

    async function removeCert(index) {
        return await runSettingsAction(() => {
            trustStore = trustStore.filter((_, i) => i !== index);
        });
    }
</script>

{#if isOpen}
    <div class="modal-overlay" on:click={close}>
        <div class="modal-content" on:click|stopPropagation>
            <div class="modal-header">
                <h2>Signature Settings (PAdES / TSA)</h2>
                <button class="close-btn" on:click={close}>×</button>
            </div>

            <div class="modal-body">
                <FeatureNotice featureId="signature_settings" />
                {#if unavailableMessage}
                    <p class="warning">{unavailableMessage}</p>
                {/if}
                <section class="settings-section">
                    <h3>Time-Stamping Authority (RFC 3161)</h3>
                    <div class="form-group">
                        <label>TSA URL:</label>
                        <input type="text" bind:value={tsaUrl} placeholder="http://..." disabled={!featurePresentation.runnable} />
                    </div>
                    <div class="form-group">
                        <label>Username (optional):</label>
                        <input type="text" bind:value={tsaUsername} disabled={!featurePresentation.runnable} />
                    </div>
                    <div class="form-group">
                        <label>Password (optional):</label>
                        <input type="password" bind:value={tsaPassword} disabled={!featurePresentation.runnable} />
                    </div>
                </section>

                <section class="settings-section">
                    <h3>Trust Store (Trusted Root Certificates)</h3>
                    <ul class="trust-list">
                        {#each trustStore as cert, i}
                            <li>
                                <span>{cert}</span>
                                <button class="btn-remove" on:click={() => removeCert(i)} disabled={!featurePresentation.runnable}>Remove</button>
                            </li>
                        {/each}
                    </ul>
                    <div class="add-cert-form">
                        <input type="text" bind:value={newCertPath} placeholder="Path to .crt or .pem file..." disabled={!featurePresentation.runnable} />
                        <button class="btn secondary" on:click={addCert} disabled={!featurePresentation.runnable}>Add</button>
                    </div>
                </section>
            </div>

            <div class="modal-footer">
                <button class="btn secondary" on:click={close}>Cancel</button>
                <button class="btn primary" on:click={saveSettings} disabled={!featurePresentation.runnable}>Unavailable · MOCK</button>
            </div>
        </div>
    </div>
{/if}

<style>
    /* Similar styles as the Wizard modal */
    .modal-overlay {
        position: fixed; top: 0; left: 0; right: 0; bottom: 0;
        background: rgba(0,0,0,0.7); display: flex; align-items: center; justify-content: center; z-index: 1000;
        backdrop-filter: blur(4px);
    }

    .modal-content {
        background: var(--surface-color, #1e1e1e); border-radius: 12px; width: 500px; max-width: 90vw;
        color: var(--text-color, #eee); border: 1px solid var(--border-color, #333);
        display: flex; flex-direction: column;
    }

    .modal-header {
        padding: 20px; border-bottom: 1px solid var(--border-color, #333); display: flex; justify-content: space-between;
    }
    
    .modal-header h2 { margin: 0; font-size: 1.2em; }
    .close-btn { background: none; border: none; color: inherit; font-size: 1.5em; cursor: pointer; }

    .modal-body { padding: 20px; max-height: 60vh; overflow-y: auto; }

    .settings-section { margin-bottom: 30px; }
    .settings-section h3 { color: #2196f3; margin-top: 0; border-bottom: 1px solid #333; padding-bottom: 8px; margin-bottom: 16px; font-size: 1.1em;}

    .form-group { margin-bottom: 12px; }
    .form-group label { display: block; margin-bottom: 6px; font-size: 0.9em; color: #ccc;}
    .form-group input { width: 100%; padding: 8px; border-radius: 4px; border: 1px solid #444; background: rgba(0,0,0,0.2); color: white; }

    .trust-list { list-style: none; padding: 0; margin: 0 0 12px 0; }
    .trust-list li { display: flex; justify-content: space-between; align-items: center; padding: 8px; background: #2a2a2a; margin-bottom: 4px; border-radius: 4px; font-size: 0.9em;}
    
    .btn-remove { background: #f44336; color: white; border: none; padding: 4px 8px; border-radius: 4px; cursor: pointer; font-size: 0.8em;}
    
    .add-cert-form { display: flex; gap: 8px; }
    .add-cert-form input { flex: 1; padding: 8px; border-radius: 4px; border: 1px solid #444; background: transparent; color: white;}

    .modal-footer { padding: 16px 20px; border-top: 1px solid var(--border-color, #333); display: flex; justify-content: flex-end; gap: 12px; }
    .btn { padding: 8px 16px; border: none; border-radius: 4px; cursor: pointer; font-weight: bold; }
    .btn.primary { background: #2196f3; color: white; }
    .btn.secondary { background: #555; color: white; }
</style>
