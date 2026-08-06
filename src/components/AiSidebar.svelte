<script>
  import { onMount, createEventDispatcher } from "svelte";
  import { listen } from "../services/platform/events.js";
  import { queryDocumentRag, buildSemanticIndex } from "../pdfState.js";
  import { fly } from "svelte/transition";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  export let sessionId = "";
  export let contentGeneration = 0;
  
  const dispatch = createEventDispatcher();
  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("local_ai", featureContext);

  let messages = []; // { role: "user"|"assistant", content: String }
  let inputMessage = "";
  let isThinking = false;
  let isIndexing = false;
  let hasIndexed = false;
  
  let unlistenToken = null;
  let unlistenDone = null;
  
  // Varsayılan Ayarlar
  let ollamaUrl = localStorage.getItem("ollamaUrl") || "http://127.0.0.1:11434";
  let llmModel = localStorage.getItem("llmModel") || "llama3";
  let embedModel = localStorage.getItem("embedModel") || "nomic-embed-text";

  onMount(() => {
    let destroyed = false;
    void listen("ai_chat_token", (event) => {
      // Gelen token'ı son mesaja (assistant) ekle
      if (messages.length > 0 && messages[messages.length - 1].role === "assistant") {
        messages[messages.length - 1].content += event.payload;
        messages = [...messages]; // Re-render
      } else {
        messages = [...messages, { role: "assistant", content: event.payload }];
      }
      scrollToBottom();
    }).then((dispose) => { if (destroyed) dispose(); else unlistenToken = dispose; })
      .catch((error) => console.warn("AI chat token listener kurulamadı:", error));
    void listen("ai_chat_done", () => {
      isThinking = false;
    }).then((dispose) => { if (destroyed) dispose(); else unlistenDone = dispose; })
      .catch((error) => console.warn("AI chat completion listener kurulamadı:", error));
    return () => {
      destroyed = true;
      unlistenToken?.();
      unlistenDone?.();
      unlistenToken = null;
      unlistenDone = null;
    };
  });

  async function handleIndexDocument() {
    try {
      return await guardFeatureAction("local_ai", featureContext, performIndexDocument);
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        alert(error.presentation?.warning || error.message);
        return;
      }
      throw error;
    }
  }

  async function performIndexDocument() {
    isIndexing = true;
    try {
      await buildSemanticIndex(sessionId, contentGeneration, 500, 50, ollamaUrl, embedModel);
      hasIndexed = true;
      messages = [...messages, { role: "assistant", content: "Belge başarıyla okundu ve yapay zeka hafızasına eklendi. Bana belge hakkında sorular sorabilirsiniz!" }];
    } catch(e) {
      alert("İndeksleme başarısız oldu: " + e + "\n\nOllama'nın çalışıp çalışmadığını kontrol edin.");
    }
    isIndexing = false;
  }

  async function handleSend() {
    try {
      return await guardFeatureAction("local_ai", featureContext, performSend);
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        alert(error.presentation?.warning || error.message);
        return;
      }
      throw error;
    }
  }

  async function performSend() {
    if (!inputMessage.trim() || isThinking) return;
    
    if (!hasIndexed) {
      await handleIndexDocument();
      if(!hasIndexed) return;
    }
    
    const userMsg = inputMessage;
    inputMessage = "";
    messages = [...messages, { role: "user", content: userMsg }];
    isThinking = true;
    scrollToBottom();
    
    try {
      // Sadece asistan olmayan eski mesajları history'e al (basitlik için)
      const history = messages.slice(0, -1).filter(m => m.role === "user");
      await queryDocumentRag(sessionId, contentGeneration, userMsg, history, ollamaUrl, llmModel, embedModel);
    } catch(e) {
      messages = [...messages, { role: "assistant", content: "Hata oluştu: " + e }];
      isThinking = false;
    }
  }

  let chatContainer;
  function scrollToBottom() {
    setTimeout(() => {
      if (chatContainer) chatContainer.scrollTop = chatContainer.scrollHeight;
    }, 50);
  }
  
</script>

