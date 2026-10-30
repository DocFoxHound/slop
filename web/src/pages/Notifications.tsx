// SPDX-License-Identifier: AGPL-3.0-or-later
// The bell's page (crates/slop-api/src/notify.rs), as an inbox: the kinds down the side with
// what's unread in each (a row along the top on phones), the notifications grouped by day, Unread
// only, Mark all as read for the kind shown, and each one marked read or unread, or dismissed.
// Opening one marks it read. The kind is in the address (/notifications/friends), so Back works.

import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Link, NavLink, useLocation, useNavigate, useParams, useSearchParams } from "react-router";

import { api, ApiError, type Notification, type NotificationInbox } from "../api";
import { ErrorState, Loading } from "../components/ui";
import { DAY_GROUPS, dayGroup, formatDateTime, timeAgo } from "../format";
import { useDocumentTitle } from "../hooks";
import { useSession } from "../session";

type Kind = Notification["category"];

/** Each kind: its name, icon (a 16×16 line drawing), and what shows up under it (as on the server's CATEGORIES). */
export const KINDS: Array<{ id: Kind; label: string; icon: string; about: string }> = [
  { id: "jobs", label: "People jobs", icon: "M2.5 5.5h11v7.5h-11z M6 5.5V3.5h4v2 M2.5 9h11", about: "Offers, payments, deliveries, messages and problems in your People jobs show up here." },
  { id: "interested", label: "Interested", icon: "M8 2.2l1.8 3.6 4 .6-2.9 2.8.7 4L8 11.3l-3.6 1.9.7-4-2.9-2.8 4-.6z", about: "When an app you're Interested in reaches beta or release, it shows up here." },
  {
    id: "apps",
    label: "Your apps",
    icon: "M2.5 4.8L8 2.3l5.5 2.5v6.4L8 13.7l-5.5-2.5z M2.5 4.8L8 7.3l5.5-2.5 M8 7.3v6.4",
    about: "Every release of your apps (received, approved, rejected, withdrawn), going public, and a repository SLAPorSLOP can't read show up here.",
  },
  { id: "shared", label: "Shared with you", icon: "M4 7.5h8v6H4z M5.7 7.5V5.3a2.3 2.3 0 0 1 4.6 0v2.2", about: "Private apps someone shares with you show up here." },
  { id: "teams", label: "Teams", icon: "M6 7.2a2 2 0 1 0 0-4 2 2 0 0 0 0 4z M2.3 13.2c0-2.1 1.6-3.7 3.7-3.7s3.7 1.6 3.7 3.7 M10.8 7a1.8 1.8 0 1 0 0-3.6 M11.6 9.6c1.3.4 2.1 1.7 2.1 3.6", about: "Team invitations, game nights and verification show up here." },
  { id: "friends", label: "Friends", icon: "M6.5 7.2a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4z M2.5 13.5c0-2.3 1.8-4 4-4s4 1.7 4 4 M12.5 5.2v4 M10.5 7.2h4", about: "Friend requests, and people accepting yours, show up here." },
  { id: "chat", label: "Chat", icon: "M2.5 3.5h11v7.2H8L5 13.2v-2.5H2.5z", about: "New chat messages while you're away show up here." },
  { id: "donations", label: "Donations", icon: "M8 13.3S2.5 10 2.5 6.2A2.8 2.8 0 0 1 8 5a2.8 2.8 0 0 1 5.5 1.2C13.5 10 8 13.3 8 13.3z", about: "Donation receipts, server donation reminders and changes to where yours are paid show up here." },
  { id: "moderation", label: "Moderation", icon: "M3.5 14V2.5 M3.5 3h8.5l-1.8 3 1.8 3H3.5", about: "Decisions about your content, answers to appeals and the outcome of your reports show up here." },
  { id: "agents", label: "AI agents", icon: "M3.5 5.5h9v7h-9z M8 5.5V3 M6.2 8.5v1.2 M9.8 8.5v1.2 M1.8 8v2.5 M14.2 8v2.5", about: "Your AI agent tokens show up here a week and a day before one expires, and when it has." },
  {
    id: "policies",
    label: "Terms and policies",
    icon: "M4 2.5h5.5L12 5v8.5H4z M9.5 2.5V5H12 M6 8h4 M6 10.5h4",
    about: "Changes to SLAPorSLOP's terms and policies show up here, before they take effect.",
  },
  { id: "security", label: "Security", icon: "M8 2l5 2v4c0 3-2.2 5-5 6-2.8-1-5-3-5-6V4z M6 8l1.5 1.5L10.2 7", about: "Passkeys added, sign-ins linked and accounts combined show up here." },
];
const KIND = Object.fromEntries(KINDS.map((k) => [k.id, k])) as Record<Kind, (typeof KINDS)[number]>;
const PAGE = 30;
const BELL = "M4 11V7a4 4 0 0 1 8 0v4l1.2 1.5H2.8z M6.5 13.5a1.5 1.5 0 0 0 3 0";

