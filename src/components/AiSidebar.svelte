<script>
  import { onDestroy } from "svelte";
  import ArrowUpIcon from "@lucide/svelte/icons/arrow-up";
  import SparklesIcon from "@lucide/svelte/icons/sparkles";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";
  import { aiController } from "../services/app/aiController.js";
  import { aiState } from "../stores/aiState.svelte.js";

  let { sessionId = "", contentGeneration = 0 } = $props();
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

<div class="ai-sidebar">
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
        <SparklesIcon class="size-8 text-muted-foreground" strokeWidth={1.5} />
        <h4>Belge ile Sohbet</h4>
        <p>Belgeyi indeksleyerek semantik arama ve özetleme özelliklerini başlatın.</p>
        <Button size="sm" onclick={handleIndexDocument} disabled={aiState.isIndexing || !featurePresentation.runnable}>
          {aiState.isIndexing ? "İndeksleniyor..." : "Endpoint’e Göndererek İndeksle"}
        </Button>
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
    <Input
      type="text"
      bind:value={inputMessage}
      onkeydown={(e) => e.key === "Enter" && handleSend()}
      placeholder="Belge hakkında soru sorun..."
      class="h-8 text-[13px]"
      disabled={aiState.isStreaming || aiState.isIndexing || !featurePresentation.runnable}
    />
    {#if aiState.isStreaming}
      <Button variant="outline" size="sm" onclick={handleCancelRequest}>İptal Et</Button>
    {:else}
      <Button size="icon-sm" aria-label="Gönder" onclick={handleSend} disabled={!inputMessage.trim() || !featurePresentation.runnable}>
        <ArrowUpIcon />
      </Button>
    {/if}
  </div>
</div>

<style>
  .ai-sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    color: var(--foreground);
  }

  .privacy-warning {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius);
    background: var(--warning);
    color: var(--warning-foreground);
    font-size: 11px;
    line-height: 1.4;
  }

  .chat-container {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    text-align: center;
    gap: 10px;
    padding: 16px 8px;
  }

  .empty-state h4 {
    font-size: 13px;
    font-weight: 600;
  }

  .empty-state p {
    color: var(--muted-foreground);
    font-size: 12px;
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
    padding: 8px 12px;
    border-radius: 16px;
    font-size: 13px;
    line-height: 1.4;
    white-space: pre-wrap;
  }
  .message.user .msg-bubble {
    background: var(--primary);
    color: var(--primary-foreground);
    border-bottom-right-radius: 4px;
  }
  .message.assistant .msg-bubble {
    background: var(--muted);
    color: var(--foreground);
    border-bottom-left-radius: 4px;
  }
  .thinking {
    font-style: italic;
    color: var(--muted-foreground);
  }

  .input-area {
    padding: 10px 12px;
    border-top: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
