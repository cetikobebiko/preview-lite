<script>
  import { runOcrOnPage } from "../pdfState.js";
  import { getOcrLanguages, isPageScanned } from "../ocrDocumentState.js";
  import { fade } from "svelte/transition";

  let {
    sessionId = "",
    contentGeneration = 0,
    pageIndex = 0,
    pageWidth = 0,
    pageHeight = 0,
  } = $props();
  
  let isScanned = $state(false);
  let ocrData = $state(null);
  let isProcessing = $state(false);
  let availableLangs = $state(["eng", "tur"]);
  let selectedLang = $state("eng");
  let requestSequence = 0;

  function requestIsCurrent(
    sequence,
    requestSessionId,
    requestGeneration,
    requestPageIndex,
  ) {
    return (
      sequence === requestSequence &&
      requestSessionId === sessionId &&
      requestGeneration === contentGeneration &&
      requestPageIndex === pageIndex
    );
  }

  $effect(() => {
    const requestSessionId = sessionId;
    const requestGeneration = contentGeneration;
    const requestPageIndex = pageIndex;
    const sequence = ++requestSequence;
    ocrData = null;
    isScanned = false;
    isProcessing = false;
    if (!requestSessionId) return;
    void inspectPage(
      requestSessionId,
      requestGeneration,
      requestPageIndex,
      sequence,
    );
  });

  async function inspectPage(
    requestSessionId,
    requestGeneration,
    requestPageIndex,
    sequence,
  ) {
    try {
      // Tespit belge başına tek job, dil listesi uygulama düzeyinde cache'lenir;
      // sayfa mount/unmount döngüsü yeni job veya tesseract süreci başlatmaz.
      const [scanned, langs] = await Promise.all([
        isPageScanned(
          requestSessionId,
          requestGeneration,
          requestPageIndex,
        ),
        getOcrLanguages(),
      ]);
      if (!requestIsCurrent(
        sequence,
        requestSessionId,
        requestGeneration,
        requestPageIndex,
      )) return;
      isScanned = scanned;
      if (langs && langs.length > 0) {
        availableLangs = langs;
        if (langs.includes("tur")) selectedLang = "tur"; // Türkçe öncelikli
      }
    } catch(e) {
      if (requestIsCurrent(
        sequence,
        requestSessionId,
        requestGeneration,
        requestPageIndex,
      )) {
        console.warn("OCR kontrol hatası:", e);
      }
    }
  }

  async function handleRunOcr() {
    if (!sessionId) return;
    const requestSessionId = sessionId;
    const requestGeneration = contentGeneration;
    const requestPageIndex = pageIndex;
    const sequence = ++requestSequence;
    isProcessing = true;
    try {
      const result = await runOcrOnPage(
        requestSessionId,
        requestGeneration,
        requestPageIndex,
        selectedLang,
      );
      if (!requestIsCurrent(
        sequence,
        requestSessionId,
        requestGeneration,
        requestPageIndex,
      )) return;
      ocrData = result;
      isScanned = false; // OCR bitti, uyarıyı gizle
    } catch(e) {
      if (requestIsCurrent(
        sequence,
        requestSessionId,
        requestGeneration,
        requestPageIndex,
      )) {
        alert("OCR İşlemi Başarısız: " + e + "\n\n(Tesseract kurulu olmayabilir. Lütfen Tesseract'ı sisteminize kurun: brew install tesseract)");
      }
    } finally {
      if (requestIsCurrent(
        sequence,
        requestSessionId,
        requestGeneration,
        requestPageIndex,
      )) {
        isProcessing = false;
      }
    }
  }
</script>

<!-- Taranmış Sayfa Uyarısı -->
{#if isScanned && !ocrData}
  <div class="ocr-banner glass-panel" transition:fade>
    <span>Bu sayfa muhtemelen taranmış bir görsel. Metin seçimi için OCR çalıştırılsın mı?</span>
    <select bind:value={selectedLang} class="lang-select">
      {#each availableLangs as lang (lang)}
        <option value={lang}>{lang}</option>
      {/each}
    </select>
    <button class="tech-button primary" onclick={handleRunOcr} disabled={isProcessing}>
      {isProcessing ? 'Taranıyor...' : 'OCR Başlat'}
    </button>
  </div>
{/if}

<!-- OCR Metin Katmanı (Görünmez, Seçilebilir) -->
{#if ocrData}
  <div class="ocr-layer">
    {#each ocrData.words as word, index (`${index}:${word.x}:${word.y}:${word.text}`)}
      <!-- 
        OCR'ı 2.0 scale ile aldığımız için resim boyutları pageWidth * 2, pageHeight * 2. 
        Yüzdelik hesaplama: (x / (pageWidth * 2)) * 100
      -->
      <span 
        class="ocr-word"
        style="
          left: {(word.x / (pageWidth * 2)) * 100}%;
          top: {(word.y / (pageHeight * 2)) * 100}%;
          width: {(word.width / (pageWidth * 2)) * 100}%;
          height: {(word.height / (pageHeight * 2)) * 100}%;
        "
      >
        {word.text}
      </span>
    {/each}
  </div>
{/if}

<style>
  .ocr-banner {
    position: absolute;
    top: 16px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 100;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 24px;
    border-radius: 4px;
    background: var(--card);
    border: 1px solid var(--primary);
    box-shadow: 0 4px 12px rgba(0,0,0,0.5);
    font-size: 13px;
    color: var(--foreground);
    pointer-events: auto; /* Tıklanabilir olsun */
  }
  
  .lang-select {
    background: var(--secondary);
    border: 1px solid var(--border);
    color: var(--foreground);
    padding: 4px 8px;
    border-radius: 2px;
  }

  .tech-button {
    background: var(--secondary);
    border: 1px solid var(--border);
    color: var(--foreground);
    padding: 4px 12px;
    cursor: pointer;
    font-weight: 600;
    transition: all 0.1s;
  }

  .tech-button:hover:not(:disabled) {
    background: var(--primary);
    border-color: var(--primary);
    color: #000;
  }
  
  .tech-button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  
  .ocr-layer {
    position: absolute;
    top: 0; left: 0;
    width: 100%; height: 100%;
    pointer-events: auto; /* Kullanıcı metni seçebilsin */
  }
  
  .ocr-word {
    position: absolute;
    color: transparent;
    cursor: text;
    user-select: text;
    -webkit-user-select: text;
    font-size: 100%;
    line-height: 1;
    white-space: nowrap;
    overflow: hidden;
  }
  
  /* Seçildiğinde şeffaf mavi görünsün */
  .ocr-word::selection {
    background: rgba(0, 120, 255, 0.4);
    color: transparent;
  }
</style>
