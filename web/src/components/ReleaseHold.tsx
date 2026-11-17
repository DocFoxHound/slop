// SPDX-License-Identifier: AGPL-3.0-or-later
// A release on hold, or one whose checks failed or SLAPorSLOP staff turned down, for its maker
// (crates/slop-api/src/holds.rs): what holds it in plain words (with what the scanner found, so it
// can be fixed), running the scans again, the conversation with SLAPorSLOP staff, withdrawing it,
// and what else can be done. Everything it tells the maker to do can be done here or on this page
// (owner rule: no dead-end steps).

import { useEffect, useId, useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { Link } from "react-router";

import {
  api,
  ApiError,
  type ContentLabel,
  type HoldKind,
  type HoldLineDetail,
  type HoldProblem,
  type HoldStatus,
  type MakerHold,
  type MakerHoldMessage,
  type Release,
} from "../api";
import { CONTENT_LABEL, formatDate, formatDateTime } from "../format";
import { useConfirmStep, useInterval, useLoad } from "../hooks";

/** Text from the server with names and paths in `backticks`, shown as code; never as HTML. */
export function PlainText({ text }: { text: string }) {
  const parts = text.split("`");
  return (
    <>
      {parts.map((p, i) => (i % 2 === 1 ? <code key={i}>{p}</code> : p))}
    </>
  );
}

/** "16+", or "Everyone" for 0. */
export const ageLabel = (age: number) => (age === 0 ? "Everyone" : `${age}+`);

/** Content labels in words. */
export const labelList = (labels: ContentLabel[]) => labels.map((l) => CONTENT_LABEL[l] ?? l).join(", ");

/** How a hold ended, for its maker. Exported for tests. */
export const HOLD_ENDED: Record<Exclude<HoldStatus, "open">, string> = {
  passed: "The hold ended: it passed SLAPorSLOP's checks.",
  cleared: "The hold ended: SLAPorSLOP staff accepted it.",
  rejected: "The hold ended: SLAPorSLOP staff turned it down.",
  replaced: "The hold ended: a newer version replaced it.",
  withdrawn: "The hold ended: the release was withdrawn.",
  answered: "SLAPorSLOP staff answered your question. You can ask again below.",
};

/** One line of a hold's conversation, from either side. */
export interface LogLine {
  key: string;
  /** "You", "SLAPorSLOP reviewer", "SLAPorSLOP". */
  who: string;
  /** Written by the viewer's side (shown on the right). */
  mine: boolean;
  body: string;
  /** A re-scan's verdicts, in words. */
  facts?: string;
  at: string;
}

/** A SLAPorSLOP line's facts in words: "Windows: on hold → passed". */
export function lineFacts(d: HoldLineDetail | undefined): string | undefined {
  if (!d || (!d.before && !d.after)) return undefined;
  const verdict = (v?: string) => (v ?? "not known").replace(/_/g, " ");
  return `${d.platform ? `${d.platform}: ` : ""}${verdict(d.before)} → ${verdict(d.after)}`;
}

/** The maker's view of a line. Exported for tests. */
export function makerLine(m: MakerHoldMessage, i: number): LogLine {
  return {
    key: `${m.at}-${i}`,
    who: m.from === "you" ? (m.by_agent ? "Your AI agent" : "You") : m.from === "staff" ? (m.staff_as ?? "SLAPorSLOP staff") : "SLAPorSLOP",
    mine: m.from === "you",
    body: m.body,
    facts: lineFacts(m.detail),
    at: m.at,
  };
}

/** "On hold: …" and "On hold: ….", compared as the same words. */
const sameWords = (a: string, b: string) => a.trim().replace(/\.$/, "") === b.trim().replace(/\.$/, "");

/**
 * A hold's lines without SLAPorSLOP's opening one when it only repeats the reason shown above it
 * (the server posts the release's `status_reason` as the hold's first line). Exported for tests.
 */
export function withoutRepeatedReason<M extends { from: string; body?: string; text?: string }>(
  messages: M[],
  reason: string | null | undefined,
): M[] {
  if (!reason) return messages;
  const first = messages.findIndex((m) => m.from === "slaporslop");
  if (first < 0 || !sameWords(messages[first].body ?? messages[first].text ?? "", reason)) return messages;
  return messages.filter((_, i) => i !== first);
}

/**
 * "Send a new version": the form at the top of the Releases page. Moves focus to its first field
 * (or the form itself), so keyboard and screen reader users land there too.
 */
export function focusNewRelease() {
  const target = document.getElementById("new-release");
  if (!target) return;
  const field = target.querySelector<HTMLElement>("input:not([type=hidden]):not([disabled]), select:not([disabled]), textarea:not([disabled])");
  (field ?? target).focus({ preventScroll: true });
}

function NewReleaseLink() {
  const go = (e: MouseEvent<HTMLAnchorElement>) => {
    e.preventDefault();
    document.getElementById("new-release")?.scrollIntoView({ block: "start" });
    focusNewRelease();
  };
  return (
    <a href="#new-release" onClick={go}>
      Send a new version
    </a>
  );
}

/** A hold's conversation: plain text, oldest first. */
export function HoldLog({ lines, label }: { lines: LogLine[]; label: string }) {
  if (lines.length === 0) return null;
  return (
    <ol className="messages hold-log" aria-label={label}>
      {lines.map((l) => (
        <li key={l.key} className={l.mine ? "mine" : undefined}>
          <div className="small">
            <b>{l.who}</b> <span className="muted">· {formatDateTime(l.at)}</span>
          </div>
          <p className="pre">{l.body}</p>
          {l.facts && <p className="muted small">{l.facts}</p>}
        </li>
      ))}
    </ol>
  );
}

/** The guide's pointer is a link here, not text. */
const GUIDE_POINTER = / ?\(see \/publish\/guide#slop-check\)/;

function Problem({ p }: { p: HoldProblem }) {
  return (
    <li>
      <b>{p.what}</b>
      <p className="small">
        <PlainText text={p.explained.replace(GUIDE_POINTER, "")} />
      </p>
      {/* Slop Check runs on GitHub Actions: offered only where it can run. */}
      {p.kind === "defender_missing" && p.you_can.includes("run_slop_check") && (
        <p className="small">
          <Link to="/publish/guide#slop-check">How Slop Check works</Link>.{" "}
          <a href="#slop-check" onClick={openSlopCheck}>
            Add Slop Check to your repository
          </a>{" "}
          from this page.
        </p>
      )}
    </li>
  );
}

/** Opens the "Building somewhere else?" part of Slop Build's panel, where Slop Check is added. */
function openSlopCheck() {
  const details = document.getElementById("slop-check");
  if (details instanceof HTMLDetailsElement) details.open = true;
}

/**
 * What the maker sees about running the scans again: why it can or can't be now. `scanning`: this
 * server's scanner is on (GET /v1/server); unknown counts as on. Exported for tests.
 */
export function rescanLine(
  h: Pick<MakerHold, "rescans_left" | "next_rescan_at" | "rescan_requested" | "rescan_running" | "you_can">,
  scanning = true,
): string {
  if (h.rescan_requested) {
    return h.rescan_running
      ? "SLAPorSLOP is scanning the held files again now. You'll be told the result."
      : "SLAPorSLOP will scan the held files again shortly. You'll be told the result.";
  }
  if (h.rescans_left <= 0) return "No re-runs left: SLAPorSLOP runs the scans again up to 3 times per release. Write to SLAPorSLOP staff, or send a new version.";
  const left = `${h.rescans_left} of 3 re-runs left.`;
  if (h.next_rescan_at) return `${left} The next is allowed from ${formatDateTime(h.next_rescan_at)}.`;
  if (!h.you_can.includes("rescan")) {
    return scanning ? "Scanning again won't help with this hold." : "SLAPorSLOP's scanner is off right now, so it can't scan again.";
  }
  return left;
}

/** A hold's conversation, or a question about an ended release: how the maker's box reads. */
type Writing = "ask" | "flagged" | "rating" | "other";

/** Which box to show, from what holds it. Exported for tests. */
export function writingFor(kinds: HoldKind[], asking: boolean): Writing {
  if (asking) return "ask";
  if (kinds.includes("scan") || kinds.includes("defender")) return "flagged";
  if (kinds.includes("rating_lowered")) return "rating";
  return "other";
}

const WRITING: Record<Writing, { placeholder: string; hint: string }> = {
  ask: {
    placeholder: "What you'd like them to look at again, or what you don't understand",
    hint: "A person at SLAPorSLOP reads it and answers here. ",
  },
  flagged: {
    placeholder: "Why it's a false positive, or what you'd like to know",
    hint: "Say what the flagged file is and why it's fine (a packer, a game engine's runtime). ",
  },
  rating: {
    placeholder: "What changed, so the lower rating fits",
    hint: "Say what changed so the lower rating fits (content removed or toned down). ",
  },
  other: {
    placeholder: "What you'd like SLAPorSLOP staff to know",
    hint: "A person at SLAPorSLOP reads it and answers here. ",
  },
};

/** The line before "Send a new version", from what holds it. Exported for tests. */
export function newVersionLine(kinds: HoldKind[], ended: boolean): { before: string; after: string } {
  if (ended) return { before: "Fixed it?", after: "." };
  const after = ": once it passes its checks, this one is withdrawn.";
  if (kinds.includes("scan") || kinds.includes("defender")) return { before: "Fixing the build instead?", after };
  if (kinds.includes("rating_lowered")) return { before: "Changing it back instead?", after };
  return { before: "Replacing it instead?", after };
}

/**
 * A release's hold (status awaiting_review), or a failed or turned-down release its maker asks
 * SLAPorSLOP staff about. `release` is the maker's own (with its hold summary); the hold itself is
 * loaded here and again whenever that summary changes. `scanning`: this server's scanner is on.
 */
export function ReleaseHold({
  slug,
  release,
  scanning,
  onChanged,
}: {
  slug: string;
  release: Release;
  scanning?: boolean;
  /** After something changed (a message, a re-run, a withdrawal): the page looks at its releases again. */
  onChanged: () => void;
}) {
  const hold = useLoad(
    () => api.releaseHold(slug, release.id),
    [slug, release.id, release.status, JSON.stringify(release.hold ?? null)],
  );
  // What an action answered, until the hold is loaded again.
  const [fresh, setFresh] = useState<MakerHold>();
  useEffect(() => setFresh(undefined), [hold.data]);
  const h = fresh ?? hold.data;
  // A re-run under way: look again now and then, so its result shows without reloading.
  useInterval(hold.reload, h?.rescan_requested ? 30_000 : null);
  const titleId = useId();

  if (hold.error) {
    return (
      <p className="form-error" role="alert">
        {hold.error.message}{" "}
        <button type="button" className="link-button" onClick={hold.reload}>
          Try again
        </button>
      </p>
    );
  }
  if (!h) return <p className="muted small">Loading what holds it…</p>;

  // Failed, or turned down by staff: the maker can only ask about it.
  const asking = h.release_status === "failed" || h.release_status === "rejected";
  const open = h.status === "open";
  const question = h.kinds.includes("question");
  const changed = (next: MakerHold) => {
    setFresh(next);
    onChanged();
  };
  const ended = h.status && h.status !== "open" ? HOLD_ENDED[h.status] : undefined;
  const title = asking ? "Ask SLAPorSLOP staff" : open ? "Why it's on hold" : "Its hold";
  const next = newVersionLine(h.kinds, asking);
  // The release's reason is on its card (or, on hold, said by what holds it): not again here.
  const messages = withoutRepeatedReason(h.messages, release.status_reason);

  return (
    <section className="release-hold" aria-labelledby={titleId}>
      <h3 id={titleId}>{title}</h3>
      {open && !asking && (
        <p className="small" role="status">
          {h.needs_staff
            ? h.unanswered
              ? `You wrote to SLAPorSLOP staff${h.asked_staff_at ? ` on ${formatDate(h.asked_staff_at)}` : ""}; they haven't answered yet. You'll get a notice when they do.`
              : "SLAPorSLOP staff will look at it. You'll get a notice when they decide or answer."
            : "Nothing for SLAPorSLOP staff to do yet: it goes on by itself as soon as Microsoft Defender's check arrives. If something's wrong, write to them below."}
        </p>
      )}
      {asking && question && open && (
        <p className="small" role="status">
          You asked SLAPorSLOP staff about it. You'll get a notice when they answer.
        </p>
      )}
      {ended && !(asking && open) && <p className="small">{ended}</p>}
      {/* A failed or turned-down release's reason is on its card already. */}
      {!asking && h.problems.length > 0 && (
        <ul className="hold-problems">
          {h.problems.map((p, i) => (
            <Problem key={`${p.kind}-${p.platform ?? i}`} p={p} />
          ))}
        </ul>
      )}
      {h.rating && open && (
        <dl className="kv small">
          <dt>Age rating</dt>
          <dd>
            {ageLabel(h.rating.from_age)} → {ageLabel(h.rating.to_age)}
          </dd>
          {h.rating.dropped_labels.length > 0 && (
            <>
              <dt>Content labels it drops</dt>
              <dd>{labelList(h.rating.dropped_labels)}</dd>
            </>
          )}
        </dl>
      )}
      {open && h.kinds.includes("scan") && <Rescan slug={slug} hold={h} scanning={scanning} onChanged={changed} />}
      {h.you_can.includes("new_release") && (
        <p className="small">
          {next.before} <NewReleaseLink />
          {next.after}
          {h.kinds.includes("rating_lowered") && open && " A new version with the rating of the last published one needs no check by staff."}
        </p>
      )}
      <HoldLog lines={messages.map(makerLine)} label={`Conversation with SLAPorSLOP staff about version ${h.version}`} />
      {h.you_can.includes("write_to_staff") && (
        <WriteToStaff
          slug={slug}
          releaseId={release.id}
          version={h.version}
          writing={writingFor(h.kinds, asking)}
          onSent={changed}
        />
      )}
      {open && !asking && h.you_can.includes("withdraw") && (
        <Withdraw slug={slug} releaseId={release.id} version={h.version} onDone={onChanged} />
      )}
    </section>
  );
}

/** "Scan again": SLAPorSLOP downloads the same files and scans them with today's signatures. */
function Rescan({
  slug,
  hold: h,
  scanning,
  onChanged,
}: {
  slug: string;
  hold: MakerHold;
  scanning?: boolean;
  onChanged: (h: MakerHold) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const can = h.you_can.includes("rescan") && !h.next_rescan_at && !h.rescan_requested && h.rescans_left > 0;
  const lineId = useId();
  const run = async () => {
    setBusy(true);
    setError(undefined);
    try {
      onChanged(await api.rescanRelease(slug, h.release_id));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  // What Scan again does, wherever it is (or will be) offered.
  const explain = h.you_can.includes("rescan") || Boolean(h.next_rescan_at) || h.rescan_requested;
  return (
    <div className="hold-rescan">
      {explain && (
        <p className="muted small">
          Scanners sometimes match harmless programs, and their signatures change daily. Scan again downloads the same files
          from your release page, checks they haven't changed, and scans them with today's signatures. It can't run Microsoft
          Defender again.
        </p>
      )}
      <div className="row-actions">
        {can && (
          <button type="button" className="btn small" disabled={busy} aria-describedby={lineId} onClick={() => void run()}>
            {busy ? "Asking…" : "Scan again"}
          </button>
        )}
        <span id={lineId} className="small" role="status">
          {rescanLine(h, scanning !== false)}
        </span>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/** "Write to SLAPorSLOP staff" on a hold, or "Ask SLAPorSLOP staff" about a failed or turned-down release. */
function WriteToStaff({
  slug,
  releaseId,
  version,
  writing,
  onSent,
}: {
  slug: string;
  releaseId: string;
  version: string;
  /** What the box asks for: a question about an ended release, or why a hold is wrong. */
  writing: Writing;
  onSent: (h: MakerHold) => void;
}) {
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError>();
  const [sent, setSent] = useState(false);
  const hintId = useId();
  const labelId = useId();
  const errorId = useId();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!body.trim()) return;
    setBusy(true);
    setError(undefined);
    setSent(false);
    try {
      const next = await api.writeToStaff(slug, releaseId, body.trim());
      setBody("");
      setSent(true);
      onSent(next);
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };
  const asking = writing === "ask";
  const label = asking ? "Ask SLAPorSLOP staff" : "Write to SLAPorSLOP staff";
  const bodyError = error?.field === "body";
  return (
    <form className="form hold-write" onSubmit={(e) => void submit(e)} aria-label={`${label} about version ${version}`}>
      <label>
        <span id={labelId}>{asking ? "Your question for SLAPorSLOP staff" : "Your message to SLAPorSLOP staff"}</span>
        <textarea
          value={body}
          rows={3}
          maxLength={2000}
          required
          aria-labelledby={labelId}
          aria-describedby={bodyError ? `${hintId} ${errorId}` : hintId}
          aria-invalid={bodyError || undefined}
          placeholder={WRITING[writing].placeholder}
          onChange={(e) => setBody(e.target.value)}
        />
        <span id={hintId} className="hint">
          {WRITING[writing].hint}
          Plain text, up to 2,000 characters and 3 links; 5 messages a day per release.
        </span>
        {bodyError && (
          <span id={errorId} className="field-error" role="alert">
            {error.message}
          </span>
        )}
      </label>
      {error && !bodyError && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {sent && (
        <p className="notice" role="status">
          Sent. SLAPorSLOP staff are told, and you'll get a notice when they answer.
        </p>
      )}
      <div className="row-actions">
        <button className="btn small slime" disabled={busy || !body.trim()}>
          {busy ? "Sending…" : label}
        </button>
      </div>
    </form>
  );
}

/** Withdrawing a release on hold, after asking: it's never published (the maker's own call). */
function Withdraw({ slug, releaseId, version, onDone }: { slug: string; releaseId: string; version: string; onDone: () => void }) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const { opener, close } = useConfirmStep(confirming, setConfirming);
  const withdraw = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await api.withdrawRelease(slug, releaseId);
      setConfirming(false);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="hold-withdraw">
      {!confirming ? (
        <div className="row-actions">
          <button ref={opener} type="button" className="btn small ghost" onClick={() => setConfirming(true)}>
            Withdraw version {version}…
          </button>
        </div>
      ) : (
        <div className="stage-confirm" role="group" aria-label={`Withdraw version ${version}`}>
          <b>Withdraw version {version}?</b>
          <p className="small">It won't be published or reach players. You can send a new version at any time.</p>
          <div className="row-actions">
            <button type="button" className="btn small danger" autoFocus disabled={busy} onClick={() => void withdraw()}>
              {busy ? "Withdrawing…" : "Yes, withdraw it"}
            </button>
            <button type="button" className="btn small ghost" disabled={busy} onClick={close}>
              Not yet
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * A failed release, or one SLAPorSLOP staff turned down: "Ask SLAPorSLOP staff" opens the
 * conversation (a person looks at a refusal again on request). Shown open once there's a
 * conversation already.
 */
export function AskAboutFailed({
  slug,
  release,
  scanning,
  onChanged,
}: {
  slug: string;
  release: Release;
  scanning?: boolean;
  onChanged: () => void;
}) {
  // Open once there's a question: a failed release's hold is one; a turned-down release's past
  // hold (what held it) folds away on its own until its maker asks.
  const [open, setOpen] = useState(Boolean(release.hold && (release.status === "failed" || release.hold.kinds.includes("question"))));
  if (open) return <ReleaseHold slug={slug} release={release} scanning={scanning} onChanged={onChanged} />;
  return (
    <p className="small">
      {release.status === "rejected" ? "Want SLAPorSLOP staff to explain, or look again?" : "Think the checks got it wrong?"}{" "}
      <button type="button" className="btn small" onClick={() => setOpen(true)}>
        Ask SLAPorSLOP staff
      </button>
    </p>
  );
}

/** A hold that ended, folded away: what held it and the conversation, loaded when opened. */
export function PastHold({ slug, release, summary }: { slug: string; release: Release; summary: ReactNode }) {
  const [seen, setSeen] = useState(false);
  return (
    <details className="small past-hold" onToggle={(e) => (e.currentTarget.open ? setSeen(true) : undefined)}>
      <summary>{summary}</summary>
      {seen && <ReleaseHold slug={slug} release={release} onChanged={() => undefined} />}
    </details>
  );
}
