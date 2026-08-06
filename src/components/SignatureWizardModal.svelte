<script>
    import {
        listSystemCertificatesAction,
        signPdfPadesAction,
    } from '../services/actions.js';
    import { createEventDispatcher, onMount } from 'svelte';
    import FeatureNotice from './FeatureNotice.svelte';
    import { FeatureUnavailableError, guardFeatureAction } from '../featureActionGuard.js';
    import { getFeaturePresentation, getRuntimeFeatureContext } from '../featureRegistry.js';

    export let isOpen = false;
    export let displayPath = "";
    export let sessionId = "";
    export let contentGeneration = 0;
    const dispatch = createEventDispatcher();
    const featureContext = getRuntimeFeatureContext();
    const featurePresentation = getFeaturePresentation('pades_signing', featureContext);

    let step = 1;
    let certificates = [];
    let selectedCertId = "";
    
    let pin = "";
    let reason = "I approve this document";
    let location = "";
    
    let loading = false;
    let error = null;

    onMount(() => {
        if (isOpen && featurePresentation.runnable) {
            void loadCertificates();
        }
    });

    $: if (isOpen && featurePresentation.runnable && step === 1 && certificates.length === 0) {
        loadCertificates();
    }

    async function loadCertificates() {
        try {
            return await guardFeatureAction('pades_signing', featureContext, async () => {
                loading = true;
                error = null;
                try {
                    certificates = await listSystemCertificatesAction(
                        featureContext,
                    );
                    if (certificates.length > 0) {
                        selectedCertId = certificates[0].id;
                    }
                } finally {
                    loading = false;
                }
            });
        } catch (err) {
            if (err instanceof FeatureUnavailableError) {
                error = err.presentation?.warning || err.message;
                return;
            }
            throw err;
        }
    }

    function close() {
        isOpen = false;
        step = 1;
        pin = "";
        dispatch('close');
    }

    function nextStep() {
        if (step === 1 && !selectedCertId) {
            error = "Please select a certificate.";
            return;
        }
        error = null;
        step++;
    }

    function prevStep() {
        error = null;
        step--;
    }

    async function signDocument() {
        try {
            return await guardFeatureAction('pades_signing', featureContext, async () => {
                if (!pin) {
                    error = "PIN is required.";
                    return;
                }

                loading = true;
                error = null;
                try {
                    const outputPath = displayPath.replace('.pdf', '_signed.pdf');
                    await signPdfPadesAction(
                        sessionId,
                        contentGeneration,
                        outputPath,
                        selectedCertId,
                        pin,
                        {
                            reason,
                            location,
                            contact_info: null,
                            visual_signature: null
                        },
                        featureContext,
                    );
                } finally {
                    loading = false;
                }
            });
        } catch (err) {
            if (err instanceof FeatureUnavailableError) {
                error = err.presentation?.warning || err.message;
                return;
            }
            throw err;
        }
    }
</script>

