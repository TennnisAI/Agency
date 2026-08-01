// Docs search query grammar: `key:value` tokens filter on note frontmatter,
// everything else is free text passed to the title/body search. Values with
// spaces are quoted (`status:"in review"`); a trailing bare `key:` means "has
// this key". Colon tokens that are clearly prose stay literal text: URLs
// ("http://…") and a mid-query bare `word:` ("error: timeout") never become
// filters. Kept free of imports so both the index and the tree can use it.

export interface DocsQuery {
  filters: [string, string][];
  text: string;
}

const FILTER_RE = /(^|\s)([A-Za-z][A-Za-z0-9_-]*):("([^"]*)"|\S*)/g;

export function parseDocsQuery(query: string): DocsQuery {
  const filters: [string, string][] = [];
  const text = query
    .replace(FILTER_RE, (m, lead: string, key: string, value: string, quoted: string | undefined, offset: number) => {
      const v = quoted !== undefined ? quoted : value;
      // Literal prose, not filters: URLs ("http://…") and an empty-value
      // token mid-query ("error: timeout"). A trailing `key:` stays the
      // has-key filter — that's how one is typed.
      const atEnd = query.slice(offset + m.length).trim() === "";
      if (v.startsWith("//") || (v === "" && !atEnd)) return m;
      filters.push([key, v]);
      return lead;
    })
    .replace(/\s+/g, " ")
    .trim();
  return { filters, text };
}
