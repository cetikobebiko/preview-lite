<script>
    import {
        analyzeSessionColorSpaces,
        calculateSessionInkCoverage,
        renderSessionSeparationChannels,
    } from "../services/backend/prepress.js";
    import { createEventDispatcher, onDestroy } from "svelte";
    import FeatureNotice from "./FeatureNotice.svelte";

    const dispatch = createEventDispatcher();

    let {
        scale = 1.5,
        sessionId = "",
        pageIndex = -1,
        contentGeneration = 0,
        isDocumentReady = false,
        ...rest
    } = $props();

    let colorReport = $state(null);
    let loadingReport = $state(false);
    let reportError = $state(null);

    let channels = $state({ c: true, m: true, y: true, k: true });
    let separationImages = $state(null);
    let loadingSeparations = $state(false);
    let separationsError = $state(null);

    let ticLimit = $state(300);
    let heatmapBase64 = $state(null);
    let loadingHeatmap = $state(false);
    let heatmapError = $state(null);

    let boxes = $state({
        bleed: 3.0,
        trim: 0.0,
    });

    let reportRequestId = $state(0);
    let separationsRequestId = $state(0);
    let heatmapRequestId = $state(0);
    let observedSessionId = $state();
    let observedPageIndex = $state();
    let observedContentGeneration = $state();
    let observedDocumentReady = $state();
    let isDestroyed = $state(false);

    let canUseDocument = $derived(
        isDocumentReady &&
        typeof sessionId === "string" &&
        sessionId.length > 0
    );
    let canUsePage = $derived(
        canUseDocument &&
        Number.isInteger(pageIndex) &&
        pageIndex >= 0
    );

    $effect(() => {
        const documentContextChanged =
            sessionId !== observedSessionId ||
            contentGeneration !== observedContentGeneration ||
            isDocumentReady !== observedDocumentReady;
        const pageContextChanged = pageIndex !== observedPageIndex;

        observedSessionId = sessionId;
        observedPageIndex = pageIndex;
        observedContentGeneration = contentGeneration;
        observedDocumentReady = isDocumentReady;

        if (documentContextChanged) {
            resetDocumentOperationState();
        } else if (pageContextChanged) {
            resetPageOperationState();
        }
    });

    function resetDocumentOperationState() {
        reportRequestId += 1;
        loadingReport = false;
        reportError = null;
        colorReport = null;
        resetPageOperationState();
    }

    function resetPageOperationState() {
        separationsRequestId += 1;
        heatmapRequestId += 1;
        loadingSeparations = false;
        loadingHeatmap = false;
        separationsError = null;
        heatmapError = null;
        separationImages = null;
        heatmapBase64 = null;
        channels = { c: true, m: true, y: true, k: true };
    }

    function getDocumentSnapshot() {
        if (!canUseDocument) return null;
        return {
            sessionId,
            contentGeneration,
        };
    }

    function getPageSnapshot() {
        if (!canUsePage) return null;
        return {
            sessionId,
            pageIndex,
            contentGeneration,
        };
    }

    function isCurrentDocumentRequest(requestId, snapshot) {
        return Boolean(
            !isDestroyed &&
            requestId === reportRequestId &&
            canUseDocument &&
            sessionId === snapshot.sessionId &&
            contentGeneration === snapshot.contentGeneration
        );
    }

    function isCurrentPageRequest(requestId, activeRequestId, snapshot) {
        return Boolean(
            !isDestroyed &&
            requestId === activeRequestId &&
            canUsePage &&
            sessionId === snapshot.sessionId &&
            pageIndex === snapshot.pageIndex &&
            contentGeneration === snapshot.contentGeneration
        );
    }

    function errorMessage(error) {
        return error instanceof Error ? error.message : String(error);
    }

    async function analyzeColors() {
        const snapshot = getDocumentSnapshot();
        if (!snapshot) return;

        const requestId = ++reportRequestId;
        loadingReport = true;
        reportError = null;
        try {
            const result = await analyzeSessionColorSpaces(
                snapshot.sessionId,
                snapshot.contentGeneration,
            );
            if (!isCurrentDocumentRequest(requestId, snapshot)) return;
            colorReport = result;
        } catch (e) {
            if (isCurrentDocumentRequest(requestId, snapshot)) {
                reportError = errorMessage(e);
                console.error("Color analysis failed:", e);
            }
        } finally {
            if (isCurrentDocumentRequest(requestId, snapshot)) {
                loadingReport = false;
            }
        }
    }

    async function generateSeparations() {
        const snapshot = getPageSnapshot();
        if (!snapshot) return;

        const requestId = ++separationsRequestId;
        loadingSeparations = true;
        separationsError = null;
        try {
            const result = await renderSessionSeparationChannels(
                snapshot.sessionId,
                snapshot.contentGeneration,
                snapshot.pageIndex,
                scale,
            );
            if (!isCurrentPageRequest(requestId, separationsRequestId, snapshot)) return;

            separationImages = result;
            const payload = {
                images: result,
                channels,
                ...snapshot,
            };
            dispatch("separationsReady", payload);
        } catch (e) {
            if (isCurrentPageRequest(requestId, separationsRequestId, snapshot)) {
                separationsError = errorMessage(e);
                console.error("Separations failed:", e);
            }
        } finally {
            if (isCurrentPageRequest(requestId, separationsRequestId, snapshot)) {
                loadingSeparations = false;
            }
        }
    }

    function toggleChannel(c) {
        const snapshot = getPageSnapshot();
        if (!snapshot) return;

        channels = { ...channels, [c]: !channels[c] };
        dispatch("channelsChanged", { channels, ...snapshot });
    }

    async function analyzeTic() {
        const snapshot = getPageSnapshot();
        if (!snapshot) return;

        const requestId = ++heatmapRequestId;
        loadingHeatmap = true;
        heatmapError = null;
        try {
            const result = await calculateSessionInkCoverage(
                snapshot.sessionId,
                snapshot.contentGeneration,
                snapshot.pageIndex,
                ticLimit,
                scale,
            );
            if (!isCurrentPageRequest(requestId, heatmapRequestId, snapshot)) return;

            heatmapBase64 = result.heatmap_png_base64;
            dispatch("heatmapReady", {
                heatmap: heatmapBase64,
                ...snapshot,
            });
        } catch (e) {
            if (isCurrentPageRequest(requestId, heatmapRequestId, snapshot)) {
                heatmapError = errorMessage(e);
                console.error("TIC calculation failed:", e);
            }
        } finally {
            if (isCurrentPageRequest(requestId, heatmapRequestId, snapshot)) {
                loadingHeatmap = false;
            }
        }
    }

    function clearHeatmap() {
        const snapshot = getPageSnapshot();
        heatmapBase64 = null;
        if (snapshot) {
            dispatch("heatmapReady", { heatmap: null, ...snapshot });
        }
    }

    function updateGuides() {
        const snapshot = getPageSnapshot();
        if (snapshot) {
            dispatch("guidesChanged", { boxes, ...snapshot });
        }
    }

    function applyBoxes() {
        const snapshot = getPageSnapshot();
        if (!snapshot) return;
        // BleedBox is typically larger than TrimBox. Assuming points or mm?
        // Let's assume mm and 1 mm = 2.83465 points
        const mmToPt = 2.83465;
        // In a real app we'd get the actual media box and calculate the coords.
        // For now, dispatch an event that Export or another logic will use to apply.
        dispatch("applyBoxes", { boxes, ...snapshot });
    }

    onDestroy(() => {
        isDestroyed = true;
        reportRequestId += 1;
        separationsRequestId += 1;
        heatmapRequestId += 1;
    });