{#if isOpen}
    <div class="modal-overlay" on:click={close}>
        <div class="modal-content" on:click|stopPropagation>
            <div class="modal-header">
                <h2>Digital Signature Wizard</h2>
                <button class="close-btn" on:click={close}>×</button>
            </div>

            <div class="modal-body">
                <FeatureNotice featureId="pades_signing" />
                {#if error}
                    <div class="error-msg">{error}</div>
                {/if}

                {#if step === 1}
                    <h3>Step 1: Select Certificate (Smart Card / Token)</h3>
                    {#if loading}
                        <p>Scanning for hardware tokens and certificates...</p>
                    {:else if certificates.length === 0}
                        <p>No certificates found. Please insert your smart card or USB token.</p>
                        <button class="btn secondary" on:click={loadCertificates} disabled={!featurePresentation.runnable}>Refresh</button>
                    {:else}
                        <div class="cert-list">
                            {#each certificates as cert}
                                <label class="cert-item">
                                    <input type="radio" name="cert" value={cert.id} bind:group={selectedCertId} />
                                    <div class="cert-info">
                                        <strong>{cert.subject}</strong>
                                        <span>Token: {cert.token_label}</span>
                                        <span>Issuer: {cert.issuer}</span>
                                        <span>Valid until: {new Date(cert.not_after).toLocaleDateString()}</span>
                                    </div>
                                </label>
                            {/each}
                        </div>
                    {/if}
                {/if}

                {#if step === 2}
                    <h3>Step 2: Signature Details</h3>
                    <div class="form-group">
                        <label for="reason">Reason for signing:</label>
                        <input id="reason" type="text" bind:value={reason} placeholder="I approve this document" />
                    </div>
                    <div class="form-group">
                        <label for="location">Location (optional):</label>
                        <input id="location" type="text" bind:value={location} placeholder="e.g., Istanbul, TR" />
                    </div>
                    <!-- Future: Visual Signature Box Drawing Integration -->
                    <p class="info-text">Bu ekran yalnız arayüz taslağıdır; görünür veya görünmez imza uygulanmaz.</p>
                {/if}

                {#if step === 3}
                    <h3>Step 3: Authenticate and Sign</h3>
                    <div class="form-group">
                        <label for="pin">Enter Smart Card PIN:</label>
                        <input id="pin" type="password" bind:value={pin} placeholder="****" autocomplete="new-password" />
                    </div>
                    <p class="warning-text">Make sure your hardware token is plugged in.</p>
                {/if}
            </div>

            <div class="modal-footer">
                {#if step > 1}
                    <button class="btn secondary" on:click={prevStep} disabled={loading}>Back</button>
                {/if}
                
                {#if step < 3}
                    <button class="btn primary" on:click={nextStep} disabled={!featurePresentation.runnable || loading || (step === 1 && !selectedCertId)}>Next</button>
                {:else}
                    <button class="btn success" on:click={signDocument} disabled={!featurePresentation.runnable || loading}>
                        {loading ? 'Signing...' : 'Unavailable · MOCK'}
                    </button>
                {/if}
            </div>
        </div>
    </div>
{/if}

<style>
    .modal-overlay {
        position: fixed;
        top: 0; left: 0; right: 0; bottom: 0;
        background: rgba(0,0,0,0.7);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 1000;
        backdrop-filter: blur(4px);
    }

    .modal-content {
        background: var(--surface-color, #1e1e1e);
        border-radius: 12px;
        width: 500px;
        max-width: 90vw;
        box-shadow: 0 10px 30px rgba(0,0,0,0.5);
        display: flex;
        flex-direction: column;
        color: var(--text-color, #eee);
        border: 1px solid var(--border-color, #333);
    }

    .modal-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        padding: 20px;
        border-bottom: 1px solid var(--border-color, #333);
    }

    .modal-header h2 { margin: 0; font-size: 1.2em; }

    .close-btn {
        background: none; border: none; color: inherit; font-size: 1.5em; cursor: pointer;
    }

    .modal-body {
        padding: 20px;
        min-height: 250px;
    }

    .modal-body h3 {
        margin-top: 0;
        margin-bottom: 20px;
        color: #4caf50;
    }

    .error-msg {
        background: rgba(244, 67, 54, 0.2);
        color: #ff5252;
        padding: 10px;
        border-radius: 6px;
        margin-bottom: 16px;
        border: 1px solid #ff5252;
    }

    .cert-list {
        display: flex;
        flex-direction: column;
        gap: 12px;
        max-height: 200px;
        overflow-y: auto;
    }

    .cert-item {
        display: flex;
        gap: 12px;
        align-items: flex-start;
        padding: 12px;
        background: var(--surface-light, #2a2a2a);
        border: 1px solid var(--border-color, #444);
        border-radius: 8px;
        cursor: pointer;
        transition: border-color 0.2s;
    }

    .cert-item:hover { border-color: #4caf50; }

    .cert-info { display: flex; flex-direction: column; font-size: 0.9em; color: #aaa; }
    .cert-info strong { color: #fff; font-size: 1.1em; margin-bottom: 4px; }

    .form-group {
        margin-bottom: 16px;
    }

    .form-group label {
        display: block; margin-bottom: 8px; color: #ccc;
    }

    .form-group input {
        width: 100%;
        padding: 10px;
        border-radius: 6px;
        border: 1px solid var(--border-color, #444);
        background: rgba(0,0,0,0.2);
        color: white;
    }

    .info-text { font-size: 0.9em; color: #888; font-style: italic; }
    .warning-text { font-size: 0.9em; color: #ff9800; }

    .modal-footer {
        padding: 20px;
        border-top: 1px solid var(--border-color, #333);
        display: flex;
        justify-content: flex-end;
        gap: 12px;
    }

    .btn {
        padding: 8px 16px; border: none; border-radius: 6px; cursor: pointer; font-weight: bold;
    }
    .btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .btn.primary { background: #2196f3; color: white; }
    .btn.secondary { background: #555; color: white; }
    .btn.success { background: #4caf50; color: white; }
</style>
