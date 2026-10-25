// SPDX-License-Identifier: AGPL-3.0-or-later
// Admin → Review queue, as an inbox: categories down the left with how many wait in each, one line
// per item (what it is, a short blurb, its status, how long it has waited) with quick Approve and
// Deny where an item has them, and the full item (QueueCards.tsx) when a line is opened. The
// category and the item are in the address (inbox.ts), so a view can be linked and Back works.
// On a phone the categories become a menu. Denying always asks "Are you sure?" first, with the
// reason the person will read.

import { useEffect, useId, useRef, useState, type ReactNode, type Ref } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";

import {
  api,
  ApiError,
  type AdminDonationHold,
  type AdminQueue,
  type AiThreadSummary,
  type LegalRequest,
  type StaffActions,
} from "../../api";
import { LegalRequestCard } from "../../components/LegalRequests";
import { ErrorState, Loading } from "../../components/ui";
import { formatDate } from "../../format";
import { useLoad, type Loaded } from "../../hooks";
import { HoldCard } from "./DonationHolds";
import {
  BOX,
  BOXES,
  boxCounts,
  defaultBox,
  inboxUrl,
  isBox,
  rowsFor,
  sortRows,
  totalWaiting,
  waited,
  waitedLong,
  type BoxKey,
  type InboxData,
  type Order,
  type RowInfo,
} from "./inbox";
import {
  AbuseCard,
  AppealCard,
  ChangeCard,
  DisputeCard,
  MediaCard,
  OpsAlerts,
  PortfolioCard,
  ProblemCard,
  ReadyCard,
  ReleaseCard,
  ReportCard,
  RescanCard,
  ServerChecks,
  StaffActionCard,
} from "./QueueCards";
import { ActionMessages } from "./shared";

/** Everything waiting for staff, loaded once for the whole admin panel (Home, the queue, its tab). */
export interface Waiting {
  queue: Loaded<AdminQueue>;
  actions: Loaded<StaffActions>;
  ai: Loaded<AiThreadSummary[]>;
  legal: Loaded<LegalRequest[]>;
  holds: Loaded<AdminDonationHold[]>;
  data: InboxData;
  counts: Record<BoxKey, number | undefined>;
  /** Everything waiting; undefined until the queue has loaded. */
  total: number | undefined;
  reload: () => void;
}

export function useWaiting(): Waiting {
  const queue = useLoad(() => api.admin.queue(), []);
  const actions = useLoad(() => api.admin.actions(), []);
  const ai = useLoad(() => api.admin.aiMessages(undefined, "open"), []);
  // Legal requests are for staff who may take sensitive actions: for others this is refused, and
  // the category isn't shown.
  const legal = useLoad(() => api.admin.legalRequests(), []);
  const holds = useLoad(() => api.admin.donationHolds("open"), []);
  const data: InboxData = {
    queue: queue.data,
    actions: actions.data,
    ai: ai.data,
    legal: legal.data,
    holds: holds.data,
  };
  const counts = boxCounts(data);
  return {
    queue,
    actions,
    ai,
    legal,
    holds,
    data,
    counts,
    total: queue.data ? totalWaiting(counts) : undefined,
    reload: () => {
      queue.reload();
      actions.reload();
      ai.reload();
      legal.reload();
      holds.reload();
    },
  };
}

const HIDE_EMPTY = "slop.admin.inbox.hideEmpty";

function readHideEmpty(): boolean {
  try {
    return localStorage.getItem(HIDE_EMPTY) === "1";
  } catch {
    return false;
  }
}

