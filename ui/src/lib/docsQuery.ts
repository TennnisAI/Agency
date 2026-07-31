// Docs search query grammar: `key:value` tokens filter on note frontmatter,
// everything else is free text passed to the title/body search. Values with
// spaces are quoted (`status:"in review"`); a bare `key:` means "has this
// key". Kept free of imports so both the index and the tree can use it.

export interface DocsQuery {
  filters: [string, string][];
  text: string;
}

const FILTER_RE = /(^|\s)([A-Za-z][A-Za-z0-9_-]*):("([^"]*)"|\S*)/g;

export function parseDocsQuery(query: string): DocsQuery {
  const filters: [string, string][] = [];
  const text = query
    .replace(FILTER_RE, (_m, lead: string, key: string, value: string, quoted?: string) => {
      filters.push([key, quoted !== undefined ? quoted : value]);
      return lead;
    })
    .replace(/\s+/g, " ")
    .trim();
  return { filters, text };
}
