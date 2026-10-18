// Store-page Markdown, rendered with raw HTML switched off (docs/SECURITY.md §8.1: Markdown with
// a strict allowlist, no raw HTML). markdown-it escapes any HTML in the source and refuses
// javascript:, vbscript:, file: and most data: links.

import MarkdownIt from "markdown-it";

const md = new MarkdownIt({ html: false, linkify: true, typographer: false });

// Headings inside a store page start at h3, below the page's own headings.
md.core.ruler.push("demote_headings", (state) => {
  for (const token of state.tokens) {
    if (token.type === "heading_open" || token.type === "heading_close") {
      const level = Math.min(6, Number(token.tag.slice(1)) + 2);
      token.tag = `h${level}`;
    }
  }
});

// Every link is external: mark it so the click handler sends it to the system browser.
const defaultLinkOpen = md.renderer.rules.link_open ?? ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  tokens[idx].attrSet("rel", "noopener noreferrer nofollow");
  tokens[idx].attrSet("target", "_blank");
  tokens[idx].attrSet("data-external", "");
  return defaultLinkOpen(tokens, idx, options, env, self);
};

// No remote images: they would let any maker track who views their page.
md.disable("image");

export function renderMarkdown(source: string): string {
  return md.render(source);
}

// Slop's own documents (e.g. public/publishing.md): normal headings with anchors, images allowed,
// raw HTML still off. Links that leave the page go to the system browser on desktop.
const docs = new MarkdownIt({ html: false, linkify: true, typographer: false });
const slugify = (text: string) =>
  text
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");
docs.core.ruler.push("heading_ids", (state) => {
  state.tokens.forEach((token, i) => {
    if (token.type === "heading_open") {
      const text = state.tokens[i + 1]?.content ?? "";
      token.attrSet("id", slugify(text));
    }
  });
});
const docsLinkOpen = docs.renderer.rules.link_open ?? ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
docs.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  const href = String(tokens[idx].attrGet("href") ?? "");
  if (/^https?:/.test(href)) {
    tokens[idx].attrSet("rel", "noopener noreferrer");
    tokens[idx].attrSet("target", "_blank");
    tokens[idx].attrSet("data-external", "");
  }
  return docsLinkOpen(tokens, idx, options, env, self);
};

export function renderDocument(source: string): string {
  return docs.render(source);
}
