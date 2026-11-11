// SPDX-License-Identifier: AGPL-3.0-or-later
// Maker news (crates/slop-api/src/news.rs; DESIGN §8): short posts by an app's maker. They show
// in the app page's News section (the newest three, older ones on request), in Library →
// Activity, and in the maker's News panel on Manage. People who marked the app Interested are
// told under the bell only (never by email or on a device), unless they switch it off for that
// app with the checkbox here. The headline is plain text; the text is Markdown through the chat
// renderer only (markdown.ts renderChat: no pictures, headings or raw HTML).

import { useEffect, useRef, useState, type FormEvent, type MouseEvent } from "react";
import { Link, useLocation } from "react-router";

import { api, ApiError, type AppDetail, type NewsPage, type NewsPost } from "../api";
import { formatDate, formatDateTime } from "../format";
import { useLoad } from "../hooks";
import { renderChat } from "../markdown";
import { openExternal } from "../platform";
import { ReportButton } from "./Report";
import { ErrorState } from "./ui";

/** A post's text. Its links leave SLAPorSLOP (the system browser, in the desktop app). */
export function NewsBody({ body }: { body: string }) {
  const onClick = (e: MouseEvent) => {
    const link = (e.target as HTMLElement).closest("a[data-external]");
    if (link) {
      e.preventDefault();
      void openExternal((link as HTMLAnchorElement).href);
    }
  };
  return (
    <div
      className="prose small"
      onClick={onClick}
      // Rendered with raw HTML and pictures disabled; see markdown.ts (renderChat).
      dangerouslySetInnerHTML={{ __html: renderChat(body) }}
    />
  );
}