<div class="ai-sidebar glass-panel" in:fly={{ x: 300, duration: 300 }}>
  <div class="header">
    <h3>Yapay Zeka (Ollama)</h3>
    <div class="header-actions">
      <button class="icon-btn" on:click={() => dispatch("openSettings")} title="Ayarlar">⚙️</button>
      <button class="icon-btn" on:click={() => dispatch("close")} title="Kapat">✕</button>
    </div>
  </div>
  
  <div class="chat-container" bind:this={chatContainer}>
    <FeatureNotice featureId="local_ai" />
    <p class="privacy-warning">Belge içeriği yapılandırılan AI endpoint’ine gönderilebilir. Yalnızca loopback endpoint kullanıldığında trafik cihaz içinde kalır.</p>
    {#if messages.length === 0}
      <div class="empty-state">
        <div class="ai-icon">🧠</div>
        <h4>Belge ile Sohbet</h4>
        <p>Belgeyi indeksleyerek semantik arama ve özetleme özelliklerini başlatın.</p>
        <button class="tech-button primary" on:click={handleIndexDocument} disabled={isIndexing || !featurePresentation.runnable}>
          {isIndexing ? 'İndeksleniyor...' : 'Endpoint’e Göndererek İndeksle'}
        </button>
      </div>
    {/if}
    
    {#each messages as msg}
      <div class="message {msg.role}">
        <div class="msg-bubble">
          <!-- Çok basit markdown tarzı linkleri (--- Parça X ---) vs düzeltmek için Svelte HTML render edilebilir ama basit tutuyoruz -->
          {msg.content}
        </div>
      </div>
    {/each}
    
    {#if isThinking && messages[messages.length-1]?.role === "user"}
      <div class="message assistant">
        <div class="msg-bubble thinking">Düşünüyor...</div>
      </div>
    {/if}
  </div>
  
  <div class="input-area">
    <input 
      type="text" 
      bind:value={inputMessage} 
      on:keydown={(e) => e.key === 'Enter' && handleSend()}
      placeholder="Belge hakkında soru sorun..." 
      class="tech-input"
      disabled={isThinking || isIndexing || !featurePresentation.runnable}
    />
    <button class="tech-button send-btn" on:click={handleSend} disabled={isThinking || !inputMessage.trim() || !featurePresentation.runnable}>↑</button>
  </div>
</div>

<style>
  .ai-sidebar {
    position: absolute;
    top: 60px;
    right: 16px;
    bottom: 16px;
    width: 380px;
    background: rgba(15, 20, 25, 0.85);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    z-index: 1000;
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    box-shadow: -5px 0 25px rgba(0,0,0,0.5);
  }
  
  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  }
  
  .header h3 { margin: 0; font-size: 16px; color: var(--accent-industrial); }
  
  .header-actions { display: flex; gap: 8px; }
  .icon-btn { background: none; border: none; color: white; cursor: pointer; font-size: 16px; opacity: 0.6; }
  .icon-btn:hover { opacity: 1; }
  .privacy-warning { margin: 0; color: #fbbf24; font-size: 11px; line-height: 1.4; }
  
  .chat-container {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
    opacity: 0.7;
    gap: 16px;
  }
  .ai-icon { font-size: 48px; }
  
  .message {
    display: flex;
    flex-direction: column;
    max-width: 85%;
  }
  .message.user { align-self: flex-end; }
  .message.assistant { align-self: flex-start; }
  
  .msg-bubble {
    padding: 10px 14px;
    border-radius: 16px;
    font-size: 14px;
    line-height: 1.4;
    white-space: pre-wrap;
  }
  .message.user .msg-bubble {
    background: var(--accent-industrial);
    color: var(--bg-dark);
    border-bottom-right-radius: 4px;
  }
  .message.assistant .msg-bubble {
    background: rgba(255, 255, 255, 0.1);
    color: white;
    border-bottom-left-radius: 4px;
  }
  .thinking { font-style: italic; opacity: 0.6; }
  
  .input-area {
    padding: 16px;
    border-top: 1px solid rgba(255, 255, 255, 0.1);
    display: flex;
    gap: 8px;
  }
  .input-area input { flex: 1; }
  .send-btn { padding: 0 16px; font-weight: bold; }
</style>
