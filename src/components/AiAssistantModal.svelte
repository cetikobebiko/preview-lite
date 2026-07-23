<script>
  import { createEventDispatcher } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  export let isOpen = false;
  export let documentContext = "";

  const dispatch = createEventDispatcher();

  let selectedModel = "llama3";
  let prompt = "";
  let isLoading = false;
  let chatHistory = [];

  function close() {
    dispatch("close");
  }

  async function handleSend(customPrompt = null) {
    const textToSend = customPrompt || prompt;
    if (!textToSend.trim()) return;

    chatHistory = [
      ...chatHistory,
      { sender: "user", text: textToSend }
    ];

    if (!customPrompt) {
      prompt = "";
    }

    isLoading = true;
    try {
      const aiResponse = await invoke("ask_local_ai", {
        model: selectedModel,
        prompt: textToSend,
        context: documentContext,
      });

      chatHistory = [
        ...chatHistory,
        { sender: "ai", text: aiResponse }
      ];
    } catch (err) {
      console.error("Local AI Hatası:", err);
      chatHistory = [
        ...chatHistory,
        { sender: "error", text: err.toString() }
      ];
    } finally {
      isLoading = false;
    }
  }

  function handleQuickAction(action) {
    if (action === "summarize") {
      handleSend("Lütfen bu belgedeki ana fikirleri ve önemli noktaları Türkçe olarak özetle.");
    } else if (action === "explain") {
      handleSend("Bu belgede geçen karmaşık terimleri ve kavramları basitleştirerek açıkla.");
    } else if (action === "actionables") {
      handleSend("Bu belgeden çıkarılabilecek önemli aksiyon maddelerini listele.");
    }
  }
</script>

