<script>
  import { auditSessionAccessibility, getSessionStructureTree } from "../services/backend/session.js";
  import FeatureNotice from "./FeatureNotice.svelte";
  import TagTreeExplorer from "./TagTreeExplorer.svelte";

  let { sessionId = "", contentGeneration = 0, onclose } = $props();
  let activeTab = $state("summary");
  let isLoading = $state(false);
  let errorMsg = $state("");
  let report = $state(null);
  let inspection = $state(null);
  let selectedNode = $state(null);
  let requestSequence = 0;
  let mounted = true;

  const resultLabels = {
    pass: "No finding",
    fail: "Finding",
    warning: "Warning",
    not_applicable: "Not applicable",
    not_checked: "Not checked",
    indeterminate: "Indeterminate",
    human_review_required: "Human review",
  };
  const summaryLabels = {
    findings_detected: "Findings detected",
    no_findings_in_implemented_checks: "No findings in implemented checks",
    audit_incomplete: "Audit incomplete",
    audit_failed: "Audit failed",
  };

  $effect(() => {
    mounted = true;
    const requestedSession = sessionId;
    const requestedGeneration = contentGeneration;
    const sequence = ++requestSequence;
    report = null;
    inspection = null;
    selectedNode = null;
    errorMsg = "";
    if (!requestedSession) {
      isLoading = false;
      return () => { mounted = false; requestSequence += 1; };
    }
    void refresh(requestedSession, requestedGeneration, sequence);
    return () => { mounted = false; requestSequence += 1; };
  });

  async function refresh(requestedSession, requestedGeneration, sequence) {
    isLoading = true;
    const [auditResult, structureResult] = await Promise.allSettled([
      auditSessionAccessibility(requestedSession, requestedGeneration),
      getSessionStructureTree(requestedSession, requestedGeneration),
    ]);
    if (!mounted || sequence !== requestSequence || requestedSession !== sessionId || requestedGeneration !== contentGeneration) return;
    if (auditResult.status === "fulfilled") report = auditResult.value;
    else errorMsg = auditResult.reason?.toString?.() ?? String(auditResult.reason);
    if (structureResult.status === "fulfilled") inspection = structureResult.value;
    else if (!errorMsg) errorMsg = structureResult.reason?.toString?.() ?? String(structureResult.reason);
    isLoading = false;
  }

  function statusClass(status) {
    if (status === "findings_detected" || status === "audit_failed") return "text-red-300 border-red-500/30 bg-red-500/10";
    if (status === "audit_incomplete") return "text-yellow-300 border-yellow-500/30 bg-yellow-500/10";
    return "text-blue-300 border-blue-500/30 bg-blue-500/10";
  }
</script>