</script>

<div class="prepress-panel">
    <h3>Prepress & Matbaa</h3>
    <FeatureNotice featureId="prepress_analysis" />
    
    <div class="section">
        <h4>Renk Profili Analizi</h4>
        <button onclick={analyzeColors} disabled={loadingReport || !canUseDocument}>
            {loadingReport ? "Analiz Ediliyor..." : "Belgeyi Analiz Et"}
        </button>
        {#if reportError}
            <p class="error-msg">{reportError}</p>
        {/if}
        {#if colorReport}
            <div class="report-box">
                <p><strong>RGB Objeler:</strong> {colorReport.rgb_count}</p>
                <p><strong>CMYK Objeler:</strong> {colorReport.cmyk_count}</p>
                <p><strong>Gri/Spot:</strong> {colorReport.grayscale_count} / {colorReport.spot_count}</p>
                <p class="warning" class:hidden={colorReport.rgb_count === 0}>⚠️ Tahmini analiz RGB nesneler bildirdi. Üretim kararından önce uzman araçla doğrulayın.</p>
            </div>
        {/if}
    </div>

    <div class="section">
        <h4>Renk Ayrımları (Separation)</h4>
        <button onclick={generateSeparations} disabled={loadingSeparations || !canUsePage}>
            {loadingSeparations ? "Hazırlanıyor..." : "Kanalları Çıkar"}
        </button>
        {#if separationsError}
            <p class="error-msg">{separationsError}</p>
        {/if}
        
        {#if separationImages}
            <div class="channel-toggles">
                <label class="channel c" class:active={channels.c}>
                    <input type="checkbox" checked={channels.c} onchange={() => toggleChannel('c')} /> C
                </label>
                <label class="channel m" class:active={channels.m}>
                    <input type="checkbox" checked={channels.m} onchange={() => toggleChannel('m')} /> M
                </label>
                <label class="channel y" class:active={channels.y}>
                    <input type="checkbox" checked={channels.y} onchange={() => toggleChannel('y')} /> Y
                </label>
                <label class="channel k" class:active={channels.k}>
                    <input type="checkbox" checked={channels.k} onchange={() => toggleChannel('k')} /> K
                </label>
            </div>
            <p class="tip">Önizleme üzerinde aktif kanallar (Multiply modu ile) gösterilmektedir.</p>
        {/if}
    </div>

    <div class="section">
        <h4>Mürekkep Yoğunluğu (TIC)</h4>
        <div class="input-group">
            <label for="prepress-tic-limit">Limit (%):</label>
            <input id="prepress-tic-limit" type="number" bind:value={ticLimit} min="100" max="400" step="10" />
        </div>
        <div class="btn-group">
            <button onclick={analyzeTic} disabled={loadingHeatmap || !canUsePage}>
                {loadingHeatmap ? "Hesaplanıyor..." : "Isı Haritası Çıkar"}
            </button>
            {#if heatmapBase64}
                <button class="secondary" onclick={clearHeatmap}>Temizle</button>
            {/if}
        </div>
        {#if heatmapError}
            <p class="error-msg">{heatmapError}</p>
        {/if}
    </div>

    <div class="section">
        <h4>Sayfa Kutuları (mm)</h4>
        <div class="input-group">
            <label for="prepress-bleed">Taşırma Payı (Bleed):</label>
            <input id="prepress-bleed" type="number" bind:value={boxes.bleed} min="0" step="1" oninput={updateGuides} />
        </div>
        <div class="input-group">
            <label for="prepress-trim">Kesim Payı (Trim):</label>
            <input id="prepress-trim" type="number" bind:value={boxes.trim} min="0" step="1" oninput={updateGuides} />
        </div>
        <button onclick={applyBoxes} disabled={!canUsePage}>Önizleme Kılavuzlarını Güncelle</button>
    </div>
</div>

<style>
    .prepress-panel {
        padding: 16px;
        background: #1e1e1e;
        color: #ddd;
        font-size: 14px;
        height: 100%;
        overflow-y: auto;
    }
    
    h3 {
        margin-top: 0;
        margin-bottom: 20px;
        color: #fff;
        border-bottom: 1px solid #333;
        padding-bottom: 8px;
    }
    
    .section {
        margin-bottom: 24px;
        background: #2a2a2a;
        padding: 12px;
        border-radius: 6px;
    }
    
    h4 {
        margin-top: 0;
        margin-bottom: 12px;
        font-size: 13px;
        color: #aaa;
        text-transform: uppercase;
        letter-spacing: 0.5px;
    }
    
    button {
        width: 100%;
        padding: 8px;
        background: #007aff;
        color: white;
        border: none;
        border-radius: 4px;
        cursor: pointer;
        font-weight: 500;
        margin-bottom: 8px;
    }
    
    button:disabled {
        background: #444;
        color: #888;
        cursor: not-allowed;
    }
    
    button.secondary {
        background: #444;
    }
    
    .btn-group {
        display: flex;
        gap: 8px;
    }
    
    .report-box {
        margin-top: 12px;
        padding: 10px;
        background: #1a1a1a;
        border-radius: 4px;
        font-size: 13px;
    }
    
    .report-box p {
        margin: 4px 0;
    }
    
    .warning {
        color: #ff9f0a;
        margin-top: 8px !important;
    }

    .error-msg {
        color: #ff453a;
        margin: 8px 0;
        font-size: 12px;
    }
    
    .hidden {
        display: none;
    }
    
    .channel-toggles {
        display: flex;
        gap: 8px;
        margin-top: 12px;
    }
    
    .channel {
        flex: 1;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 6px;
        border-radius: 4px;
        cursor: pointer;
        background: #222;
        border: 1px solid #444;
        font-weight: bold;
    }
    
    .channel input {
        display: none;
    }
    
    .channel.active.c { background: rgba(0, 255, 255, 0.2); border-color: cyan; color: cyan; }
    .channel.active.m { background: rgba(255, 0, 255, 0.2); border-color: magenta; color: magenta; }
    .channel.active.y { background: rgba(255, 255, 0, 0.2); border-color: yellow; color: yellow; }
    .channel.active.k { background: rgba(0, 0, 0, 0.5); border-color: #888; color: white; }
    
    .input-group {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 12px;
    }
    
    .input-group input {
        width: 60px;
        padding: 4px 8px;
        background: #111;
        border: 1px solid #444;
        color: white;
        border-radius: 4px;
    }
    
    .tip {
        font-size: 12px;
        color: #888;
        margin-top: 8px;
        font-style: italic;
    }
</style>