export function QueueTab({ waiting }: { waiting: Waiting }) {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const [announce, setAnnounce] = useState<string>();
  const boxParam = params.get("box");
  const item = params.get("item") ?? undefined;
  const order: Order = params.get("sort") === "newest" ? "newest" : "oldest";
  const { counts } = waiting;
  const box: BoxKey = isBox(boxParam) && counts[boxParam] !== undefined ? boxParam : defaultBox(counts);

  // A new category, or opening an item, starts without the last message (coming back to the list
  // after deciding one keeps it: it says what happened).
  useEffect(() => setAnnounce(undefined), [box]);
  useEffect(() => {
    if (item) setAnnounce(undefined);
  }, [item]);

  if (waiting.queue.error) return <ErrorState error={waiting.queue.error} onRetry={waiting.reload} />;
  if (!waiting.queue.data) return <Loading />;
  const q = waiting.queue.data;

  return (
    <>
      <OpsAlerts />
      <ServerChecks checks={q.checks} />
      <div className="inbox">
        <InboxNav box={box} counts={counts} order={order} />
        <div className="inbox-main">
          <div role="status" aria-live="polite" className="inbox-announce">
            {announce && <p className="notice">{announce}</p>}
          </div>
          {item ? (
            <InboxDetail
              key={`${box}/${item}`}
              box={box}
              item={item}
              order={order}
              waiting={waiting}
              onNotice={setAnnounce}
              onGone={(title) => {
                setAnnounce((a) => a ?? `Done: “${title}” isn't waiting any more.`);
                void navigate(inboxUrl(box, undefined, order), { replace: true });
              }}
            />
          ) : (
            <InboxList box={box} order={order} waiting={waiting} onAnnounce={setAnnounce} />
          )}
        </div>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------------------------
// The categories

function InboxNav({ box, counts, order }: { box: BoxKey; counts: Record<BoxKey, number | undefined>; order: Order }) {
  const navigate = useNavigate();
  const id = useId();
  const [hideEmpty, setHideEmpty] = useState(readHideEmpty);
  const shown = BOXES.filter((b) => counts[b.key] !== undefined);
  const groups = [...new Set(shown.map((b) => b.group))];
  const toggle = () => {
    const next = !hideEmpty;
    setHideEmpty(next);
    try {
      localStorage.setItem(HIDE_EMPTY, next ? "1" : "0");
    } catch {
      // Remembered for this visit only.
    }
  };
  return (
    <>
      <nav className="inbox-nav" aria-label="Review categories">
        <button type="button" className="btn small ghost inbox-hide" aria-pressed={hideEmpty} onClick={toggle}>
          {hideEmpty ? "Show empty categories" : "Hide empty categories"}
        </button>
        {groups.map((g, gi) => {
          const boxes = shown.filter((b) => b.group === g && (!hideEmpty || counts[b.key] || b.key === box));
          if (boxes.length === 0) return null;
          return (
            <div key={g} className="inbox-group">
              <p className="inbox-group-label" id={`${id}-g${gi}`}>
                {g}
              </p>
              <ul aria-labelledby={`${id}-g${gi}`}>
                {boxes.map((b) => {
                  const n = counts[b.key] ?? 0;
                  return (
                    <li key={b.key}>
                      <Link
                        to={inboxUrl(b.key, undefined, order)}
                        className={`inbox-box${b.key === box ? " on" : ""}${n === 0 ? " empty" : ""}`}
                        aria-current={b.key === box ? "page" : undefined}
                      >
                        <span>{b.label}</span>
                        <span className="inbox-count">
                          {n}
                          <span className="sr-only"> waiting</span>
                        </span>
                      </Link>
                    </li>
                  );
                })}
              </ul>
            </div>
          );
        })}
      </nav>
      <label className="inbox-select">
        Category
        <select value={box} onChange={(e) => void navigate(inboxUrl(e.target.value as BoxKey, undefined, order))}>
          {groups.map((g) => (
            <optgroup key={g} label={g}>
              {shown
                .filter((b) => b.group === g)
                .map((b) => (
                  <option key={b.key} value={b.key}>
                    {b.label} ({counts[b.key] ?? 0})
                  </option>
                ))}
            </optgroup>
          ))}
        </select>
      </label>
    </>
  );
}

// ---------------------------------------------------------------------------------------------
// One category's list

/** A quick decision from the list. */
interface Quick {
  /** The button: "Approve", "Deny", "List"… */
  label: string;
  /** What's announced once it's done: "Approved", "Denied"… */
  done: string;
  /** The reason the API needs, and who reads it. */
  reason?: { label: string; placeholder: string };
  /** Why it can't be done from here (the button is disabled and says so). */
  unavailable?: string;
  run: (reason: string) => Promise<unknown>;
  /** An announcement of its own from the answer, if it has one. */
  told?: (answer: unknown) => string | undefined;
}

interface Quicks {
  approve?: Quick;
  deny?: Quick;
}

/** Which items can be approved or denied from the list, and how (the same requests as their cards). */
function quicksFor(box: BoxKey, key: string, d: InboxData): Quicks {
  const q = d.queue;
  switch (box) {
    case "releases": {
      const r = q?.releases.find((x) => x.id === key);
      if (!r) return {};
      return {
        approve: {
          label: "Approve",
          done: "Approved and signed",
          unavailable: r.status !== "awaiting_review" ? "Only a release waiting for review can be approved." : undefined,
          run: () => api.admin.approve(r.id),
        },
        deny: {
          label: "Deny",
          done: "Rejected",
          reason: { label: "Why (the maker reads this)", placeholder: "Reason, shown to the maker" },
          run: (reason) => api.admin.reject(r.id, reason),
        },
      };
    }
    case "changes":
      return {
        approve: { label: "Approve", done: "Approved the change to", run: () => api.admin.approveChange(key) },
        deny: {
          label: "Deny",
          done: "Turned down the change to",
          reason: { label: "Why (the maker reads this)", placeholder: "Reason, shown to the maker" },
          run: (reason) => api.admin.rejectChange(key, reason),
        },
      };
    case "pictures":
      return {
        approve: { label: "Approve", done: "Approved the pictures of", run: () => api.admin.approveMedia(key) },
        deny: {
          label: "Deny",
          done: "Turned down the pictures of",
          reason: { label: "What's wrong (the maker reads this)", placeholder: "What's wrong, shown to the maker" },
          run: (reason) => api.admin.rejectMedia(key, reason),
        },
      };
    case "people":
      return {
        approve: { label: "Approve", done: "Approved the People page of", run: () => api.admin.approvePortfolio(key) },
        deny: {
          label: "Deny",
          done: "Turned down the portfolio of",
          reason: { label: "What's wrong (the creative reads this)", placeholder: "What's wrong, shown to the creative" },
          run: (reason) => api.admin.rejectPortfolio(key, reason),
        },
      };
    case "listing": {
      const a = q?.ready_to_list.find((x) => x.slug === key);
      if (!a) return {};
      return {
        approve: {
          label: a.going_public ? "Make public and list" : "List",
          done: "Put in the store:",
          unavailable: a.pictures_pending ? "Review its pictures first." : undefined,
          run: () => api.admin.list(a.slug),
        },
      };
    }
    case "rescans": {
      const [release, platform] = key.split(":");
      return {
        approve: {
          label: "Keep",
          done: "Kept",
          reason: {
            label: "Why it's a false positive (logged, at least 10 characters)",
            placeholder: "Why it's a false positive",
          },
          run: (reason) => api.admin.clearRescan(release, platform, reason),
        },
        deny: {
          label: "Withdraw",
          done: "Withdrew",
          reason: { label: "Why (the maker and players read this)", placeholder: "Why, shown to the maker and players" },
          run: (reason) => api.admin.revoke(release, reason),
        },
      };
    }
    case "appeals": {
      const a = q?.appeals.find((x) => x.id === key);
      if (!a) return {};
      return {
        approve: {
          label: "Reverse",
          done: "Reversed the decision:",
          reason: { label: "Why (the person reads this)", placeholder: "Why it's reversed" },
          run: (reason) => api.admin.decideAppeal(a.id, true, reason),
          told: (answer) => {
            const out = (answer as { stays_out?: string | null } | undefined)?.stays_out;
            return out
              ? `Reversed, but ${a.subject_label} can't go back in the store yet. The maker was told: ${out}`
              : undefined;
          },
        },
        deny: {
          label: "Keep decision",
          done: "Kept the decision:",
          reason: { label: "Why (the person reads this)", placeholder: "Why the decision stands" },
          run: (reason) => api.admin.decideAppeal(a.id, false, reason),
        },
      };
    }
    case "approvals":
      return {
        approve: { label: "Approve", done: "Approved", run: () => api.admin.approveAction(key) },
        deny: { label: "Deny", done: "Turned down", run: () => api.admin.cancelAction(key) },
      };
    case "donations": {
      const h = d.holds?.find((x) => x.id === key);
      const asked = h?.requests.find((r) => r.outcome === null);
      if (!asked) return {};
      return {
        approve: {
          label: "Lift pause",
          done: "Lifted the donation pause on",
          reason: { label: "Your answer to the maker", placeholder: "What you checked" },
          run: (answer) => api.admin.answerDonationHoldRequest(asked.id, true, answer),
        },
        deny: {
          label: "Keep paused",
          done: "Kept the donation pause on",
          reason: { label: "Your answer to the maker", placeholder: "What still needs fixing" },
          run: (answer) => api.admin.answerDonationHoldRequest(asked.id, false, answer),
        },
      };
    }
    default:
      // Reports, problems, disputes, abuse alerts, legal requests and AI messages are decided in
      // full: open them.
      return {};
  }
}

/** Where a line leads: the item in the inbox, or (AI messages) the thread on its own tab. */
function hrefFor(box: BoxKey, key: string, order: Order): string {
  if (box === "ai") return `/admin?tab=ai-messages&thread=${encodeURIComponent(key)}`;
  return inboxUrl(box, key, order);
}

function InboxList({
  box,
  order,
  waiting,
  onAnnounce,
}: {
  box: BoxKey;
  order: Order;
  waiting: Waiting;
  onAnnounce: (text: string) => void;
}) {
  const navigate = useNavigate();
  const def = BOX[box];
  const headingId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const links = useRef(new Map<string, HTMLAnchorElement>());
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();
  /** The line just decided: once it's gone, focus moves to the line now in its place. */
  const decided = useRef<{ key: string; index: number }>(undefined);
  const rows = sortRows(rowsFor(box, waiting.data), order);
  const keys = rows.map((r) => r.key).join("\n");

  useEffect(() => {
    const d = decided.current;
    if (!d || rows.some((r) => r.key === d.key)) return;
    decided.current = undefined;
    const next = rows[Math.min(d.index, rows.length - 1)];
    (next ? links.current.get(next.key) : heading.current)?.focus();
  }, [keys]);

  const act = async (row: RowInfo, index: number, quick: Quick, reason: string): Promise<boolean> => {
    setBusy(row.key);
    setError(undefined);
    try {
      const answer = await quick.run(reason);
      const pending = !!answer && typeof answer === "object" && "pending_action" in answer;
      onAnnounce(
        pending
          ? `Recorded: “${row.title}” waits for another staff member to approve it under Second approvals.`
          : (quick.told?.(answer) ?? `${quick.done} “${row.title}”.`),
      );
      decided.current = { key: row.key, index };
      waiting.reload();
      return true;
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
      return false;
    } finally {
      setBusy(undefined);
    }
  };

  return (
    <section className="inbox-list" aria-labelledby={headingId}>
      <div className="inbox-list-head">
        <h2 className="section-title" id={headingId} ref={heading} tabIndex={-1}>
          {def.label} <span className="muted">({rows.length})</span>
        </h2>
        <label className="small inbox-order">
          Order{" "}
          <select
            value={order}
            onChange={(e) => void navigate(inboxUrl(box, undefined, e.target.value as Order), { replace: true })}
          >
            <option value="oldest">Waiting longest first</option>
            <option value="newest">Newest first</option>
          </select>
        </label>
      </div>
      <p className="muted small">{def.help}</p>
      <ActionMessages error={error} />
      {rows.length === 0 ? (
        <div className="empty-state">Nothing waiting here.</div>
      ) : (
        <ul className="inbox-rows" aria-label={def.label}>
          {rows.map((row, i) => (
            <InboxRow
              key={row.key}
              row={row}
              href={hrefFor(box, row.key, order)}
              quicks={quicksFor(box, row.key, waiting.data)}
              busy={busy}
              linkRef={(el) => {
                if (el) links.current.set(row.key, el);
                else links.current.delete(row.key);
              }}
              onAct={(quick, reason) => act(row, i, quick, reason)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function InboxRow({
  row,
  href,
  quicks,
  busy,
  linkRef,
  onAct,
}: {
  row: RowInfo;
  href: string;
  quicks: Quicks;
  busy: string | undefined;
  linkRef: (el: HTMLAnchorElement | null) => void;
  onAct: (quick: Quick, reason: string) => Promise<boolean>;
}) {
  const [asking, setAsking] = useState<"approve" | "deny">();
  const [reason, setReason] = useState("");
  const approveButton = useRef<HTMLButtonElement>(null);
  const denyButton = useRef<HTMLButtonElement>(null);
  const yesButton = useRef<HTMLButtonElement>(null);
  /** Focus back on a button once it's enabled again. */
  const refocus = (button: { current: HTMLButtonElement | null }) => window.setTimeout(() => button.current?.focus(), 0);
  const questionId = useId();
  const disabled = busy !== undefined;
  const quick = asking ? quicks[asking] : undefined;

  const close = () => {
    const back = asking === "deny" ? denyButton : approveButton;
    setAsking(undefined);
    setReason("");
    // Back where they were.
    refocus(back);
  };

  return (
    <li className={`inbox-row${asking ? " asking" : ""}`}>
      <div className="inbox-line">
        <Link ref={linkRef} className="inbox-open" to={href}>
          <b>{row.title}</b>
          {row.by && <span className="muted"> {row.by}</span>}
        </Link>
        <span className="inbox-blurb muted small" title={row.blurb}>
          {row.blurb}
        </span>
        <span className={`badge ${row.status.tone} inbox-pill`}>{row.status.label.toUpperCase()}</span>
        <time className="inbox-age small muted" dateTime={row.since} title={`${waitedLong(row.since)} (since ${formatDate(row.since)})`}>
          {waited(row.since)}
          <span className="sr-only"> waiting</span>
        </time>
        <span className="inbox-actions">
          {quicks.approve && (
            <button
              ref={approveButton}
              type="button"
              className="btn small slime"
              aria-label={`${quicks.approve.label} ${row.title}`}
              title={quicks.approve.unavailable}
              aria-expanded={quicks.approve.reason ? asking === "approve" : undefined}
              disabled={disabled || !!quicks.approve.unavailable}
              onClick={() => {
                if (quicks.approve!.reason) setAsking("approve");
                // Refused: back on the button (it was disabled while busy).
                else void onAct(quicks.approve!, "").then((ok) => ok || refocus(approveButton));
              }}
            >
              {busy === row.key && !asking ? "…" : quicks.approve.label}
            </button>
          )}
          {quicks.deny && (
            <button
              ref={denyButton}
              type="button"
              className="btn small ghost danger"
              aria-label={`${quicks.deny.label} ${row.title}`}
              aria-expanded={asking === "deny"}
              disabled={disabled}
              onClick={() => setAsking("deny")}
            >
              {quicks.deny.label}
            </button>
          )}
        </span>
      </div>
      {quick && (
        <form
          className="inbox-confirm"
          aria-labelledby={questionId}
          onSubmit={(e) => {
            e.preventDefault();
            if (quick.reason && !reason.trim()) return;
            void onAct(quick, reason.trim()).then((ok) => {
              if (ok) {
                setAsking(undefined);
                setReason("");
              } else refocus(yesButton);
            });
          }}
        >
          <p id={questionId}>
            {asking === "deny" ? (
              <>
                <b>Are you sure?</b> {quick.label} “{row.title}”.
              </>
            ) : (
              <>
                {quick.label} “{row.title}”.
              </>
            )}
          </p>
          {quick.reason ? (
            <label>
              {quick.reason.label}
              <input
                autoFocus
                value={reason}
                maxLength={2000}
                placeholder={quick.reason.placeholder}
                onChange={(e) => setReason(e.target.value)}
                onKeyDown={(e) => e.key === "Escape" && close()}
              />
            </label>
          ) : null}
          <div className="row-actions">
            <ConfirmButton
              buttonRef={yesButton}
              danger={asking === "deny"}
              disabled={disabled || (!!quick.reason && !reason.trim())}
              autoFocus={!quick.reason}
            >
              {busy === row.key ? "…" : `Yes, ${quick.label.toLowerCase()}`}
            </ConfirmButton>
            <button type="button" className="btn small ghost" onClick={close}>
              Cancel
            </button>
          </div>
        </form>
      )}
    </li>
  );
}

function ConfirmButton({
  buttonRef,
  danger,
  disabled,
  autoFocus,
  children,
}: {
  buttonRef: Ref<HTMLButtonElement>;
  danger: boolean;
  disabled: boolean;
  autoFocus: boolean;
  children: ReactNode;
}) {
  return (
    <button ref={buttonRef} className={`btn small ${danger ? "danger" : "slime"}`} disabled={disabled} autoFocus={autoFocus}>
      {children}
    </button>
  );
}

// ---------------------------------------------------------------------------------------------
// One item in full

function InboxDetail({
  box,
  item,
  order,
  waiting,
  onNotice,
  onGone,
}: {
  box: BoxKey;
  item: string;
  order: Order;
  waiting: Waiting;
  onNotice: (text: string | undefined) => void;
  onGone: (title: string) => void;
}) {
  const row = rowsFor(box, waiting.data).find((r) => r.key === item);
  const seen = useRef<string>(undefined);
  const heading = useRef<HTMLHeadingElement>(null);
  const back = inboxUrl(box, undefined, order);

  useEffect(() => {
    if (row) {
      if (!seen.current) heading.current?.focus();
      seen.current = row.title;
    } else if (seen.current) {
      // Decided: back to the list, which says so.
      onGone(seen.current);
    }
  }, [row?.key]);

  if (!row) {
    if (seen.current) return <Loading />;
    return (
      <div className="inbox-detail">
        <p>
          <Link to={back}>← {BOX[box].label}</Link>
        </p>
        <div className="empty-state">This isn't waiting any more: someone may have decided it already.</div>
      </div>
    );
  }
  return (
    <div className="inbox-detail">
      <p>
        <Link to={back}>← {BOX[box].label}</Link>
      </p>
      <h2 className="section-title" ref={heading} tabIndex={-1}>
        {row.title}
      </h2>
      <ItemCard box={box} item={item} waiting={waiting} onNotice={onNotice} />
    </div>
  );
}

function ItemCard({
  box,
  item,
  waiting,
  onNotice,
}: {
  box: BoxKey;
  item: string;
  waiting: Waiting;
  onNotice: (text: string | undefined) => void;
}) {
  const q = waiting.data.queue!;
  const done = waiting.reload;
  const list = (card: ReactNode) => <ul className="release-list">{card}</ul>;
  switch (box) {
    case "releases": {
      const r = q.releases.find((x) => x.id === item);
      return r ? list(<ReleaseCard r={r} onDone={done} />) : null;
    }
    case "changes": {
      const c = q.changes.find((x) => x.id === item);
      return c ? list(<ChangeCard c={c} onDone={done} />) : null;
    }
    case "pictures": {
      const m = q.media.find((x) => x.id === item);
      return m ? list(<MediaCard m={m} onDone={done} />) : null;
    }
    case "listing": {
      const a = q.ready_to_list.find((x) => x.slug === item);
      return a ? list(<ReadyCard a={a} onDone={done} />) : null;
    }
    case "rescans": {
      const f = q.rescans.find((x) => `${x.release_id}:${x.platform}` === item);
      return f ? list(<RescanCard f={f} onDone={done} />) : null;
    }
    case "people": {
      const p = q.portfolios.find((x) => x.set.id === item);
      return p ? list(<PortfolioCard p={p} onDone={done} />) : null;
    }
    case "problems": {
      const p = q.problems.find((x) => x.id === item);
      return p ? list(<ProblemCard p={p} onDone={done} />) : null;
    }
    case "disputes": {
      const d = q.review_disputes.find((x) => x.job_id === item);
      return d ? list(<DisputeCard d={d} onDone={done} />) : null;
    }
    case "reports": {
      const r = q.reports.find((x) => x.id === item);
      return r ? list(<ReportCard r={r} rules={q.rules} onDone={done} />) : null;
    }
    case "appeals": {
      const a = q.appeals.find((x) => x.id === item);
      return a ? list(<AppealCard a={a} onDone={done} onNotice={onNotice} />) : null;
    }
    case "abuse": {
      const a = q.abuse.find((x) => x.id === item);
      return a ? list(<AbuseCard a={a} onDone={done} />) : null;
    }
    case "legal": {
      const r = waiting.data.legal?.find((x) => x.id === item);
      return r ? list(<LegalRequestCard r={r} onAnswered={done} />) : null;
    }
    case "approvals": {
      const a = waiting.data.actions?.actions.find((x) => x.id === item);
      return a
        ? list(<StaffActionCard a={a} approvalsRequired={waiting.data.actions!.approvals_required} onDone={done} />)
        : null;
    }
    case "donations": {
      const h = waiting.data.holds?.find((x) => x.id === item);
      return h ? <HoldCard hold={h} showApp onDone={done} /> : null;
    }
    case "ai":
      return (
        <p>
          <Link to={`/admin?tab=ai-messages&thread=${encodeURIComponent(item)}`}>Read the thread</Link>
        </p>
      );
  }
}
