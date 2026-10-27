<!-- Test double for PdfPage: records every value each per-page prop receives. -->
<script module>
  export const PROBED_PROPS = Object.freeze([
    "currentPage",
    "searchMatches",
    "sensitiveDataHighlights",
    "compareHighlights",
    "pendingRedactionRegions",
    "approximateCoveragePreview",
    "approximateProcessPreview",
  ]);
  // Every update since the last clear, and the current value per page and prop.
  export const probeLog = [];
  export const probeValues = new Map();
</script>

<script>
  import { untrack } from "svelte";

  let props = $props();

  for (const name of PROBED_PROPS) {
    $effect.pre(() => {
      const value = props[name];
      untrack(() => {
        probeLog.push({ pageIndex: props.pageIndex, name, value });
        probeValues.set(`${props.pageIndex}:${name}`, value);
      });
    });
  }
</script>
