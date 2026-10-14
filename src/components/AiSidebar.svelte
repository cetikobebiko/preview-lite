<script>
  import { onDestroy } from "svelte";
  import { fly } from "svelte/transition";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";
  import { aiController } from "../services/app/aiController.js";
  import { aiState } from "../stores/aiState.svelte.js";

  let { sessionId = "", contentGeneration = 0, onopenSettings, onclose } = $props();
  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("local_ai", featureContext);

  let messages = $state([]);
  let inputMessage = $state("");
  let chatContainer = $state(null);
  let activeHandle = null;

  onDestroy(() => {
    if (activeHandle) {
      void activeHandle.cancel();
      activeHandle = null;
    }
  });

  function scrollToBottom() {
    setTimeout(() => {
      if (chatContainer) chatContainer.scrollTop = chatContainer.scrollHeight;
    }, 50);
  }

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
    try {
      await aiController.buildSemanticIndex(sessionId, contentGeneration);
      messages = [
        ...messages,
        {
          role: "assistant",
          content: "Belge başarıyla okundu ve yapay zeka hafızasına eklendi. Bana belge hakkında sorular sorabilirsiniz!",
        },
      ];
    } catch (e) {
      alert("İndeksleme başarısız oldu: " + e + "\n\nAI endpoint yapılandırmanızı veya Ollama servisini kontrol edin.");
    }
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
    if (!inputMessage.trim() || aiState.isStreaming) return;

    const userMsg = inputMessage;
    inputMessage = "";
    messages = [...messages, { role: "user", content: userMsg }];
    scrollToBottom();

    let assistantMsgIndex = messages.length;
    messages = [...messages, { role: "assistant", content: "" }];

    try {
      const history = messages.slice(0, -1).filter((m) => m.role === "user");
      const handle = aiController.startQueryDocumentRag(
        sessionId,
        contentGeneration,
        userMsg,
        history,
        (token) => {
          messages[assistantMsgIndex].content += token;
          messages = [...messages];
          scrollToBottom();
        },
      );
      activeHandle = handle;
      await handle.done;
    } catch (e) {
      if (e?.code === "AI_REQUEST_CANCELLED") {
        messages[assistantMsgIndex].content += " [İptal Edildi]";
      } else {
        messages[assistantMsgIndex].content = "Hata oluştu: " + (e?.message || e);
      }
    } finally {
      activeHandle = null;
    }
  }

  function handleCancelRequest() {
    if (activeHandle) {
      void activeHandle.cancel();
      activeHandle = null;
    }
  }
</script>

<div class="ai-sidebar glass-panel" in:fly={{ x: 300, duration: 300 }}>
  <div class="header">
    <h3>Yapay Zeka (Ollama)</h3>
    <div class="header-actions">
      <button class="icon-btn" onclick={() => onopenSettings?.()} title="Ayarlar">⚙️</button>
      <button class="icon-btn" onclick={() => onclose?.()} title="Kapat">✕</button>
    </div>
  </div>

  <div class="chat-container" bind:this={chatContainer}>
    <FeatureNotice featureId="local_ai" />
    {#if messages.length === 0}
      <p class="privacy-warning">
        {aiState.modeLabel}: Belgeden çıkarılan metin parçaları ve embedding model adı ({aiState.embedModel}) yapılandırılan endpoint'e iletilir.
      </p>
    {:else}
      <p class="privacy-warning">
        {aiState.modeLabel}: Kullanıcı sorusu, getirilen belge bağlam parçaları, sınırlandırılmış sohbet geçmişi ve LLM model adı ({aiState.llmModel}) yapılandırılan endpoint'e iletilir; semantik erişim sırasında embedding modeli ({aiState.embedModel}) ile iletişim kurulabilir.
      </p>
    {/if}

    {#if messages.length === 0}
      <div class="empty-state">
        <div class="ai-icon">🧠</div>
        <h4>Belge ile Sohbet</h4>
        <p>Belgeyi indeksleyerek semantik arama ve özetleme özelliklerini başlatın.</p>
        <button
          class="tech-button primary"
          onclick={handleIndexDocument}
          disabled={aiState.isIndexing || !featurePresentation.runnable}
        >
          {aiState.isIndexing ? "İndeksleniyor..." : "Endpoint’e Göndererek İndeksle"}
        </button>
      </div>
    {/if}

    {#each messages as msg}
      <div class="message {msg.role}">
        <div class="msg-bubble">
          {msg.content}
        </div>
      </div>
    {/each}

    {#if aiState.isStreaming && messages[messages.length - 1]?.role === "user"}
      <div class="message assistant">
        <div class="msg-bubble thinking">Düşünüyor...</div>
      </div>
    {/if}
  </div>

  <div class="input-area">
    <input
      type="text"
      bind:value={inputMessage}
      onkeydown={(e) => e.key === "Enter" && handleSend()}
      placeholder="Belge hakkında soru sorun..."
      class="tech-input"
      disabled={aiState.isStreaming || aiState.isIndexing || !featurePresentation.runnable}
    />
    {#if aiState.isStreaming}
      <button class="tech-button cancel-btn" onclick={handleCancelRequest}>İptal Et</button>
    {:else}
      <button
        class="tech-button send-btn"
        onclick={handleSend}
        disabled={!inputMessage.trim() || !featurePresentation.runnable}
      >
        ↑
      </button>
    {/if}
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
    box-shadow: -5px 0 25px rgba(0, 0, 0, 0.5);
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  }

  .header h3 {
    margin: 0;
    font-size: 16px;
    color: var(--accent-industrial);
  }

  .header-actions {
    display: flex;
    gap: 8px;
  }
  .icon-btn {
    background: none;
    border: none;
    color: white;
    cursor: pointer;
    font-size: 16px;
    opacity: 0.6;
  }
  .icon-btn:hover {
    opacity: 1;
  }
  .privacy-warning {
    margin: 0;
    color: #fbbf24;
    font-size: 11px;
    line-height: 1.4;
  }

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
  .ai-icon {
    font-size: 48px;
  }

  .message {
    display: flex;
    flex-direction: column;
    max-width: 85%;
  }
  .message.user {
    align-self: flex-end;
  }
  .message.assistant {
    align-self: flex-start;
  }

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
  .thinking {
    font-style: italic;
    opacity: 0.6;
  }

  .input-area {
    padding: 16px;
    border-top: 1px solid rgba(255, 255, 255, 0.1);
    display: flex;
    gap: 8px;
  }
  .input-area input {
    flex: 1;
  }
  .send-btn {
    padding: 0 16px;
    font-weight: bold;
  }
</style>
