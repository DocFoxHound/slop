// SPDX-License-Identifier: AGPL-3.0-or-later
// A creative's own page design (crates/slop-api/src/people/design.rs), drawn inside their People
// page. The server already rebuilt their HTML as a tree of allowed elements and their CSS from
// allowed rules; this draws the tree element by element (never as HTML) in a shadow root, so the
// design's CSS can't reach SLAPorSLOP's page and SLAPorSLOP's can't fight the design. The box contains its
// layout and paint: nothing in it can cover SLAPorSLOP's header or reach outside.
//
// SLAPorSLOP's own parts (<slop-works> and the rest) are SLAPorSLOP's components, slotted in from outside the
// shadow root: they keep SLAPorSLOP's behaviour and accessibility, and the design's CSS can't select
// them (though the design can still lay things over them, so nothing that matters for trust is a
// part: a creative's services and Hire buttons are always below the design, never inside it).

import { createElement, useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";

import type { CreativePage, DesignNode, PageDesign, PortfolioWork, Work } from "../api";
import { workKey } from "../api";
import { isDesktop, openExternal } from "../platform";
import { Lightbox, ServicesList, WorkTile } from "./People";

/** What draws each of SLAPorSLOP's parts. Returning null leaves the part out. */
export interface DesignParts {
  works: () => ReactNode;
  work: (key: string) => ReactNode;
  video: (youtubeId: string, title: string | null) => ReactNode;
  track: (src: string, title: string | null) => ReactNode;
}

// The server's allow-list again (design.rs), so nothing else is drawn even if a stored design
// somehow had it.
const TAGS = new Set([
  "section", "div", "header", "footer", "article", "aside", "h2", "h3", "h4", "h5", "h6", "p", "span", "strong", "em", "b",
  "i", "u", "s", "small", "mark", "code", "pre", "sub", "sup", "br", "hr", "ul", "ol", "li", "dl", "dt", "dd", "blockquote",
  "figure", "figcaption", "table", "caption", "thead", "tbody", "tfoot", "tr", "th", "td", "a", "img",
]);
const VOID = new Set(["br", "hr", "img"]);
export const LINK_REL = "nofollow noopener noreferrer ugc";

const srOnly: CSSProperties = {
  position: "absolute",
  width: 1,
  height: 1,
  overflow: "hidden",
  clip: "rect(0 0 0 0)",
  whiteSpace: "nowrap",
};

/** The design's content (for the shadow root) and SLAPorSLOP's parts (slotted in from outside). */
export interface BuiltDesign {
  content: ReactNode[];
  parts: Array<{ slot: string; node: ReactNode }>;
}

export function buildDesign(tree: DesignNode[], draw: DesignParts): BuiltDesign {
  const parts: BuiltDesign["parts"] = [];
  const openLink = (e: MouseEvent<HTMLAnchorElement>) => {
    if (!isDesktop) return;
    e.preventDefault();
    void openExternal(e.currentTarget.href);
  };
  const walk = (nodes: DesignNode[], at: string): ReactNode[] =>
    nodes.map((node, i) => {
      const key = `${at}${i}`;
      if (typeof node === "string") return node;
      const { tag, attrs = {}, children = [] } = node;
      if (tag.startsWith("slop-")) {
        const title = attrs.title ?? null;
        const part =
          tag === "slop-works"
            ? draw.works()
            : tag === "slop-work" && attrs.work
              ? draw.work(attrs.work)
              : tag === "slop-video" && attrs.youtube
                ? draw.video(attrs.youtube, title)
                : tag === "slop-track" && attrs.src && /^https?:\/\//.test(attrs.src)
                  ? draw.track(attrs.src, title)
                  : null;
        if (part === null || part === undefined) return null;
        const slot = `slop-part-${parts.length}`;
        parts.push({ slot, node: part });
        return <slot key={key} name={slot} />;
      }
      if (!TAGS.has(tag)) return <span key={key}>{walk(children, `${key}.`)}</span>;
      const props: Record<string, unknown> = { key };
      if (attrs.class) props.className = attrs.class;
      if (attrs.lang) props.lang = attrs.lang;
      let element = tag;
      const inside = VOID.has(tag) ? undefined : walk(children, `${key}.`);
      if (tag === "a") {
        if (attrs.href && /^(https:\/\/|mailto:)/i.test(attrs.href)) {
          props.href = attrs.href;
          props.target = "_blank";
          props.rel = LINK_REL;
          props.onClick = openLink;
          return createElement(
            "a",
            props,
            inside,
            <span key="new-tab" style={srOnly}>
              {" "}
              (opens in a new tab)
            </span>,
          );
        }
        element = "span";
      } else if (tag === "img") {
        if (!attrs.src || !/^https?:\/\//.test(attrs.src)) return null;
        props.src = attrs.src;
        props.alt = attrs.alt ?? "";
        props.loading = "lazy";
        props.decoding = "async";
        if (attrs.width) props.width = Number(attrs.width);
        if (attrs.height) props.height = Number(attrs.height);
      } else if (tag === "ol") {
        if (attrs.start) props.start = Number(attrs.start);
        if (attrs.reversed !== undefined) props.reversed = true;
      } else if (tag === "td" || tag === "th") {
        if (attrs.colspan) props.colSpan = Number(attrs.colspan);
        if (attrs.rowspan) props.rowSpan = Number(attrs.rowspan);
        if (tag === "th" && attrs.scope) props.scope = attrs.scope;
      }
      return createElement(element, props, inside);
    });
  return { content: walk(tree, ""), parts };
}

/**
 * Under the design's own CSS: readable defaults that the design can change. <slop-frame> (which
 * the design's CSS can't name) is the container its width queries ask about: the server turns
 * width-only media queries into container queries (design.rs).
 */
const BASE_CSS = `
slop-frame { display: block; container-type: inline-size; }
.page { display: flow-root; color: var(--text); font: 15px/1.55 system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif; overflow-wrap: anywhere; }
.page img { max-width: 100%; height: auto; }
.page a { color: var(--accent); }
.page :focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
`;

/** Over it: people who ask for less motion get none (the server drops !important from designs). */
const MOTION_CSS = `
@media (prefers-reduced-motion: reduce) {
  .page, .page *, .page *::before, .page *::after {
    animation-duration: 0.001ms !important; animation-delay: 0s !important; animation-iteration-count: 1 !important;
    transition-duration: 0.001ms !important; transition-delay: 0s !important; scroll-behavior: auto !important;
  }
}
`;

/** A design in its contained box: a labelled region of the page. */
export function DesignArea({ design, label, parts }: { design: PageDesign; label: string; parts: DesignParts }) {
  const host = useRef<HTMLElement>(null);
  const [root, setRoot] = useState<ShadowRoot | null>(null);
  useLayoutEffect(() => {
    const el = host.current;
    if (el) setRoot(el.shadowRoot ?? el.attachShadow({ mode: "open" }));
  }, []);
  const built = buildDesign(design.tree, parts);
  return (
    <section ref={host} className="person-design" aria-label={label}>
      {root &&
        built.parts.map((p) => (
          <div key={p.slot} slot={p.slot} className="design-part">
            {p.node}
          </div>
        ))}
      {root &&
        createPortal(
          <>
            <style>{BASE_CSS + design.css + MOTION_CSS}</style>
            {createElement("slop-frame", null, <div className="page">{built.content}</div>)}
          </>,
          root,
        )}
    </section>
  );
}

function videoWork(youtubeId: string, title: string | null): Work {
  return { key: `video:${youtubeId}`, kind: "video", url: null, width: null, height: null, youtube_id: youtubeId, audio_url: null, audio_format: null, title, caption: null, skill: null };
}

function trackWork(src: string, title: string | null): Work {
  return { key: `audio:${src}`, kind: "audio", url: null, width: null, height: null, youtube_id: null, audio_url: src, audio_format: null, title, caption: null, skill: null };
}

/** SLAPorSLOP's parts for a page: its works (the grid opens pictures large), videos and tracks. */
export function slopParts({ works, onOpen }: { works: Work[]; onOpen?: (w: Work) => void }): DesignParts {
  return {
    works: () =>
      works.length > 0 ? (
        <div className="works">
          {works.map((w) => (
            <WorkTile key={w.key} w={w} onOpen={onOpen} />
          ))}
        </div>
      ) : null,
    work: (key) => {
      const w = works.find((x) => x.key === key);
      return w ? <WorkTile w={w} onOpen={onOpen} /> : null;
    },
    video: (id, title) => <WorkTile w={videoWork(id, title)} />,
    track: (src, title) => <WorkTile w={trackWork(src, title)} />,
  };
}

/** A portfolio set's work as the People page shows works. */
export function portfolioWork(w: PortfolioWork): Work {
  const base = { key: workKey(w), youtube_id: null, audio_url: null, audio_format: null, title: w.title, caption: w.caption, skill: null };
  if (w.kind === "image") return { ...base, kind: "image", url: w.url, width: w.width, height: w.height };
  const poster = { url: w.poster?.url ?? null, width: w.poster?.width ?? null, height: w.poster?.height ?? null };
  if (w.kind === "video") return { ...base, ...poster, kind: "video", youtube_id: w.youtube_id };
  return { ...base, ...poster, kind: "audio", audio_url: w.audio.url, audio_format: w.audio.format };
}

/**
 * A design as it will show, for its creative (the editor) and reviewers (the queue): at desktop or
 * phone width, with the creative's services below it as on their page, and what SLAPorSLOP removed or
 * noted.
 */
export function DesignPreview({
  design,
  works,
  services,
  label,
}: {
  design: PageDesign & { warnings: string[] };
  works: Work[];
  services: CreativePage["services"];
  label: string;
}) {
  const [open, setOpen] = useState<Work>();
  const [phone, setPhone] = useState(false);
  return (
    <div className="design-preview">
      {design.warnings.length > 0 ? (
        <details className="design-warnings">
          <summary>
            {design.warnings.length === 1 ? "One note" : `${design.warnings.length} notes`} on the design: what SLAPorSLOP removed, and what it hides
          </summary>
          <ul className="small">
            {design.warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        </details>
      ) : (
        <p className="muted small">SLAPorSLOP didn't need to remove anything from the design.</p>
      )}
      <div className="row-actions" role="group" aria-label={`Preview width: ${label}`}>
        <button type="button" className="btn small" aria-pressed={!phone} onClick={() => setPhone(false)}>
          Desktop
        </button>
        <button type="button" className="btn small" aria-pressed={phone} onClick={() => setPhone(true)}>
          Phone
        </button>
      </div>
      <div className={`design-preview-frame${phone ? " phone" : ""}`}>
        <DesignArea design={design} label={label} parts={slopParts({ works, onOpen: setOpen })} />
        {services.length > 0 && (
          <section className="design-preview-services" aria-label="Services, below the design">
            <h4 className="small">Services (always below the design, with Hire buttons)</h4>
            <ServicesList services={services} />
          </section>
        )}
      </div>
      {open && <Lightbox w={open} onClose={() => setOpen(undefined)} />}
    </div>
  );
}