function Icon({ d }: { d: string }) {
  return (
    <svg className="kind-icon" viewBox="0 0 16 16" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

interface InboxState {
  data?: NotificationInbox;
  /** What was shown before, while another kind loads: its counts stay on the side meanwhile. */
  stale?: NotificationInbox;
  error?: ApiError;
  loadingMore: boolean;
}

function asApiError(err: unknown): ApiError {
  return err instanceof ApiError ? err : new ApiError(0, "unknown", String(err));
}

/**
 * Newer ones that arrived, put in front of what's shown: the first page again, and what was
 * loaded after it kept (unless the first page is all there is now).
 */
function merge(shown: NotificationInbox, fresh: NotificationInbox): NotificationInbox {
  if (!fresh.more || fresh.items.length === 0) return fresh;
  const seen = new Set(fresh.items.map((n) => n.id));
  const last = fresh.items[fresh.items.length - 1].created_at;
  const older = shown.items.filter((n) => !seen.has(n.id) && n.created_at < last);
  return { ...fresh, items: [...fresh.items, ...older], more: older.length > 0 ? shown.more : fresh.more };
}

export function Notifications() {
  const { kind: kindParam } = useParams();
  const [search, setSearch] = useSearchParams();
  const kind = kindParam as Kind | undefined;
  const known = kind === undefined || kind in KIND;
  // Unread only is in the address too; the switch moves at once, before the address does.
  const unreadInAddress = search.get("unread") === "1";
  const [unreadOnly, setUnreadOnlyNow] = useState(unreadInAddress);
  useEffect(() => setUnreadOnlyNow(unreadInAddress), [unreadInAddress]);
  useDocumentTitle(kind && known ? `${KIND[kind].label} · Notifications` : "Notifications");
  const { me, refresh } = useSession();
  const navigate = useNavigate();
  const location = useLocation();
  const [state, setState] = useState<InboxState>({ loadingMore: false });
  const [error, setError] = useState<string>();
  const [said, say] = useState("");
  const [now, setNow] = useState(() => Date.now());
  const generation = useRef(0);
  const heading = useRef<HTMLHeadingElement>(null);
  const shownKind = useRef(kind);
  const list = useRef<HTMLDivElement>(null);
  const kindsRow = useRef<HTMLElement>(null);
  const signedIn = me ? me.id : undefined;

  const filter = useCallback(
    (before?: string) => ({ category: kind, unread: unreadOnly || undefined, before, limit: PAGE }),
    [kind, unreadOnly],
  );

  // The first page, whenever the kind or Unread only changes.
  useEffect(() => {
    if (!signedIn || !known) return;
    const mine = ++generation.current;
    setState((s) => ({ stale: s.data ?? s.stale, loadingMore: false }));
    setError(undefined);
    api.notifications.list(filter()).then(
      (data) => mine === generation.current && setState({ data, loadingMore: false }),
      (err: unknown) => mine === generation.current && setState({ error: asApiError(err), loadingMore: false }),
    );
  }, [signedIn, known, filter]);

  // New ones while the page is open (the bell checks every minute): added at the top, and said.
  const bellCount = me?.unread_notifications;
  const shownUnread = state.data?.unread;
  useEffect(() => {
    if (bellCount === undefined || shownUnread === undefined || bellCount === shownUnread) return;
    const mine = generation.current;
    void api.notifications
      .list(filter())
      .then((fresh) => {
        if (mine !== generation.current) return;
        setState((s) => {
          if (!s.data) return s;
          const before = new Set(s.data.items.map((n) => n.id));
          const added = fresh.items.filter((n) => !before.has(n.id)).length;
          if (added > 0) say(added === 1 ? "1 new notification" : `${added} new notifications`);
          return { ...s, data: merge(s.data, fresh) };
        });
      })
      .catch(() => undefined);
  }, [bellCount, shownUnread, filter]);

  // "3 min ago" keeps up.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  // Choosing another kind puts focus on its heading, so keyboard and screen reader users start
  // there (not on the first load).
  useEffect(() => {
    if (shownKind.current === kind) return;
    shownKind.current = kind;
    heading.current?.focus();
  }, [kind]);

  const loaded = state.data !== undefined;
  // On phones the kinds are a row that scrolls sideways: keep the one shown in view.
  useEffect(() => {
    const row = kindsRow.current;
    const active = row?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!row || !active || row.scrollWidth <= row.clientWidth) return;
    const box = row.getBoundingClientRect();
    const chip = active.getBoundingClientRect();
    row.scrollLeft += chip.left - box.left - (box.width - chip.width) / 2;
  }, [kind, me, loaded]);

  if (me === null) {
    return (
      <div className="page narrow">
        <div className="empty-state">
          <Link to="/signin?return_to=/notifications">Sign in</Link> to see your notifications.
        </div>
      </div>
    );
  }

  const data = state.data;
  const counts = (data ?? state.stale)?.counts ?? {};
  const allUnread = (data ?? state.stale)?.unread ?? 0;
  const unreadHere = data ? (kind ? (counts[kind] ?? 0) : data.unread) : 0;
  const kindName = kind && known ? KIND[kind].label : "All notifications";

  /** Changes one notification where it's shown, and the counts with it. */
  const update = (id: string, change: (n: Notification) => Notification | null) =>
    setState((s) => {
      if (!s.data) return s;
      const counts = { ...(s.data.counts ?? {}) };
      let unread = s.data.unread;
      const items: Notification[] = [];
      for (const n of s.data.items) {
        if (n.id !== id) {
          items.push(n);
          continue;
        }
        const next = change(n);
        const delta = (next && !next.read ? 1 : 0) - (n.read ? 0 : 1);
        counts[n.category] = Math.max(0, (counts[n.category] ?? 0) + delta);
        unread = Math.max(0, unread + delta);
        if (next) items.push(next);
      }
      return { ...s, data: { ...s.data, items, counts, unread } };
    });

  const fail = (err: unknown) => setError(asApiError(err).message);

  const open = async (n: Notification) => {
    if (!n.read) await api.notifications.read([n.id]).catch(() => undefined);
    void refresh();
    navigate(n.link);
  };

  const toggleRead = async (n: Notification) => {
    setError(undefined);
    try {
      await (n.read ? api.notifications.unread([n.id]) : api.notifications.read([n.id]));
    } catch (err) {
      fail(err);
      return;
    }
    update(n.id, (x) => ({ ...x, read: !n.read }));
    say(n.read ? "Marked as unread" : "Marked as read");
    void refresh();
  };

  const dismiss = async (n: Notification) => {
    setError(undefined);
    // Focus goes to the next one (or the one before), not back to the top of the page.
    const rows = [...(list.current?.querySelectorAll<HTMLButtonElement>("button.notification") ?? [])];
    const at = rows.findIndex((b) => b.dataset.id === n.id);
    const next = rows[at + 1] ?? rows[at - 1];
    try {
      await api.notifications.dismiss(n.id);
    } catch (err) {
      fail(err);
      return;
    }
    update(n.id, () => null);
    say("Dismissed");
    void refresh();
    if (next) next.focus();
    else heading.current?.focus();
  };

  const readAll = async () => {
    setError(undefined);
    try {
      await api.notifications.read([], kind);
    } catch (err) {
      fail(err);
      return;
    }
    setState((s) => {
      if (!s.data) return s;
      const counts = { ...(s.data.counts ?? {}) };
      if (kind) delete counts[kind];
      const unread = kind ? s.data.unread - (s.data.counts?.[kind] ?? 0) : 0;
      return {
        ...s,
        data: {
          ...s.data,
          counts: kind ? counts : {},
          unread: Math.max(0, unread),
          items: s.data.items.map((n) => (!kind || n.category === kind ? { ...n, read: true } : n)),
        },
      };
    });
    say(kind ? `All ${KIND[kind].label} notifications marked as read` : "All notifications marked as read");
    void refresh();
  };

  const loadMore = async () => {
    const shown = state.data;
    if (!shown || shown.items.length === 0) return;
    const mine = generation.current;
    setState((s) => ({ ...s, loadingMore: true }));
    try {
      const page = await api.notifications.list(filter(shown.items[shown.items.length - 1].id));
      if (mine !== generation.current) return;
      setState((s) => {
        if (!s.data) return s;
        const seen = new Set(s.data.items.map((n) => n.id));
        return { data: { ...page, items: [...s.data.items, ...page.items.filter((n) => !seen.has(n.id))] }, loadingMore: false };
      });
    } catch (err) {
      if (mine !== generation.current) return;
      setState((s) => ({ ...s, loadingMore: false }));
      fail(err);
    }
  };

  const setUnreadOnly = (on: boolean) => {
    setUnreadOnlyNow(on);
    const next = new URLSearchParams(search);
    if (on) next.set("unread", "1");
    else next.delete("unread");
    setSearch(next, { replace: true });
  };

  // Up and down move between notifications, Home and End to the first and last.
  const onListKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const target = e.target as HTMLElement;
    if (!target.classList.contains("notification")) return;
    const rows = [...(list.current?.querySelectorAll<HTMLButtonElement>("button.notification") ?? [])];
    const at = rows.indexOf(target as HTMLButtonElement);
    const to = e.key === "Home" ? 0 : e.key === "End" ? rows.length - 1 : at + (e.key === "ArrowDown" ? 1 : -1);
    if (to < 0 || to >= rows.length) return;
    e.preventDefault();
    rows[to].focus();
  };

  const kindLink = (to: string) => ({ pathname: to, search: location.search });
  const badge = (n: number) =>
    n > 0 ? (
      // Read as "Friends 2 unread".
      <span className="nbox-count">
        {n > 99 ? "99+" : n}
        <span className="sr-only"> unread</span>
      </span>
    ) : null;

  const items = data?.items ?? [];
  const groups = DAY_GROUPS.map((g) => ({ name: g, items: items.filter((n) => dayGroup(n.created_at, now) === g) })).filter((g) => g.items.length > 0);

  return (
    <div className="page nbox-page">
      <h1 className="page-title">Notifications</h1>
      <div className="nbox">
        <nav className="nbox-nav" aria-label="Kinds of notification" ref={kindsRow}>
          <ul>
            <li>
              <NavLink to={kindLink("/notifications")} end className="nbox-kind">
                <Icon d={BELL} />
                <span className="nbox-kind-name">All</span>
                {badge(allUnread)}
              </NavLink>
            </li>
            {KINDS.map((k) => (
              <li key={k.id}>
                <NavLink to={kindLink(`/notifications/${k.id}`)} className="nbox-kind">
                  <Icon d={k.icon} />
                  <span className="nbox-kind-name">{k.label}</span>
                  {badge(counts[k.id] ?? 0)}
                </NavLink>
              </li>
            ))}
          </ul>
        </nav>

        <section className="nbox-main" aria-labelledby="nbox-heading">
          <div className="nbox-head">
            <h2 id="inbox-heading" ref={heading} tabIndex={-1}>
              {kindName}
            </h2>
            <label className="nbox-toggle">
              <input type="checkbox" role="switch" checked={unreadOnly} onChange={(e) => setUnreadOnly(e.target.checked)} />
              Unread only
            </label>
            {unreadHere > 0 && (
              <button type="button" className="btn small" onClick={() => void readAll()}>
                Mark all as read
              </button>
            )}
          </div>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <p className="sr-only" role="status">
            {said}
          </p>

          {!known ? (
            <div className="empty-state">
              There's no such kind of notification. <Link to="/notifications">See them all</Link>
            </div>
          ) : state.error ? (
            <ErrorState
              error={state.error}
              onRetry={() => {
                setState({ loadingMore: false });
                generation.current++;
                void api.notifications.list(filter()).then(
                  (d) => setState({ data: d, loadingMore: false }),
                  (err: unknown) => setState({ error: asApiError(err), loadingMore: false }),
                );
              }}
            />
          ) : !data || me === undefined ? (
            <Loading />
          ) : items.length === 0 ? (
            <div className="empty-state nbox-empty">
              {unreadOnly ? (
                <p>{kind ? `Nothing unread in ${KIND[kind].label}.` : "You're all caught up: nothing unread."}</p>
              ) : kind ? (
                <>
                  <Icon d={KIND[kind].icon} />
                  <p>Nothing here yet.</p>
                  <p className="muted">{KIND[kind].about}</p>
                </>
              ) : (
                <p>Nothing yet. Job steps, apps you're Interested in and decisions about your content show up here.</p>
              )}
            </div>
          ) : (
            <div className="nbox-list" ref={list} onKeyDown={onListKey}>
              {groups.map((g) => (
                <section key={g.name} className="nbox-day" aria-labelledby={`inbox-day-${g.name.replace(/\W+/g, "-")}`}>
                  <h3 id={`inbox-day-${g.name.replace(/\W+/g, "-")}`}>{g.name}</h3>
                  <ul className="notifications panel">
                    {g.items.map((n) => (
                      <Row key={n.id} n={n} now={now} onOpen={open} onToggle={toggleRead} onDismiss={dismiss} />
                    ))}
                  </ul>
                </section>
              ))}
              {data.more && (
                <button type="button" className="btn nbox-more" disabled={state.loadingMore} onClick={() => void loadMore()}>
                  {state.loadingMore ? "Loading…" : "Load older notifications"}
                </button>
              )}
            </div>
          )}
          <p className="muted small nbox-settings">
            Choose which of these are also emailed to you on <Link to="/account/notifications">your account page</Link>.
          </p>
        </section>
      </div>
    </div>
  );
}

