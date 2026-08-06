<script>
    import {
        enableLtvAction,
        verifyPdfSignaturesAction,
    } from '../services/actions.js';
    import FeatureNotice from './FeatureNotice.svelte';
    import { FeatureUnavailableError, guardFeatureAction } from '../featureActionGuard.js';
    import { getFeaturePresentation, getRuntimeFeatureContext } from '../featureRegistry.js';

    let {
        isOpen = false,
        sessionId = "",
        contentGeneration = 0,
        onclose = () => {},
    } = $props();
    const featureContext = getRuntimeFeatureContext();
    const verificationPresentation = getFeaturePresentation('signature_verification', featureContext);
    const ltvPresentation = getFeaturePresentation('ltv', featureContext);

    let signatures = $state([]);
    let loading = $state(false);
    let error = $state(null);
    let verifiedKey = $state("");
    let observedIdentityKey = $state("");
    let requestSequence = 0;

    function requestIsCurrent(sequence, requestSessionId, requestGeneration) {
        return (
            sequence === requestSequence &&
            requestSessionId === sessionId &&
            requestGeneration === contentGeneration
        );
    }

    async function verifySignatures(
        requestSessionId = sessionId,
        requestGeneration = contentGeneration,
    ) {
        if (!requestSessionId) return;
        verifiedKey = `${requestSessionId}:${requestGeneration}`;
        const sequence = ++requestSequence;
        try {
            return await guardFeatureAction('signature_verification', featureContext, async () => {
                loading = true;
                error = null;
                try {
                    const result = await verifyPdfSignaturesAction(
                        requestSessionId,
                        requestGeneration,
                        featureContext,
                    );
                    if (!requestIsCurrent(sequence, requestSessionId, requestGeneration)) return;
                    signatures = result;
                } finally {
                    if (requestIsCurrent(sequence, requestSessionId, requestGeneration)) {
                        loading = false;
                    }
                }
            });
        } catch (err) {
            if (
                err instanceof FeatureUnavailableError &&
                requestIsCurrent(sequence, requestSessionId, requestGeneration)
            ) {
                error = err.presentation?.warning || err.message;
                return;
            }
            throw err;
        }
    }

    async function enableLTV(signatureId) {
        if (!sessionId) return;
        try {
            return await guardFeatureAction('ltv', featureContext, async () => {
                await enableLtvAction(
                    sessionId,
                    contentGeneration,
                    signatureId,
                    featureContext,
                );
                await verifySignatures();
            });
        } catch (err) {
            if (err instanceof FeatureUnavailableError) {
                error = err.presentation?.warning || err.message;
                return;
            }
            throw err;
        }
    }

    $effect(() => {
        const key = `${sessionId}:${contentGeneration}`;
        if (key !== observedIdentityKey) {
            observedIdentityKey = key;
            requestSequence += 1;
            loading = false;
            error = null;
            signatures = [];
            verifiedKey = "";
        }
        if (
            isOpen &&
            sessionId &&
            verificationPresentation.runnable &&
            verifiedKey !== key &&
            !loading
        ) {
            void verifySignatures(sessionId, contentGeneration);
        }
    });
</script>

{#if isOpen}
    <div class="signature-panel">
        <div class="panel-header">
            <h3>Digital Signatures</h3>
            <button class="close-btn" onclick={onclose}>×</button>
        </div>

        <div class="panel-content">
            <FeatureNotice featureId="signature_verification" />
            <FeatureNotice featureId="ltv" compact />
            {#if loading}
                <div class="loading">İmza verisi inceleniyor...</div>
            {:else if error}
                <div class="error">Error: {error}</div>
            {:else if signatures.length === 0}
                <div class="no-signatures">Kriptografik imza doğrulaması bu sürümde kullanılamıyor.</div>
            {:else}
                <ul class="signature-list">
                    {#each signatures as sig (sig.id)}
                        <li class="signature-item {sig.is_valid ? 'valid' : 'invalid'}">
                            <div class="sig-header">
                                <span class="icon">
                                    {#if sig.is_valid}
                                        ✅
                                    {:else}
                                        ❌
                                    {/if}
                                </span>
                                <strong>{sig.subject}</strong>
                            </div>
                            
                            <div class="sig-details">
                                <p>Backend-reported status (not cryptographically verified): {sig.is_valid ? 'valid' : 'invalid'}</p>
                                {#if sig.signing_time}
                                    <p>Date: {new Date(sig.signing_time).toLocaleString()}</p>
                                {/if}
                                <p>Timestamp: {sig.has_timestamp ? 'Yes (RFC 3161)' : 'No'}</p>
                                <p>Backend-reported LTV flag (not verified): {sig.ltv_enabled ? 'present' : 'absent'}</p>
                                
                                {#if !sig.is_valid}
                                    <ul class="errors">
                                        {#if sig.is_document_modified}<li>Document has been modified.</li>{/if}
                                        {#if !sig.is_certificate_trusted}<li>Certificate is not trusted.</li>{/if}
                                        {#if sig.is_certificate_expired}<li>Certificate expired.</li>{/if}
                                    {#each sig.errors as err, index (`${index}:${err}`)}
                                            <li>{err}</li>
                                        {/each}
                                    </ul>
                                {/if}

                                {#if sig.is_valid && !sig.ltv_enabled}
                                    <button class="btn-ltv" onclick={() => enableLTV(sig.id)} disabled={!ltvPresentation.runnable}>LTV unavailable · MOCK</button>
                                {/if}
                            </div>
                        </li>
                    {/each}
                </ul>
            {/if}
        </div>
    </div>
{/if}

<style>
    .signature-panel {
        position: absolute;
        right: 0;
        top: 0;
        bottom: 0;
        width: 320px;
        background: var(--surface-color, #1e1e1e);
        border-left: 1px solid var(--border-color, #333);
        display: flex;
        flex-direction: column;
        color: var(--text-color, #eee);
        z-index: 100;
        box-shadow: -2px 0 10px rgba(0,0,0,0.5);
    }

    .panel-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        padding: 16px;
        border-bottom: 1px solid var(--border-color, #333);
    }

    .panel-header h3 {
        margin: 0;
        font-size: 1.1em;
    }

    .close-btn {
        background: none;
        border: none;
        color: inherit;
        font-size: 1.5em;
        cursor: pointer;
    }

    .panel-content {
        padding: 16px;
        overflow-y: auto;
        flex: 1;
    }

    .signature-list {
        list-style: none;
        padding: 0;
        margin: 0;
    }

    .signature-item {
        background: var(--surface-light, #2a2a2a);
        border-radius: 8px;
        padding: 12px;
        margin-bottom: 12px;
        border-left: 4px solid #666;
    }

    .signature-item.valid {
        border-left-color: #4caf50;
    }

    .signature-item.invalid {
        border-left-color: #f44336;
    }

    .sig-header {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-bottom: 8px;
    }

    .sig-details {
        font-size: 0.9em;
        color: #aaa;
    }
    
    .sig-details p {
        margin: 4px 0;
    }

    .errors {
        color: #f44336;
        margin-top: 8px;
        padding-left: 16px;
    }

    .btn-ltv {
        margin-top: 8px;
        background: #2196f3;
        color: white;
        border: none;
        padding: 6px 12px;
        border-radius: 4px;
        cursor: pointer;
    }
    .btn-ltv:hover {
        background: #1976d2;
    }
</style>