{#if isOpen}
  <div class="ai-overlay" on:click={close} role="presentation" tabindex="-1">
    <div class="ai-dialog" on:click|stopPropagation role="presentation">
      <!-- AI Header -->
      <div class="ai-header">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-size: 20px;">🤖</span>
          <div>
            <h3 class="ai-title">Çevrimdışı Yapay Zeka (Local Ollama AI)</h3>
            <p class="ai-subtitle">%100 Gizli, Verileriniz Cihazınızda Kalır</p>
          </div>
        </div>

        <div style="display: flex; align-items: center; gap: 8px;">
          <select bind:value={selectedModel} class="model-select">
            <option value="llama3">llama3</option>
            <option value="mistral">mistral</option>
            <option value="qwen">qwen</option>
            <option value="gemma">gemma</option>
          </select>
          <button class="close-btn" on:click={close}>✕</button>
        </div>
      </div>

      <!-- Quick Action Pills -->
      <div class="quick-pills">
        <button class="pill-btn" on:click={() => handleQuickAction('summarize')}>
          📝 Belgeyi Özetle
        </button>
        <button class="pill-btn" on:click={() => handleQuickAction('explain')}>
          💡 Terimleri Açıkla
        </button>
        <button class="pill-btn" on:click={() => handleQuickAction('actionables')}>
          📌 Aksiyon Maddeleri
        </button>
      </div>

      <!-- Chat History Area -->
      <div class="chat-area">
        {#if chatHistory.length === 0}
          <div class="empty-state">
            <span style="font-size: 32px; margin-bottom: 8px;">💬</span>
            <p style="font-weight: 500; color: var(--text-primary);">Çevrimdışı Yapay Zekaya Soru Sorun</p>
            <p style="font-size: 12px; color: var(--text-muted); max-width: 320px; margin-top: 4px;">
              Belgeniz hakkında sorular sorun, metinleri özetletin veya aksiyon maddeleri çıkarın.
            </p>
          </div>
        {:else}
          {#each chatHistory as msg}
            <div class="chat-bubble {msg.sender}">
              <div class="bubble-header">
                {msg.sender === "user" ? "Siz" : msg.sender === "ai" ? "Local AI" : "Sistem Notu"}
              </div>
              <div class="bubble-content">{msg.text}</div>
            </div>
          {/each}
        {/if}

        {#if isLoading}
          <div class="chat-bubble ai loading">
            <div class="typing-indicator">
              <span></span><span></span><span></span>
            </div>
            <span style="font-size: 12px; color: var(--text-muted);">Yerel model yanıt üretiyor...</span>
          </div>
        {/if}
      </div>

      <!-- Chat Input Area -->
      <div class="input-area">
        <input
          type="text"
          class="chat-input"
          placeholder="Belge hakkında soru sorun... (Örn: Ana fikri ne?)"
          bind:value={prompt}
          on:keydown={(e) => e.key === "Enter" && handleSend()}
        />
        <button class="send-btn" on:click={() => handleSend()} disabled={isLoading}>
          Gönder
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .ai-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--bg-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    animation: fadeIn 0.15s ease-out;
  }

  .ai-dialog {
    width: 600px;
    height: 520px;
    max-width: 92vw;
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .ai-header {
    padding: 14px 18px;
    border-bottom: 1px solid var(--border-raw);
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--bg-panel);
  }

  .ai-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .ai-subtitle {
    font-size: 11px;
    color: var(--success-color);
    font-weight: 500;
  }

  .model-select {
    background: var(--bg-primary);
    border: 1px solid var(--border-raw);
    color: var(--text-primary);
    font-size: 12px;
    padding: 4px 8px;
    outline: none;
  }

  .close-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    font-size: 16px;
    cursor: pointer;
    padding: 4px 8px;
  }

  .quick-pills {
    display: flex;
    gap: 8px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border-raw);
    background: var(--bg-panel);
  }

  .pill-btn {
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    color: var(--text-primary);
    font-size: 11px;
    font-weight: 500;
    padding: 5px 10px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .pill-btn:hover {
    background: var(--bg-panel);
    border-color: var(--accent-industrial);
  }

  .chat-area {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .empty-state {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
  }

  .chat-bubble {
    display: flex;
    flex-direction: column;
    max-width: 85%;
    padding: 10px 14px;
    font-size: 13px;
    line-height: 1.5;
  }

  .chat-bubble.user {
    align-self: flex-end;
    background: var(--accent-industrial);
    color: var(--bg-primary);
  }

  .chat-bubble.ai {
    align-self: flex-start;
    background: var(--bg-panel);
    color: var(--text-primary);
    border: 1px solid var(--border-raw);
  }

  .chat-bubble.error {
    align-self: flex-start;
    background: var(--bg-panel);
    color: var(--error-color);
    border: 1px solid var(--error-color);
  }

  .bubble-header {
    font-size: 10px;
    font-weight: 700;
    opacity: 0.6;
    margin-bottom: 4px;
    text-transform: uppercase;
  }

  .input-area {
    padding: 12px 16px;
    border-top: 1px solid var(--border-raw);
    display: flex;
    gap: 8px;
    background: var(--bg-panel);
  }

  .chat-input {
    flex: 1;
    background: var(--bg-primary);
    border: 1px solid var(--border-raw);
    color: var(--text-primary);
    font-size: 13px;
    padding: 8px 12px;
    outline: none;
  }

  .chat-input:focus {
    border-color: var(--accent-industrial);
  }

  .send-btn {
    background: var(--accent-industrial);
    color: var(--bg-primary);
    border: none;
    padding: 0 16px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }

  .typing-indicator {
    display: flex;
    gap: 4px;
    margin-bottom: 4px;
  }

  .typing-indicator span {
    width: 6px;
    height: 6px;
    background: var(--text-muted);
    animation: bounce 1.2s infinite ease-in-out;
  }

  .typing-indicator span:nth-child(2) { animation-delay: 0.2s; }
  .typing-indicator span:nth-child(3) { animation-delay: 0.4s; }

  @keyframes bounce {
    0%, 80%, 100% { transform: scale(0); }
    40% { transform: scale(1.0); }
  }
</style>
