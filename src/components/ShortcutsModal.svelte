<script>
    import { createEventDispatcher, onMount } from "svelte";
    import { fade, fly } from "svelte/transition";
  
    const dispatch = createEventDispatcher();
  
    const shortcuts = [
      { key: "Ctrl + O", desc: "Dosya Aç" },
      { key: "Ctrl + S", desc: "Kaydet (Burn & Save)" },
      { key: "Ctrl + P", desc: "Yazdır" },
      { key: "Ctrl + F", desc: "Arama Çubuğunu Aç/Kapat" },
      { key: "Ctrl + G", desc: "Sayfaya Git" },
      { key: "Ok Tuşları", desc: "Sayfalar Arası Geçiş" },
      { key: "PageUp / PageDown", desc: "Sayfalar Arası Geçiş" },
      { key: "Ctrl + (+/-)", desc: "Yakınlaştır / Uzaklaştır" },
      { key: "Ctrl + 0", desc: "Yakınlaştırmayı Sıfırla" },
      { key: "Ctrl + / veya ?", desc: "Kısayol Menüsünü Aç/Kapat" }
    ];
  
    function handleKeyDown(e) {
      if (e.key === "Escape") dispatch("close");
    }
  </script>
  
  <svelte:window on:keydown={handleKeyDown} />
  
  <div class="modal-overlay" transition:fade={{ duration: 150 }} on:click={() => dispatch("close")}>
    <div class="modal-content glass-panel" transition:fly={{ y: 20, duration: 250 }} on:click|stopPropagation>
      <div class="modal-header">
        <h3>Klavye Kısayolları</h3>
        <button class="close-btn" on:click={() => dispatch("close")}>×</button>
      </div>
      
      <div class="modal-body">
        <div class="shortcuts-list">
          {#each shortcuts as sc}
            <div class="shortcut-item">
              <div class="shortcut-key"><kbd>{sc.key}</kbd></div>
              <div class="shortcut-desc">{sc.desc}</div>
            </div>
          {/each}
        </div>
      </div>
    </div>
  </div>
  
  <style>
    .modal-overlay {
      position: fixed; top: 0; left: 0; right: 0; bottom: 0;
      background: rgba(0, 0, 0, 0.7);
      backdrop-filter: blur(4px);
      display: flex; align-items: center; justify-content: center;
      z-index: 10000;
    }
    
    .modal-content {
      background: var(--bg-surface);
      border: 1px solid var(--border-color);
      border-radius: 12px;
      width: 500px;
      max-width: 90vw;
      box-shadow: 0 10px 40px rgba(0,0,0,0.6);
      overflow: hidden;
    }
    
    .modal-header {
      display: flex; justify-content: space-between; align-items: center;
      padding: 16px 20px; border-bottom: 1px solid var(--border-color);
      background: var(--bg-surface-hover);
    }
    
    .modal-header h3 { margin: 0; font-size: 1.1rem; color: var(--text-primary); }
    
    .close-btn {
      background: none; border: none; color: var(--text-secondary);
      font-size: 1.5rem; cursor: pointer; transition: color 0.2s;
    }
    .close-btn:hover { color: var(--text-primary); }
    
    .modal-body { padding: 20px; }
    
    .shortcuts-list {
      display: flex; flex-direction: column; gap: 12px;
    }
    
    .shortcut-item {
      display: flex; justify-content: space-between; align-items: center;
      padding-bottom: 8px; border-bottom: 1px solid rgba(255,255,255,0.05);
    }
    
    .shortcut-key kbd {
      background: rgba(255,255,255,0.1);
      border: 1px solid rgba(255,255,255,0.2);
      padding: 4px 10px;
      border-radius: 4px;
      font-family: monospace;
      font-size: 0.9rem;
      color: var(--accent-color);
    }
    
    .shortcut-desc {
      color: var(--text-secondary);
      font-size: 0.95rem;
    }
  </style>
