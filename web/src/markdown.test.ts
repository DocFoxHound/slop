import { describe, expect, it } from "vitest";

import { renderMarkdown } from "./markdown";

describe("renderMarkdown", () => {
  it("escapes raw HTML", () => {
    const html = renderMarkdown('<script>alert(1)</script><img src=x onerror="alert(1)">');
    expect(html).not.toContain("<script");
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;script&gt;");
  });

  it("refuses script links", () => {
    for (const src of ["[x](javascript:alert(1))", "[x](JAVASCRIPT:alert(1))", "[x](vbscript:msgbox(1))", "[x](data:text/html,hi)"]) {
      expect(renderMarkdown(src)).not.toContain("<a");
    }
  });

  it("marks links external and safe", () => {
    const html = renderMarkdown("[code](https://github.com/a/b)");
    expect(html).toContain('href="https://github.com/a/b"');
    expect(html).toContain('rel="noopener noreferrer nofollow"');
    expect(html).toContain("data-external");
  });

  it("drops images, which could track viewers", () => {
    expect(renderMarkdown("![x](https://tracker.example/p.gif)")).not.toContain("<img");
  });

  it("starts headings at h3", () => {
    expect(renderMarkdown("# Title")).toContain("<h3>Title</h3>");
  });
});

describe("renderDocument", () => {
  it("gives headings anchors the guide's links use", async () => {
    const { renderDocument } = await import("./markdown");
    const html = renderDocument("## Slop Pulse (player counts)\n\n[jump](#slop-pulse-player-counts)\n\n<b>raw</b>");
    expect(html).toContain('<h2 id="slop-pulse-player-counts">');
    expect(html).toContain('href="#slop-pulse-player-counts"');
    expect(html).not.toContain("<b>raw</b>");
  });
});
