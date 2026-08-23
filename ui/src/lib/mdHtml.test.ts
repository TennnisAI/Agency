import { describe, expect, it } from "vitest";
import { isExternalHref, labelCodeLangs } from "./mdHtml";

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