/** "Tell me about news from this app": the person's own switch on their Interested mark. */
export function NewsSwitch({ slug, on, onChange }: { slug: string; on: boolean; onChange: (on: boolean) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const box = useRef<HTMLInputElement>(null);
  // A disabled checkbox loses the keyboard's focus: it gets it back once the answer is in.
  const refocus = useRef(false);
  useEffect(() => {
    if (busy || !refocus.current) return;
    refocus.current = false;
    box.current?.focus();
  }, [busy]);
  const flip = async (next: boolean) => {
    refocus.current = document.activeElement === box.current;
    setBusy(true);
    setError(undefined);
    try {
      await api.interested.setNews(slug, next);
      onChange(next);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <label className="check news-switch">
        <input ref={box} type="checkbox" checked={on} disabled={busy} onChange={(e) => void flip(e.target.checked)} />
        <span>Tell me about news from this app</span>
      </label>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}

/**
 * The News section of an app's store page. Nothing shows while it loads, if it can't load, or
 * when there's no post and no switch to show: a released app keeps the section for the switch of
 * someone who marked it before release (its Interested button is gone). The page shows what the
 * public sees, its maker included.
 */
export function AppNews({
  app,
  marked,
  newsOn,
  onNewsOn,
}: {
  app: AppDetail;
  /** The viewer has an Interested mark on it. */
  marked: boolean;
  newsOn: boolean;
  onNewsOn: (on: boolean) => void;
}) {
  const { hash } = useLocation();
  const [posts, setPosts] = useState<NewsPost[]>();
  const [more, setMore] = useState(false);
  // Where the next page starts: the last post the server sent, whether shown or not.
  const [last, setLast] = useState<string>();
  const [moreError, setMoreError] = useState<string>();
  const loadingMore = useRef(false);
  // After "Show older news": the first new post shown, if any, takes the focus.
  const focusPost = useRef<{ id?: string } | undefined>(undefined);
  const section = useRef<HTMLElement>(null);
  // A link to one post (/app/<slug>#news-<id>) asks for more, so the post is on the page when the
  // layout scrolls to it (Layout.tsx).
  const linked = hash.startsWith("#news-");
  useEffect(() => {
    let current = true;
    api.news.list(app.slug, { limit: linked ? 50 : 3 }).then(
      (page) => {
        if (!current) return;
        setPosts(page.news);
        setMore(page.more);
        setLast(page.news.at(-1)?.id);
      },
      () => current && setPosts(undefined),
    );
    return () => {
      current = false;
    };
  }, [app.slug, linked]);
  useEffect(() => {
    const to = focusPost.current;
    if (!to || !section.current) return;
    focusPost.current = undefined;
    // Its headline; with nothing new to show, the button stays where it was, or the heading.
    if (to.id) section.current.querySelector<HTMLElement>(`[id="news-${to.id}"] h3`)?.focus();
    else if (!section.current.contains(document.activeElement)) section.current.querySelector<HTMLElement>("h2")?.focus();
  }, [posts]);

  if (!posts) return null;
  const shown = posts.filter((n) => !n.hidden);
  if (shown.length === 0 && !(marked && app.stage === "released")) return null;

  const older = async () => {
    if (!last || loadingMore.current) return;
    loadingMore.current = true;
    setMoreError(undefined);
    try {
      const page = await api.news.list(app.slug, { before: last, limit: 10 });
      focusPost.current = { id: page.news.find((n) => !n.hidden)?.id };
      setPosts((p) => [...(p ?? []), ...page.news]);
      setMore(page.more);
      setLast(page.news.at(-1)?.id ?? last);
    } catch (err) {
      setMoreError(err instanceof ApiError ? err.message : String(err));
    } finally {
      loadingMore.current = false;
    }
  };

  return (
    <section ref={section} className="panel panel-pad app-news" id="news" aria-labelledby="news-title">
      <h2 id="news-title" tabIndex={-1}>
        News
      </h2>
      {marked && <NewsSwitch slug={app.slug} on={newsOn} onChange={onNewsOn} />}
      {shown.length === 0 && <p className="muted small">No news from the maker yet.</p>}
      {shown.length > 0 && (
        <ul className="news-list">
          {shown.map((n) => (
            <li key={n.id} id={`news-${n.id}`}>
              <h3 tabIndex={-1}>{n.headline}</h3>
              <span className="faint small">
                {formatDate(n.created_at)}
                {n.edited && " · edited"}
              </span>
              <NewsBody body={n.body} />
              {!app.is_mine && <ReportButton kind="news" subject={n.id} label="this news post" />}
            </li>
          ))}
        </ul>
      )}
      {more && (
        <button type="button" className="btn small" onClick={() => void older()}>
          Show older news
        </button>
      )}
      {moreError && (
        <p className="form-error" role="alert">
          {moreError}
        </p>
      )}
    </section>
  );
}

/** Where the keyboard's focus goes after the panel changes. */
type FocusTo =
  | "write"
  | "heading"
  | { edit: string }
  | { remove: string }
  | { keep: string }
  | { confirm: string }
  /** The first post of the page just asked for, by the place it will have in the list, once the
   * answer that follows `from` is in. */
  | { older: number; from: NewsPage };

/** How many posts the panel asks for at a time. */
const PANEL_PAGE = 50;

/** The maker's newest `pages` pages of posts, so a reload after a change shows as many as before. */
async function newest(slug: string, pages: number): Promise<NewsPage> {
  const first = await api.news.list(slug, { limit: PANEL_PAGE });
  let { news, more } = first;
  for (let i = 1; i < pages && more && news.length > 0; i++) {
    const next = await api.news.list(slug, { before: news[news.length - 1].id, limit: PANEL_PAGE });
    news = [...news, ...next.news];
    more = next.more;
  }
  return { ...first, news, more };
}

/**
 * The maker's News panel on Manage: write a post, edit or remove one, and reach the older ones. A
 * post staff hid stays here with the rule, and the way to appeal while one can be made; it can't
 * be changed. Nothing shows for an app whose page isn't in the store and has no posts.
 */
export function NewsPanel({ slug }: { slug: string }) {
  // Pages shown so far ("Show older posts" adds one); a new app starts again from one.
  const [paged, setPaged] = useState({ slug, pages: 1 });
  const pages = paged.slug === slug ? paged.pages : 1;
  const page = useLoad(() => newest(slug, pages), [slug, pages]);
  const [writing, setWriting] = useState(false);
  const [editing, setEditing] = useState<string>();
  const [confirming, setConfirming] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [rowError, setRowError] = useState<{ id: string; message: string }>();
  // What the last change did, for screen readers (the focus moves on without saying).
  const [said, setSaid] = useState("");
  // Removed here, before the list comes back without them.
  const [gone, setGone] = useState<string[]>([]);
  const panel = useRef<HTMLElement>(null);
  const focusTo = useRef<FocusTo | undefined>(undefined);
  useEffect(() => {
    const to = focusTo.current;
    if (!to || !panel.current) return;
    if (typeof to === "object" && "older" in to) {
      // Once the older posts are in: the first of them, or the heading if none came.
      if (page.loading || page.data === to.from) return;
      focusTo.current = undefined;
      const rows = panel.current.querySelectorAll<HTMLElement>("[data-post]");
      (rows[to.older] ?? panel.current.querySelector<HTMLElement>("h2"))?.focus();
      return;
    }
    focusTo.current = undefined;
    const selector =
      to === "write"
        ? '[data-focus="write"]'
        : to === "heading"
          ? "h2"
          : "edit" in to
            ? `[data-focus="edit-${to.edit}"]`
            : "remove" in to
              ? `[data-focus="remove-${to.remove}"]`
              : "confirm" in to
                ? `[data-focus="confirm-${to.confirm}"]`
                : `[data-focus="keep-${to.keep}"]`;
    (panel.current.querySelector<HTMLElement>(selector) ?? panel.current.querySelector<HTMLElement>("h2"))?.focus();
  });

  if (page.error && !page.data) {
    return (
      <section className="panel panel-pad" aria-labelledby="news-panel-title">
        <h2 id="news-panel-title">News</h2>
        <ErrorState error={page.error} onRetry={page.reload} />
      </section>
    );
  }
  if (!page.data) return null;
  const posts = page.data.news.filter((n) => !gone.includes(n.id));
  const posting = page.data.posting;
  const outOfStore = posting?.why_not === "not_in_store";
  if (outOfStore && posts.length === 0) return null;

  const close = (to: FocusTo) => {
    focusTo.current = to;
    setWriting(false);
    setEditing(undefined);
  };
  const remove = async (n: NewsPost) => {
    setBusy(true);
    setRowError(undefined);
    try {
      await api.news.remove(slug, n.id);
      setGone((g) => [...g, n.id]);
      setConfirming(undefined);
      setSaid("Removed.");
      focusTo.current = "heading";
      page.reload();
    } catch (err) {
      setRowError({ id: n.id, message: err instanceof ApiError ? err.message : String(err) });
      // The button was disabled while it worked, which loses the keyboard's focus: back to it.
      focusTo.current = { confirm: n.id };
    } finally {
      setBusy(false);
    }
  };
  const older = () => {
    if (page.loading) return;
    focusTo.current = { older: posts.length, from: page.data! };
    setPaged({ slug, pages: pages + 1 });
  };

  return (
    <section ref={panel} className="panel panel-pad news-panel" aria-labelledby="news-panel-title">
      <h2 id="news-panel-title" tabIndex={-1}>
        News
      </h2>
      <p className="muted small">
        Short news on your app's page. People who marked it Interested are told under their bell, unless they switched news off for it.
        One new post per app per day.
      </p>
      <p className="sr-only" role="status">
        {said}
      </p>
      {writing ? (
        <NewsForm
          slug={slug}
          onSaved={() => {
            // The panel says when the next post is allowed; there's no Write news button to go back to.
            setSaid("Posted.");
            close("heading");
            page.reload();
          }}
          onClose={() => close("write")}
        />
      ) : posting?.allowed ? (
        <button
          type="button"
          className="btn small slime"
          data-focus="write"
          onClick={() => {
            setEditing(undefined);
            setConfirming(undefined);
            setWriting(true);
          }}
        >
          Write news
        </button>
      ) : posting?.why_not === "wait" && posting.next_post_at ? (
        <p className="small">You can post again on {formatDateTime(posting.next_post_at)}.</p>
      ) : outOfStore ? (
        <p className="small">
          This app's page isn't in the store now, so news can't be posted or changed. You can still remove a post.
        </p>
      ) : null}
      {posts.length > 0 && (
        <ul className="offer-list news-rows">
          {posts.map((n) => (
            <li key={n.id}>
              {editing === n.id ? (
                <NewsForm
                  slug={slug}
                  post={n}
                  onSaved={() => {
                    setSaid("Saved.");
                    close({ edit: n.id });
                    page.reload();
                  }}
                  onClose={() => close({ edit: n.id })}
                />
              ) : (
                <>
                  <div className="news-row-head">
                    <b data-post tabIndex={-1}>
                      {n.headline}
                    </b>{" "}
                    <span className="faint small">
                      {formatDate(n.created_at)}
                      {n.edited && " · edited"}
                    </span>
                    {n.hidden && <span className="badge neutral">HIDDEN</span>}
                  </div>
                  <NewsBody body={n.body} />
                  {n.hidden ? (
                    <p className="small">
                      Removed by a moderation decision{n.hidden_reason ? `: ${n.hidden_reason}` : ""}.{" "}
                      {n.can_appeal ? (
                        <>
                          You can appeal it from <Link to="/account#decisions">your account</Link>.
                        </>
                      ) : (
                        <>
                          See the decision on <Link to="/account#decisions">your account</Link>.
                        </>
                      )}
                    </p>
                  ) : confirming === n.id ? (
                    <div className="row-actions" role="group" aria-labelledby={`news-confirm-${n.id}`}>
                      <span id={`news-confirm-${n.id}`} className="small">
                        Remove this post? It doesn't give the day's post back.
                      </span>
                      <button
                        type="button"
                        className="btn small danger"
                        disabled={busy}
                        data-focus={`confirm-${n.id}`}
                        onClick={() => void remove(n)}
                      >
                        Remove post
                      </button>
                      <button
                        type="button"
                        className="btn small ghost"
                        data-focus={`keep-${n.id}`}
                        onClick={() => {
                          focusTo.current = { remove: n.id };
                          setRowError(undefined);
                          setConfirming(undefined);
                        }}
                      >
                        Keep it
                      </button>
                    </div>
                  ) : (
                    <div className="row-actions">
                      {!outOfStore && (
                        <button
                          type="button"
                          className="btn small ghost"
                          data-focus={`edit-${n.id}`}
                          aria-label={`Edit: ${n.headline}`}
                          onClick={() => {
                            setWriting(false);
                            setConfirming(undefined);
                            setEditing(n.id);
                          }}
                        >
                          Edit
                        </button>
                      )}
                      <button
                        type="button"
                        className="btn small ghost"
                        data-focus={`remove-${n.id}`}
                        aria-label={`Remove: ${n.headline}`}
                        onClick={() => {
                          focusTo.current = { keep: n.id };
                          setRowError(undefined);
                          setConfirming(n.id);
                        }}
                      >
                        Remove
                      </button>
                    </div>
                  )}
                  {rowError?.id === n.id && (
                    <p className="form-error" role="alert">
                      {rowError.message}
                    </p>
                  )}
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      {page.data.more && (
        <button type="button" className="btn small" onClick={older}>
          Show older posts
        </button>
      )}
    </section>
  );
}

/** Writing a new post, or changing one (both fields, pre-filled). */
function NewsForm({ slug, post, onSaved, onClose }: { slug: string; post?: NewsPost; onSaved: () => void; onClose: () => void }) {
  const [headline, setHeadline] = useState(post?.headline ?? "");
  const [body, setBody] = useState(post?.body ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError>();
  const first = useRef<HTMLInputElement>(null);
  const text = useRef<HTMLTextAreaElement>(null);
  const send = useRef<HTMLButtonElement>(null);
  // The button is disabled while it works, which loses the keyboard's focus: after a refusal it
  // goes to the field to fix, or back to the button.
  const refocus = useRef<HTMLElement | null>(null);
  useEffect(() => first.current?.focus(), []);
  useEffect(() => {
    if (busy || !refocus.current) return;
    refocus.current.focus();
    refocus.current = null;
  }, [busy]);
  const editing = Boolean(post);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const sent = { headline, body };
      if (post) await api.news.edit(slug, post.id, sent);
      else await api.news.post(slug, sent);
      onSaved();
    } catch (err) {
      const refused = err instanceof ApiError ? err : new ApiError(0, "unknown", String(err));
      setError(refused);
      refocus.current = refused.field === "headline" ? first.current : refused.field === "body" ? text.current : send.current;
    } finally {
      setBusy(false);
    }
  };
  // A refusal about a field shows under it and is part of what the field says; any other (a wait,
  // the page out of the store) at the end.
  const fieldError = (field: "headline" | "body") =>
    error?.field === field ? (
      <span id={`news-${field}-error`} className="field-error" role="alert">
        {error.message}
      </span>
    ) : null;
  const describedBy = (field: "headline" | "body") => `news-${field}-hint${error?.field === field ? ` news-${field}-error` : ""}`;
  return (
    <form className="form news-form" aria-label={editing ? "Edit news" : "Write news"} onSubmit={(e) => void submit(e)}>
      <label>
        Headline
        <input
          ref={first}
          type="text"
          maxLength={100}
          required
          value={headline}
          onChange={(e) => setHeadline(e.target.value)}
          aria-describedby={describedBy("headline")}
          aria-invalid={error?.field === "headline" || undefined}
        />
      </label>
      <p id="news-headline-hint" className="faint small">
        One line, up to 100 characters. It's what people see in their notifications.
      </p>
      {fieldError("headline")}
      <label>
        News
        <textarea
          ref={text}
          maxLength={2000}
          rows={5}
          required
          value={body}
          onChange={(e) => setBody(e.target.value)}
          aria-describedby={describedBy("body")}
          aria-invalid={error?.field === "body" || undefined}
        />
      </label>
      <p id="news-body-hint" className="faint small">
        Up to 2,000 characters. Simple Markdown works: bold, lists and links. No pictures.
      </p>
      {fieldError("body")}
      {!editing && (
        <p className="small">
          Posting tells the people who marked this app Interested, unless they switched news off, and can't be unsent. You can edit or
          remove the post afterwards.
        </p>
      )}
      {error && error.field !== "headline" && error.field !== "body" && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="row-actions">
        <button ref={send} type="submit" className="btn small slime" disabled={busy}>
          {editing ? "Save changes" : "Post news"}
        </button>
        <button type="button" className="btn small ghost" onClick={onClose}>
          Cancel
        </button>
      </div>
    </form>
  );
}