function Row({
  n,
  now,
  onOpen,
  onToggle,
  onDismiss,
}: {
  n: Notification;
  now: number;
  onOpen: (n: Notification) => Promise<void>;
  onToggle: (n: Notification) => Promise<void>;
  onDismiss: (n: Notification) => Promise<void>;
}) {
  const kind = KIND[n.category] as (typeof KINDS)[number] | undefined;
  const titleId = `n-title-${n.id}`;
  return (
    <li className={n.read ? "" : "unread"}>
      <button type="button" className="notification" data-id={n.id} onClick={() => void onOpen(n)}>
        <span className="n-icon">
          <Icon d={kind?.icon ?? BELL} />
        </span>
        <span className="n-text">
          <span className="n-title" id={titleId}>
            {kind && <span className="sr-only">{kind.label}: </span>}
            {n.title}
            {n.count > 1 && <span className="n-times"> ({n.count})</span>}
          </span>
          {n.body && <span className="n-body">{n.body}</span>}
        </span>
        <span className="n-meta">
          <time dateTime={n.created_at} title={formatDateTime(n.created_at)}>
            {timeAgo(n.created_at, now)}
          </time>
          {!n.read && <span className="n-dot" aria-hidden="true" />}
        </span>
        {!n.read && <span className="sr-only">, unread</span>}
      </button>
      <span className="n-actions">
        <button
          type="button"
          className="n-action"
          aria-label={n.read ? "Mark as unread" : "Mark as read"}
          aria-describedby={titleId}
          title={n.read ? "Mark as unread" : "Mark as read"}
          onClick={() => void onToggle(n)}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true">
            {n.read ? <circle cx="8" cy="8" r="3.5" /> : <path d="M3 8.5l3 3 7-7" />}
          </svg>
        </button>
        <button type="button" className="n-action" aria-label="Dismiss" aria-describedby={titleId} title="Dismiss" onClick={() => void onDismiss(n)}>
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path d="M4 4l8 8M12 4l-8 8" />
          </svg>
        </button>
      </span>
    </li>
  );
}
