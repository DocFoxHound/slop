// "How to publish": renders public/publishing.md, which is also served as-is for AI agents
// (see public/llms.txt).

import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useLocation } from "react-router";

import { Loading } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { renderDocument } from "../markdown";
import { openExternal } from "../platform";

export function Guide() {
  useDocumentTitle("How to publish");
  const location = useLocation();
  const doc = useLoad(
    () =>
      fetch("/publishing.md").then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.text();
      }),
    [],
  );
  const body = useRef<HTMLDivElement>(null);

  // Jump to the section in the URL once the document is in place.
  useEffect(() => {
    if (!doc.data || !location.hash) return;
    document.getElementById(decodeURIComponent(location.hash.slice(1)))?.scrollIntoView();
  }, [doc.data, location.hash]);

  const onClick = (e: MouseEvent) => {
    const link = (e.target as HTMLElement).closest("a");
    if (!link) return;
    const href = link.getAttribute("href") ?? "";
    if (href.startsWith("#")) {
      e.preventDefault();
      document.getElementById(href.slice(1))?.scrollIntoView({ behavior: "smooth" });
      history.replaceState(null, "", href);
    } else if (link.hasAttribute("data-external")) {
      e.preventDefault();
      void openExternal(link.href);
    }
  };

  return (
    <div className="page narrow guide">
      <AiNotice />
      {doc.error && <p className="form-error">Couldn't load the guide.</p>}
      {!doc.data && !doc.error && <Loading />}
      {doc.data && (
        <article
          ref={body}
          className="prose doc"
          onClick={onClick}
          // Slop's own document, rendered with raw HTML off (markdown.ts).
          dangerouslySetInnerHTML={{ __html: renderDocument(doc.data) }}
        />
      )}
    </div>
  );
}

/** Points makers at the easy route: hand this page to their AI assistant. */
function AiNotice() {
  const prompt = `Read ${window.location.origin}/publishing.md. Tell me what's required and what's optional for publishing on Slop, then set up my project so it's ready to deploy there.`;
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard?.writeText(prompt).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  };
  return (
    <aside className="panel panel-pad ai-notice">
      <b>Using an AI coding assistant?</b>
      <p className="small">
        Tell it to read this page: it can work out what's required and what's optional, and set your project up for
        publishing on Slop. Paste this in:
      </p>
      <div className="ai-prompt">
        <code>{prompt}</code>
        <button type="button" className="btn small" onClick={copy}>
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <p className="muted small">
        The same guide as plain text, for tools and AI agents: <a href="/publishing.md">/publishing.md</a>
      </p>
    </aside>
  );
}
