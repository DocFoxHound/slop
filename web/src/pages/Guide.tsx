// "How to publish": renders public/publishing.md, which is also served as-is for AI agents
// (see public/llms.txt).

import { useEffect, useRef, type MouseEvent } from "react";
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
      <p className="muted small">
        Also available as plain text for tools and AI agents: <a href="/publishing.md">/publishing.md</a>
      </p>
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
