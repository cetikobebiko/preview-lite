// Background Text Search Web Worker
// Performs text searching off the main thread and streams search results page-by-page.

self.onmessage = function(e) {
  const { type, pagesText, query } = e.data;

  if (type === "search" && query) {
    const lowerQuery = query.toLowerCase();
    let totalMatches = 0;

    if (Array.isArray(pagesText)) {
      pagesText.forEach((pageItem) => {
        const pageIndex = pageItem.page_index;
        const pageText = pageItem.text || "";
        if (!pageText) return;

        const lowerText = pageText.toLowerCase();
        let matchIdx = lowerText.indexOf(lowerQuery);

        while (matchIdx !== -1) {
          totalMatches++;
          const snippetStart = Math.max(0, matchIdx - 30);
          const snippetEnd = Math.min(pageText.length, matchIdx + query.length + 30);
          const snippet = pageText.substring(snippetStart, snippetEnd);

          // Stream match result back to main thread
          self.postMessage({
            type: "match",
            match: {
              page_index: pageIndex,
              match_index: matchIdx,
              snippet: snippet,
              query_length: query.length,
            },
            totalMatches: totalMatches
          });

          matchIdx = lowerText.indexOf(lowerQuery, matchIdx + query.length);
        }
      });
    }

    self.postMessage({ type: "complete", totalMatches: totalMatches });
  }
};
