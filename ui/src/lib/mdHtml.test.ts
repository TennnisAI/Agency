import { describe, expect, it } from "vitest";
import { isExternalHref, labelCodeLangs, wrapTables } from "./mdHtml";

describe("labelCodeLangs", () => {
  it("lifts the language onto the pre and keeps it on the code", () => {
    expect(labelCodeLangs('<pre><code class="language-rust">fn main() {}</code></pre>')).toBe(
      '<pre data-lang="rust"><code class="language-rust">fn main() {}</code></pre>',
    );
  });

  it("labels every block, not just the first", () => {
    const html = '<pre><code class="language-ts">a</code></pre><pre><code class="language-sh">b</code></pre>';
    expect(labelCodeLangs(html)).toContain('data-lang="ts"');
    expect(labelCodeLangs(html)).toContain('data-lang="sh"');
  });

  it("leaves an unlabelled block alone", () => {
    const html = "<pre><code>plain</code></pre>";
    expect(labelCodeLangs(html)).toBe(html);
  });

  it("passes through languages with the punctuation marked emits", () => {
    for (const lang of ["c++", "objective-c", "f#", "asp.net", "shell_session"]) {
      expect(labelCodeLangs(`<pre><code class="language-${lang}">x</code></pre>`))
        .toContain(`data-lang="${lang}"`);
    }
  });

  it("does not label a language carrying attribute-breaking characters", () => {
    const html = '<pre><code class="language-a&quot; onmouseover=&quot;x">y</code></pre>';
    expect(labelCodeLangs(html)).toBe(html);
  });
});

describe("isExternalHref", () => {
  it("accepts the schemes the browser should open", () => {
    for (const href of ["https://example.com", "http://example.com", "HTTPS://EXAMPLE.COM", "mailto:a@b.c"]) {
      expect(isExternalHref(href)).toBe(true);
    }
  });

  it("rejects everything else, script URLs included", () => {
    for (const href of ["javascript:alert(1)", "data:text/html,x", "file:///etc/passwd", "#anchor", "./rel.md"]) {
      expect(isExternalHref(href)).toBe(false);
    }
  });
});

describe("wrapTables", () => {
  it("puts a scroll wrap around a table", () => {
    expect(wrapTables("<table>\n<tr><td>a</td></tr>\n</table>")).toBe(
      '<div class="md-table-wrap"><table>\n<tr><td>a</td></tr>\n</table></div>',
    );
  });

  it("wraps a table that carries attributes", () => {
    expect(wrapTables('<table class="x">y</table>')).toBe(
      '<div class="md-table-wrap"><table class="x">y</table></div>',
    );
  });

  it("wraps every table, and balances each one", () => {
    const html = wrapTables("<table>a</table><p>x</p><table>b</table>");
    expect(html.match(/md-table-wrap/g)).toHaveLength(2);
    expect(html.match(/<\/div>/g)).toHaveLength(2);
  });

  it("leaves markup with no table alone", () => {
    const html = "<p>a <code>tabletop</code> b</p>";
    expect(wrapTables(html)).toBe(html);
  });
});
