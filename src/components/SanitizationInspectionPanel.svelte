<script>
  let {
    state,
    runnable = false,
    onInspect = () => {},
    onCancel = () => {},
    onClose = () => {},
  } = $props();

  const facts = $derived(state?.inspection?.documentFacts ?? null);
  const controls = $derived(state?.inspection?.controls ?? []);
  const blockers = $derived(state?.inspection?.blockingFindings ?? []);
  const hasFindings = $derived(controls.some((control) => control.present));

  const factRows = $derived(facts ? [
    ["Şifreleme", facts.encrypted],
    ["Dijital imza", facts.signed],
    ["Incremental revision", facts.incrementalRevisionsDetected],
    ["XFA", facts.xfaDetected],
    ["Optional content", facts.optionalContentDetected],
    ["RichMedia", facts.richMediaDetected],
  ] : []);

  const supportLabel = (support) => ({
    supported_for_inspection: "İnceleme destekleniyor",
    experimental: "Deneysel inceleme",
    unsupported: "Bu kategori tam olarak incelenemiyor",
    not_applicable: "Kapsam dışı",
  })[support] ?? "Bilinmeyen destek durumu";

  const removalLabel = (support, present) => {
    if (present && ["unsupported", "unverified", "not_applicable"].includes(support)) {
      return "Tespit edildi — kaldırma desteği henüz yok";
    }
    return ({
      supported: "Private transform çekirdeğinde destekleniyor",
      supported_with_visual_change: "Görsel değişiklikle private transform desteği",
      supported_with_functional_change: "İşlevsel değişiklikle private transform desteği",
      candidate: "Gelecekteki sınırlı profil adayı",
      candidate_with_visual_change: "Görsel değişiklik içeren aday",
      candidate_with_functional_change: "İşlevsel değişiklik içeren aday",
      experimental: "Deneysel",
      unsupported: "Kaldırma desteği yok",
      unverified: "Kaldırma desteği doğrulanmadı",
      not_applicable: "Kapsam dışı",
    })[support] ?? "Bilinmeyen durum";
  };
</script>

<aside class="inspection-panel" aria-label="PDF sanitization ön incelemesi">
  <header>
    <div>
      <p class="eyebrow">READ-ONLY INSPECTION</p>
      <h2>Sanitization ön incelemesi</h2>
    </div>
    <button class="ghost" type="button" onclick={onClose}>Kapat</button>
  </header>

  <div class="scope-note">
    Bu ön inceleme yalnız desteklenen PDF yapılarını tarar. Belgenin bütün olası aktif
    veya gizli içeriğini kapsadığını garanti etmez. Inspection belgeyi değiştirmez.
  </div>

  {#if state?.inspectionStatus === "idle"}
    <p class="empty">Henüz inspection raporu yok.</p>
  {:else if state?.inspectionStatus === "inspecting"}
    <p class="status">Snapshot worker tarafından inceleniyor…</p>
  {:else if state?.inspectionStatus === "failed"}
    <div class="error" role="alert">{state.inspectionError?.message ?? "Inspection tamamlanamadı."}</div>
  {:else if state?.inspectionStatus === "cancelled"}
    <p class="status">Inspection iptal edildi.</p>
  {:else if state?.inspectionStatus === "review" && facts}
    {#if blockers.length > 0}
      <section class="blockers" aria-label="Minimum profil engelleri">
        <h3>Minimum profil engelleri</h3>
        {#each blockers as blocker (blocker.code + blocker.controlId)}
          <p><strong>{blocker.controlId}</strong> — {blocker.safeMessage}</p>
        {/each}
      </section>
    {/if}

    <section>
      <h3>Belge bilgileri</h3>
      <dl class="facts">
        <div><dt>Sayfa sayısı</dt><dd>{facts.pageCount}</dd></div>
        {#each factRows as fact (fact[0])}
          <div><dt>{fact[0]}</dt><dd class:present={fact[1]}>{fact[1] ? "Tespit edildi" : "Tespit edilmedi"}</dd></div>
        {/each}
      </dl>
    </section>

    <section>
      <h3>Kontroller</h3>
      {#if !hasFindings}
        <p class="empty">İncelenen destekli kategorilerde bulgu yok.</p>
      {/if}
      <div class="controls">
        {#each controls as control (control.controlId)}
          <article class:found={control.present} class:blocking={!!control.blockingReason}>
            <div class="control-title">
              <strong>{control.controlId}</strong>
              <span>{control.present ? `${control.findingCount} bulgu` : "Bulgu yok"}</span>
            </div>
            <p>{supportLabel(control.inspectionSupport)}</p>
            <p>{removalLabel(control.futureRemovalSupport, control.present)}</p>
            <p>Görsel etki: {control.expectedVisualImpact}</p>
            <p>İşlevsel etki: {control.expectedFunctionalImpact}</p>
            {#if control.blockingReason}<p class="blocking-reason">{control.blockingReason}</p>{/if}
          </article>
        {/each}
      </div>
    </section>
  {/if}

  <footer>
    {#if state?.inspectionStatus === "inspecting"}
      <button type="button" onclick={onCancel} disabled={!runnable}>İptal</button>
    {:else}
      <button type="button" onclick={onInspect} disabled={!runnable}>
        {state?.inspectionStatus === "review" ? "Yeniden incele" : "İncele"}
      </button>
    {/if}
    {#if !runnable}<span>Feature disabled</span>{/if}
  </footer>
</aside>

<style>
  .inspection-panel { width: min(440px, 94vw); height: 100%; overflow: auto; padding: 18px; color: var(--foreground); background: var(--card); border-left: 1px solid var(--border); }
  header, .control-title, footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  h2, h3, p { margin: 0; }
  h2 { font-size: 1.1rem; }
  h3 { margin: 18px 0 10px; font-size: .95rem; }
  .eyebrow { color: #60a5fa; font-size: .68rem; letter-spacing: .12em; }
  .scope-note, .error, .blockers, .status, .empty { margin-top: 14px; padding: 12px; border-radius: 8px; background: #23262c; line-height: 1.45; font-size: .84rem; }
  .error, .blockers { border: 1px solid #7f1d1d; background: #2b191b; }
  .facts { display: grid; grid-template-columns: 1fr 1fr; gap: 7px; margin: 0; }
  .facts div { padding: 9px; background: #23262c; border-radius: 7px; }
  dt { font-size: .72rem; color: #9ca3af; } dd { margin: 3px 0 0; font-size: .82rem; } dd.present { color: #fbbf24; }
  .controls { display: grid; gap: 8px; }
  article { padding: 11px; border: 1px solid #34373d; border-radius: 8px; }
  article.found { border-color: #9a6700; } article.blocking { border-color: #b91c1c; }
  article p { margin-top: 5px; font-size: .75rem; color: #b7bdc8; }
  .control-title span { font-size: .72rem; color: #9ca3af; }
  .blocking-reason { color: #fca5a5; }
  footer { margin-top: 18px; border-top: 1px solid #34373d; padding-top: 14px; }
  button { border: 1px solid #4b5563; border-radius: 7px; background: #2563eb; color: white; padding: 7px 11px; }
  button:disabled { opacity: .45; cursor: not-allowed; } button.ghost { background: transparent; }
  footer span { color: #9ca3af; font-size: .75rem; }
</style>