<div class="h-full flex flex-col bg-[#1e1e1e] text-white border-l border-white/10 w-80 shadow-2xl z-50">
  <div class="p-4 border-b border-white/10 flex justify-between items-center bg-[#252526]">
    <h2 class="text-lg font-semibold">Basic accessibility audit</h2>
    <button class="p-1 hover:bg-white/10 rounded" onclick={() => onclose?.()} aria-label="Close accessibility panel">✕</button>
  </div>

  <div class="flex border-b border-white/10 text-xs overflow-x-auto">
    {#each [["summary", "Summary"], ["checks", "Checks"], ["structure", "Structure"], ["review", "Review"], ["coverage", "Coverage"]] as tab}
      <button class="px-3 py-2 whitespace-nowrap {activeTab === tab[0] ? 'bg-blue-600 font-bold' : 'hover:bg-white/5'}" onclick={() => activeTab = tab[0]}>{tab[1]}</button>
    {/each}
  </div>

  <div class="flex-1 overflow-y-auto p-4 custom-scrollbar">
    <FeatureNotice featureId="basic_accessibility_audit" />
    {#if isLoading}<div class="text-center text-gray-400 py-4">Running bounded audit…</div>{/if}
    {#if errorMsg}<div class="bg-red-500/20 text-red-300 p-3 rounded mb-4 text-sm border border-red-500/30">Audit could not be completed: {errorMsg}</div>{/if}

    {#if report}
      {#if activeTab === "summary"}
        <div class="p-3 rounded border mb-4 {statusClass(report.summaryStatus)}">
          <div class="font-semibold">{summaryLabels[report.summaryStatus]}</div>
          <div class="text-xs mt-1">Reference: {report.referenceProfile}</div>
        </div>
        <p class="text-xs text-gray-300 mb-4">This basic audit covers only implemented machine checks. It does not establish PDF/UA conformance or document accessibility.</p>
        {#if report.summaryStatus === "no_findings_in_implemented_checks"}
          <p class="text-xs text-blue-200 mb-4">No issues were found in the implemented checks. Unchecked areas and items requiring human review remain.</p>
        {/if}
        <div class="grid grid-cols-2 gap-2 text-xs mb-4">
          <div class="p-2 bg-white/5 rounded">Findings: {report.checks.filter((item) => item.result === "fail").length}</div>
          <div class="p-2 bg-white/5 rounded">Warnings: {report.checks.filter((item) => item.result === "warning").length}</div>
          <div class="p-2 bg-white/5 rounded">Human review: {report.humanReview.length}</div>
          <div class="p-2 bg-white/5 rounded">Not checked: {report.coverage.notImplementedCheckIds.length}</div>
        </div>
        <dl class="grid grid-cols-2 gap-2 text-xs">
          <dt class="text-gray-400">PDF version</dt><dd>{report.documentFacts.effectiveVersion ?? "unknown"}</dd>
          <dt class="text-gray-400">Pages</dt><dd>{report.documentFacts.pageCount}</dd>
          <dt class="text-gray-400">Structure tree</dt><dd>{report.documentFacts.hasStructureTree ? "present" : "missing"}</dd>
          <dt class="text-gray-400">MarkInfo dictionary</dt><dd>{report.documentFacts.hasMarkInfo ? "present" : "missing"}</dd>
          <dt class="text-gray-400">Marked entry</dt><dd>{report.documentFacts.hasMarkedEntry ? "present" : "missing"}</dd>
          <dt class="text-gray-400">Marked value</dt><dd>{report.documentFacts.marked ? "true" : "not true"}</dd>
          <dt class="text-gray-400">XMP title</dt><dd>{report.documentFacts.xmpTitle}</dd>
        </dl>
        {#each report.warnings as warning}<div class="mt-2 p-2 text-xs border border-yellow-500/20 bg-yellow-500/5 text-yellow-200 rounded">{warning}</div>{/each}
      {:else if activeTab === "checks"}
        <div class="space-y-2">
          {#each report.checks as check (check.id)}
            <details class="p-3 bg-white/5 rounded border border-white/10 text-sm">
              <summary class="cursor-pointer"><span class="font-medium">{check.title}</span><span class="float-right text-xs text-gray-300">{resultLabels[check.result]}</span></summary>
              <div class="text-xs text-gray-400 mt-2">{check.id} · {check.automation} · {check.severity}</div>
              <div class="text-xs text-gray-400 mt-1">Evidence: {check.evidence.length}</div>
              {#each check.evidence as item}<div class="mt-2 text-xs border-l border-white/20 pl-2">{item.message}{#if item.valuePreview}: {item.valuePreview}{/if}</div>{/each}
              {#each check.notes as note}<div class="mt-2 text-xs text-yellow-200">{note}</div>{/each}
              {#each check.standardReferences as reference}<div class="mt-1 text-xs text-gray-500">{reference.standard} · {reference.clause}</div>{/each}
            </details>
          {/each}
        </div>
      {:else if activeTab === "structure"}
        {#if inspection?.root}
          <div class="text-xs text-yellow-200 mb-3">Read-only structural inspection; this is not a screen-reader preview or reading-order verification.</div>
          <div class="bg-[#181818] rounded border border-white/10 p-2 overflow-x-auto">
            <TagTreeExplorer node={inspection.root} isRoot onnodeSelected={(node) => selectedNode = node} />
          </div>
          {#if selectedNode}
            <div class="mt-3 bg-[#252526] p-3 rounded border border-white/10 text-xs">
              <div class="font-semibold">{selectedNode.rawType ?? selectedNode.kind}</div>
              <div class="text-gray-400 break-all">{selectedNode.id}</div>
              <div class="mt-2">Alt: {selectedNode.altTextState}{#if selectedNode.altTextPreview} — {selectedNode.altTextPreview}{/if}</div>
              <div>Language: {selectedNode.languageState}{#if selectedNode.languagePreview} — {selectedNode.languagePreview}{/if}</div>
              <div>Association: {selectedNode.associationStatus}</div>
            </div>
          {/if}
        {:else}<div class="text-center text-gray-400 mt-10">No structure tree is available.</div>{/if}
      {:else if activeTab === "review"}
        <p class="text-xs text-gray-300 mb-3">These items require a qualified human review.</p>
        {#each report.humanReview as item (item.id)}
          <div class="p-3 mb-2 bg-white/5 rounded border border-white/10 text-sm"><div>{item.title}</div><div class="text-xs text-gray-400">{item.category}</div></div>
        {/each}
      {:else if activeTab === "coverage"}
        <div class="text-sm mb-2">Implemented checks: {report.coverage.implementedCheckIds.length}</div>
        <div class="text-sm mb-2">Not implemented: {report.coverage.notImplementedCheckIds.length}</div>
        <div class="text-sm mb-4">Incomplete: {report.coverage.incompleteCheckIds.length}</div>
        <div class="text-xs text-gray-400 mb-4">Audit version: {report.auditVersion}</div>
        {#each report.coverage.categories as category}
          <div class="p-2 mb-2 text-xs bg-white/5 border border-white/10 rounded"><span class="font-medium">{category.category}</span> · {category.status}</div>
        {/each}
        {#each report.coverage.limitations as limitation}<div class="p-2 mb-2 text-xs bg-white/5 border border-white/10 rounded">{limitation}</div>{/each}
        {#if inspection?.truncated}<div class="p-2 text-xs bg-yellow-500/10 border border-yellow-500/30 text-yellow-200 rounded">Structure output was truncated by a safety budget.</div>{/if}
      {/if}
    {:else if !isLoading && !errorMsg}
      <div class="text-center text-gray-400 mt-10">Open a PDF to run the basic audit.</div>
    {/if}
  </div>
</div>

<style>
  .custom-scrollbar::-webkit-scrollbar { width: 6px; }
  .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(255, 255, 255, 0.2); border-radius: 3px; }
</style>
