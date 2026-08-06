<script>
    import { exportPdfXAction } from "../services/actions.js";
    import { save } from "@tauri-apps/plugin-dialog";
    import { createEventDispatcher } from "svelte";
    import FeatureNotice from "./FeatureNotice.svelte";
    import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
    import { getRuntimeFeatureContext } from "../featureRegistry.js";

    let {
        sessionId = "",
        contentGeneration = 0,
        isDocumentReady = false,
        onclose,
    } = $props();

    const dispatch = createEventDispatcher();
    const featureContext = getRuntimeFeatureContext();

    let standards = [
        { id: "PDF/X-1a:2001", label: "PDF/X-1a (CMYK, Şeffaflık Yok)" },
        { id: "PDF/X-4", label: "PDF/X-4 (Şeffaflık Destekli, Önerilen)" }
    ];
    let selectedStandard = $state("PDF/X-4");

    let profiles = [
        { id: "FOGRA39", label: "Coated FOGRA39 (ISO 12647-2:2004)" },
        { id: "FOGRA51", label: "PSO Coated v3 (FOGRA51)" },
        { id: "USWebCoatedSWOP", label: "US Web Coated (SWOP) v2" }
    ];
    let selectedProfile = $state("FOGRA39");

    let exporting = $state(false);
    let exportError = $state(null);

    let canExport = $derived(
        isDocumentReady &&
        typeof sessionId === "string" &&
        sessionId.trim().length > 0
    );

    async function handleExport() {
        if (!canExport) return;
        exporting = true;
        exportError = null;

        try {
            const requestSessionId = sessionId;
            const requestGeneration = contentGeneration;
            const targetPath = await save({
                filters: [{ name: "PDF Document", extensions: ["pdf"] }],
                defaultPath: `Prepress_${selectedStandard.replace(/[^a-zA-Z0-9]/g, '_')}.pdf`
            });

            if (targetPath) {
                await exportPdfXAction(
                    requestSessionId,
                    requestGeneration,
                    targetPath,
                    selectedStandard,
                    selectedProfile,
                    featureContext,
                );
            }
        } catch (e) {
            if (e instanceof FeatureUnavailableError) {
                exportError = e.presentation?.warning || e.message;
            } else {
                console.error("PDF/X Export Error:", e);
                exportError = e.toString();
            }
        } finally {
            exporting = false;
        }
    }

    function close() {
        if (typeof onclose === "function") {
            onclose();
        }
        dispatch("close");
    }
</script>

<div class="modal-backdrop" onclick={close} role="presentation">
    <div class="modal-content" onclick={(e) => e.stopPropagation()} role="presentation">
        <h2>PDF/X Dışa Aktarma Sihirbazı</h2>
        <p class="desc">PDF/X arayüz taslağı. Bu sürüm uyumlu çıktı üretmez.</p>
        <FeatureNotice featureId="pdf_x_export" />

        <div class="form-group">
            <label for="pdfx-standard-select">PDF/X Standardı</label>
            <select id="pdfx-standard-select" bind:value={selectedStandard}>
                {#each standards as std (std.id)}
                    <option value={std.id}>{std.label}</option>
                {/each}
            </select>
            {#if selectedStandard === "PDF/X-1a:2001"}
                <p class="warning">⚠️ PDF/X-1a şeffaflığı (transparency) desteklemez. Eğer belgenizde şeffaf nesneler varsa beklenmedik sonuçlar olabilir.</p>
            {/if}
        </div>

        <div class="form-group">
            <label for="pdfx-profile-select">Hedef ICC Profili (Output Intent)</label>
            <select id="pdfx-profile-select" bind:value={selectedProfile}>
                {#each profiles as prof (prof.id)}
                    <option value={prof.id}>{prof.label}</option>
                {/each}
            </select>
        </div>

        <div class="form-group checkbox">
            <label>
                <input type="checkbox" checked disabled />
                Font gömme seçeneği (uygulanmadı)
            </label>
        </div>

        {#if exportError}
            <div class="error-msg">{exportError}</div>
        {/if}
        
        <div class="actions">
            <button class="secondary" onclick={close} disabled={exporting}>İptal</button>
            <button class="primary" onclick={handleExport} disabled={exporting || !canExport}>
                {exporting ? "Dışa Aktarılıyor..." : "Dışa Aktar"}
            </button>
        </div>
    </div>
</div>

<style>
    .modal-backdrop {
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
        background: #1e1e1e;
        padding: 24px;
        border-radius: 12px;
        width: 480px;
        color: white;
        box-shadow: 0 10px 30px rgba(0,0,0,0.5);
        border: 1px solid #333;
    }
    h2 {
        margin-top: 0;
        font-size: 20px;
        margin-bottom: 8px;
    }
    .desc {
        color: #aaa;
        font-size: 14px;
        margin-bottom: 24px;
    }
    .form-group {
        margin-bottom: 20px;
    }
    label {
        display: block;
        margin-bottom: 8px;
        font-size: 14px;
        font-weight: 500;
    }
    select {
        width: 100%;
        padding: 10px;
        background: #2a2a2a;
        color: white;
        border: 1px solid #444;
        border-radius: 6px;
        font-size: 14px;
    }
    .checkbox label {
        display: flex;
        align-items: center;
        gap: 8px;
        font-weight: normal;
        color: #888;
    }
    .warning {
        color: #ff9f0a;
        font-size: 13px;
        margin-top: 8px;
        background: rgba(255, 159, 10, 0.1);
        padding: 8px;
        border-radius: 4px;
    }
    .error-msg {
        color: #ff453a;
        background: rgba(255, 69, 58, 0.1);
        padding: 10px;
        border-radius: 6px;
        margin-bottom: 16px;
        font-size: 14px;
    }
    .actions {
        display: flex;
        justify-content: flex-end;
        gap: 12px;
        margin-top: 30px;
    }
    button {
        padding: 10px 20px;
        border-radius: 6px;
        border: none;
        font-size: 14px;
        cursor: pointer;
        font-weight: 500;
    }
    button.secondary {
        background: #333;
        color: white;
    }
    button.primary {
        background: #007aff;
        color: white;
    }
    button:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }
</style>
