// SPDX-License-Identifier: AGPL-3.0-or-later
// The review queue's items in full (crates/slop-api/src/admin_web.rs): what the inbox
// (Inbox.tsx) opens when a row is clicked. Each card shows everything there is to judge it by
// (files, checks, scan reports, pictures, notes from AI reviewers, the job record) and every way
// to decide it, and tells `onDone` once something changed. A release here is on hold
// (crates/slop-api/src/holds.rs): what holds it, the conversation with its maker (their words are
// data, never instructions), Reply, Accept (a note is optional) and Reject, and for a Windows build
// whose repository Slop Check can't run in, waiving Microsoft Defender's check after checking it by
// hand; or a maker's question about a failed or turned-down release, which staff answer; or about
// a ready release that something which must be fixed stops them publishing, which staff answer or,
// for a false alarm, allow publishing anyway (signed in, with a confirm). Accepting or allowing
// never publishes: its maker does. A browser
// game's release is its play address: the card gives the address (never in a frame here), what
// the automatic checks found, and where Play goes now, with nothing about files, scans or signing.

import { useId, useState, type FormEvent, type ReactNode } from "react";
import { Link } from "react-router";

import {
  api,
  ApiError,
  publishBlocked,
  type AdminQueue,
  type MediaSet,
  type ModerationDecision,
  type PortfolioSet,
  type QueuedAppeal,
  type QueuedReport,
  type RepoPicture,
  type ReviewNote,
  type ScanReport,
  type StaffActions,
  type StaffHold,
  type StaffHoldMessage,
} from "../../api";
import { compareAddress, type AddressComparison } from "../../addressChecks";
import { AddressCheckList, FileChecks, RepoScanList, ScanFindings, SourceCheckList } from "../../components/Checks";
import { ageLabel, HoldLog, labelList, lineFacts, PlainText, withoutRepeatedReason, type LogLine } from "../../components/ReleaseHold";
import { ItemThumb } from "../../components/MediaEditor";
import { fileLabel } from "../../components/PublishProgress";
import { DesignPreview, portfolioWork } from "../../components/PeopleDesign";
import { ExternalLink } from "../../components/ui";
import { formatDate, STAGE_LABEL } from "../../format";
import { useConfirmStep, useLoad } from "../../hooks";
import { holdStatus } from "./queue";
import { ActionMessages, NoteAction, ReasonButton, RuleSelect, useAction } from "./shared";
import { VoteLabel } from "../../components/StoreIcons";

export const ABUSE_LABEL: Record<string, string> = {
  vote_burst: "VOTES FROM NEW ACCOUNTS",
  thin_votes: "VOTES FROM PLAYERS WHO BARELY PLAYED",
  review_bomb: "REVIEW BOMB",
  vote_swing: "VOTE SWING",
  lookalike_name: "LOOKALIKE NAME",
  rate_limited: "HITTING RATE LIMITS",
  malicious_dependency: "MALICIOUS DEPENDENCY",
  bad_link: "LINK TO A KNOWN-BAD ADDRESS",
};

type Q = AdminQueue;

/** Staff's view of a line: the maker's words are marked as theirs. Exported for tests. */
export function staffLine(m: StaffHoldMessage): LogLine {
  const maker = m.from === "maker";
  return {
    key: m.id,
    who:
      m.from === "slaporslop"
        ? "SLAPorSLOP"
        : maker
          ? `${m.author ?? "The maker"}${m.by_agent ? " (their AI agent)" : ""}, the maker`
          : `${m.author ?? "SLAPorSLOP staff"}${m.by_agent ? " (an AI reviewer)" : ""}`,
    mine: m.from === "staff",
    body: (maker ? m.untrusted_text : m.text) ?? "",
    facts: lineFacts(m.detail),
    at: m.at,
  };
}

/** What holds a release, the rating it lowers, and the conversation with its maker, with Reply. */
function HoldPanel({ r, hold, onDone }: { r: Q["releases"][number]; hold: StaffHold; onDone: () => void }) {
  const question = hold.kinds.includes("question");
  return (
    <div className="release-hold">
      {question && r.status === "ready" ? (
        <>
          <p className="small">
            <b>The maker asks about what must be fixed in this release.</b> It passed its checks, but they can't publish it while
            it has these; they think one is a false alarm. Answer below (that closes the question), or, if it's a false alarm,
            allow publishing it anyway.
          </p>
          {hold.problems.length > 0 && (
            <ul className="hold-problems">
              {hold.problems.map((p, i) => (
                <li key={`${p.kind}-${i}`}>
                  <span className="badge bad">MUST FIX</span> <b>{p.what}</b>
                  <p className="small">
                    <PlainText text={p.explained} />
                  </p>
                </li>
              ))}
            </ul>
          )}
        </>
      ) : question ? (
        r.status === "rejected" ? (
          <p className="small">
            <b>The maker asks about this release SLAPorSLOP staff turned down.</b> The reason given: {r.status_reason ?? "no reason recorded"}.
            Answer below; answering closes the question (they can ask again).
          </p>
        ) : (
          <p className="small">
            <b>The maker asks about this failed release.</b> Its checks said: {r.status_reason ?? "no reason recorded"}. Answer
            below; answering closes the question (they can ask again).
          </p>
        )
      ) : (
        hold.problems.length > 0 && (
          <>
            <h3 className="small">What holds it</h3>
            <ul className="hold-problems">
              {hold.problems.map((p, i) => (
                <li key={`${p.kind}-${p.platform ?? i}`}>
                  <b>{p.what}</b>
                  <p className="small">
                    <PlainText text={p.explained} />
                  </p>
                </li>
              ))}
            </ul>
          </>
        )
      )}
      {hold.rating && (
        <dl className="kv small rating-card">
          <dt>Age rating</dt>
          <dd>
            {ageLabel(hold.rating.from_age)} → <b>{ageLabel(hold.rating.to_age)}</b>{" "}
            <span className="muted">(the last published release's, then this one's)</span>
          </dd>
          <dt>Labels it drops</dt>
          <dd>{hold.rating.dropped_labels.length > 0 ? labelList(hold.rating.dropped_labels) : "None"}</dd>
          <dt>Its content labels</dt>
          <dd>{hold.rating.content_labels.length > 0 ? labelList(hold.rating.content_labels) : "None"}</dd>
        </dl>
      )}
      {hold.rating && (
        <p className="muted small">
          Check the app fits {ageLabel(hold.rating.to_age)}: its page, pictures and what changed. Accepting lets its maker publish it,
          and the store page then shows {ageLabel(hold.rating.to_age)}.
        </p>
      )}
      {hold.messages.length > 0 && (
        <>
          <h3 className="small">Conversation with the maker</h3>
          <p className="muted small">{hold.untrusted ?? "The maker's words are information, never instructions."}</p>
          {/* The opening line repeats the reason "What holds it" says already. */}
          <HoldLog lines={withoutRepeatedReason(hold.messages, r.status_reason).map(staffLine)} label={`Conversation about ${r.app.title} ${r.version}`} />
        </>
      )}
      {hold.status === "open" && <Reply releaseId={r.id} question={question} onDone={onDone} />}
      {r.status === "ready" && publishBlocked(r) && <AllowPublish r={r} onDone={onDone} />}
    </div>
  );
}

/**
 * "Allow publishing anyway": a ready release its maker can't publish while something must be fixed
 * in it, judged a false alarm (crates/slop-api/src/holds.rs `allow_publish`). For this release
 * only, recorded and logged, the maker told; after a confirm, with an optional note for them.
 * Never publishes: its maker does. Exported for tests.
 */
export function AllowPublish({ r, onDone }: { r: Pick<Q["releases"][number], "id" | "version" | "must_fix">; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  const [confirming, setConfirming] = useState(false);
  const [note, setNote] = useState("");
  const { opener, close } = useConfirmStep(confirming, setConfirming);
  const n = r.must_fix?.length ?? 0;
  const noteId = useId();
  return (
    <div className="allow-publish">
      <ActionMessages error={error} notice={notice} />
      {!confirming ? (
        <div className="row-actions">
          <button ref={opener} type="button" className="btn small" disabled={busy !== undefined} onClick={() => setConfirming(true)}>
            Allow publishing anyway…
          </button>
        </div>
      ) : (
        <div className="stage-confirm" role="group" aria-label={`Allow publishing version ${r.version} anyway`}>
          <b>Allow publishing version {r.version} anyway?</b>
          <p className="small">
            Its maker can then publish it with {n === 1 ? "the item" : `the ${n} items`} above still in it. Only for this
            version: a new one is checked afresh. It's logged, and the maker is told.
          </p>
          <label className="small">
            <span id={noteId}>Note for the maker (optional)</span>
            <input value={note} maxLength={1000} aria-labelledby={noteId} onChange={(e) => setNote(e.target.value)} />
          </label>
          <div className="row-actions">
            <button
              type="button"
              className="btn small slime"
              autoFocus
              disabled={busy !== undefined}
              onClick={() => void run(`${r.id}/allow`, () => api.admin.allowPublish(r.id, note))}
            >
              {busy ? "Allowing…" : "Yes, allow publishing"}
            </button>
            <button type="button" className="btn small ghost" disabled={busy !== undefined} onClick={close}>
              Not yet
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/** A reply in a hold's conversation: the maker gets a notice. A question's answer closes it. */
function Reply({ releaseId, question, onDone }: { releaseId: string; question: boolean; onDone: () => void }) {
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [sent, setSent] = useState(false);
  const id = useId();
  const send = async (e: FormEvent) => {
    e.preventDefault();
    if (!body.trim()) return;
    setBusy(true);
    setError(undefined);
    setSent(false);
    try {
      await api.admin.answerHold(releaseId, body.trim());
      setBody("");
      setSent(true);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const label = question ? "Answer the maker" : "Reply to the maker";
  return (
    <form className="form hold-write" onSubmit={(e) => void send(e)} aria-labelledby={id}>
      <label>
        <span id={id}>{label}</span>
        <textarea
          value={body}
          rows={3}
          maxLength={4000}
          required
          aria-labelledby={id}
          aria-describedby={`${id}-hint`}
          onChange={(e) => setBody(e.target.value)}
        />
        <span id={`${id}-hint`} className="hint">
          {question ? "Answering closes the question; the maker" : "The maker"} reads it on their Releases page and gets a notice.
        </span>
      </label>
      <ActionMessages error={error} notice={sent ? "Sent to the maker." : undefined} />
      <div className="row-actions">
        <button className="btn small" disabled={busy || !body.trim()}>
          {busy ? "Sending…" : question ? "Answer" : "Reply"}
        </button>
      </div>
    </form>
  );
}

/** Accept (a note is optional) or Reject (a reason is required) a release on hold; a question
 *  about a failed release is only answered. */
function HoldDecision({ r, onDone }: { r: Q["releases"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  if (r.hold.kinds.includes("question") || r.status !== "awaiting_review") return null;
  // Only a missing Defender check holds it: Accept can't stand in for that check.
  const missingOnly = missingCheckOnly(r.hold.kinds);
  return (
    <>
      <ActionMessages error={error} notice={notice} />
      <p className="muted small">
        {missingOnly
          ? waivable(r.hold).length > 0
            ? "Only Microsoft Defender's missing check holds it. Check the Windows build by hand, then waive the check on its file above; it then waits for its maker to publish it. Or reject it."
            : "Only Microsoft Defender's missing check holds it: it goes on by itself once its maker runs Slop Check. Reply to them, or reject it."
          : `Accept clears what holds it${r.hold.rating ? " (and accepts the lower rating)" : " as a false positive"}; it then waits for its maker to publish it. Nothing reaches players until they do.`}
      </p>
      <div className="row-actions">
        {!missingOnly && (
          <NoteAction
            label={busy === r.id ? "Accepting…" : "Accept"}
            primary
            disabled={busy !== undefined}
            onSubmit={(note) => void run(r.id, () => api.admin.approve(r.id, note))}
          />
        )}
        <ReasonButton
          label="Reject"
          placeholder="Reason, shown to the maker"
          danger
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(r.id, () => api.admin.reject(r.id, reason))}
        />
      </div>
    </>
  );
}

/** Only a missing Microsoft Defender check holds it. Exported for tests. */
export const missingCheckOnly = (kinds: string[]) => kinds.length > 0 && kinds.every((k) => k === "defender_missing");

/**
 * The platforms whose missing Defender check staff can waive: the hold is open on one, and Slop
 * Check can't run for its repository (its problem offers the maker no Slop Check). Exported for tests.
 */
export function waivable(hold: Pick<StaffHold, "status" | "problems">): string[] {
  if (hold.status !== "open") return [];
  return hold.problems
    .filter((p) => p.kind === "defender_missing" && p.platform && !p.you_can.includes("run_slop_check"))
    .map((p) => p.platform as string);
}

/** A release's badge: failed, turned down, or on hold. */
function statusBadge(status: string): { label: string; tone: string } {
  if (status === "failed") return { label: "FAILED", tone: "bad" };
  if (status === "rejected") return { label: "REJECTED", tone: "bad" };
  if (status === "ready") return { label: "PASSED, MUST FIX FIRST", tone: "warn" };
  return { label: "ON HOLD", tone: "warn" };
}

/** The release's reason, unless the hold says it already: what holds it, or the maker's question. */
const showReason = (r: Q["releases"][number]) => Boolean(r.status_reason) && r.status !== "awaiting_review" && !r.hold.kinds.includes("question");

export function ReleaseCard({ r, onDone }: { r: Q["releases"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  if (r.play_address) return <AddressCard r={r} address={r.play_address} onDone={onDone} />;
  const label = holdStatus(r.hold);
  const badge = statusBadge(r.status);
  const canWaive = waivable(r.hold);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <Link to={`/app/${r.app.slug}`}>
            <b>{r.app.title}</b>
          </Link>
          <span className="muted">
            {r.version} · by {r.app.maker}
          </span>
          <span className={`badge ${badge.tone}`}>{badge.label}</span>
          <span className={`badge ${label.tone}`}>{label.label.toUpperCase()}</span>
          {r.hold.unanswered && <span className="badge soon">MAKER WROTE</span>}
        </div>
        <span className="muted small">{formatDate(r.created_at)}</span>
      </div>
      {showReason(r) && <p className="notice">{r.status_reason}</p>}
      <HoldPanel r={r} hold={r.hold} onDone={onDone} />
      <dl className="kv">
        <dt>Repository</dt>
        <dd>
          <ExternalLink href={r.app.repo_url}>{r.app.repo_url.replace(/^https:\/\//, "")}</ExternalLink>
        </dd>
        <dt>Commit</dt>
        <dd>
          <ExternalLink href={`${r.app.repo_url}/commit/${r.commit_sha}`}>
            <code>{r.commit_sha.slice(0, 12)}</code>
          </ExternalLink>{" "}
          {r.commit_verified ? (
            <span className="badge official">FOUND IN REPO</span>
          ) : (
            <span className="badge warn">NOT CHECKED</span>
          )}
        </dd>
        {r.files.map((f) => (
          <FileRow
            key={f.platform}
            file={f}
            holdKinds={r.hold.kinds}
            canClear={r.status === "awaiting_review" && busy === undefined}
            onClear={(note) => void run(`${r.id}/${f.platform}`, () => api.admin.clearScan(r.id, f.platform, note))}
            waivable={canWaive.includes(f.platform) && !f.defender_waived}
            onWaive={(note) => void run(`${r.id}/${f.platform}/waive`, () => api.admin.waiveDefender(r.id, f.platform, note))}
          />
        ))}
      </dl>
      <ActionMessages error={error} notice={notice} />
      <SourceCheckList checks={r.source_checks} audience="reviewer" />
      <RepoScanList scan={r.repo_scan} advisories={r.advisories} audience="reviewer" />
      <Notes notes={r.notes} />
      <HoldDecision r={r} onDone={onDone} />
    </li>
  );
}

/** How the address under review compares with where Play goes now: never "the same site" for
 *  another address on the same host, which on a shared host can be someone else's game. */
const VS_CURRENT: Record<AddressComparison, ReactNode> = {
  same_address: <span className="muted small">(the same address)</span>,
  same_host: <span className="muted small">(the same host, another address: check it's still this game)</span>,
  new_site: <span className="badge warn">NEW SITE</span>,
};

/** And with the address SLAPorSLOP withdrew, when that's why Play opens nothing now. */
const VS_WITHDRAWN: Record<AddressComparison, ReactNode> = {
  same_address: <span className="badge bad">THE WITHDRAWN ADDRESS</span>,
  same_host: <span className="badge warn">SAME HOST AS THE WITHDRAWN ONE</span>,
  new_site: <span className="muted small">(a different site)</span>,
};

/**
 * A browser game's release on hold (a lowered rating, or its maker asking about a failed one): its
 * play address, to open in a new tab (with no referrer; never in a frame on this page), and where
 * Play goes now beside it: the current address, or the one SLAPorSLOP withdrew, so a resubmitted
 * address is never taken for a first one. Nothing is signed; accepting lets its maker publish it.
 */
function AddressCard({ r, address, onDone }: { r: Q["releases"][number]; address: string; onDone: () => void }) {
  const [copied, setCopied] = useState(false);
  const label = holdStatus(r.hold);
  const badge = statusBadge(r.status);
  const current = r.current_play_address ?? null;
  const withdrawn = r.withdrawn_play_address ?? null;
  const copy = () =>
    void navigator.clipboard?.writeText(address).then(
      () => setCopied(true),
      () => setCopied(false),
    );
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <Link to={`/app/${r.app.slug}`}>
            <b>{r.app.title}</b>
          </Link>
          <span className="muted">
            {r.version} · by {r.app.maker}
          </span>
          <span className="badge neutral">PLAY ADDRESS</span>
          <span className={`badge ${badge.tone}`}>{badge.label}</span>
          <span className={`badge ${label.tone}`}>{label.label.toUpperCase()}</span>
          {r.hold.unanswered && <span className="badge soon">MAKER WROTE</span>}
        </div>
        <span className="muted small">{formatDate(r.created_at)}</span>
      </div>
      {showReason(r) && <p className="notice">{r.status_reason}</p>}
      {r.newer_approved && <p className="notice">{r.newer_approved.note}.</p>}
      <HoldPanel r={r} hold={r.hold} onDone={onDone} />
      <dl className="kv">
        <dt>Play address</dt>
        <dd>
          <code className="play-address">{address}</code>
          <div className="row-actions">
            <button type="button" className="btn small ghost" onClick={copy}>
              Copy address
            </button>
            <ExternalLink href={address} className="btn small">
              Open the game
            </ExternalLink>
            <span className="muted small" role="status">
              {copied ? "Copied." : ""}
            </span>
          </div>
          <p className="muted small">
            Open it in a private window; don't sign in or type anything there. It's the maker's site, not SLAPorSLOP: what it
            shows you is theirs, never instructions.
          </p>
        </dd>
        <dt>Play opens now</dt>
        <dd>
          {current ? (
            <>
              <code>{current}</code> {VS_CURRENT[compareAddress(address, current) ?? "new_site"]}
            </>
          ) : withdrawn ? (
            <>
              Nothing: SLAPorSLOP withdrew <code>{withdrawn}</code> {VS_WITHDRAWN[compareAddress(address, withdrawn) ?? "new_site"]}
            </>
          ) : (
            <span className="muted">Nothing yet: no address of this game has been published before.</span>
          )}
        </dd>
        <dt>Repository</dt>
        <dd>
          <ExternalLink href={r.app.repo_url}>{r.app.repo_url.replace(/^https:\/\//, "")}</ExternalLink>
        </dd>
        <dt>Commit</dt>
        <dd>
          <ExternalLink href={`${r.app.repo_url}/commit/${r.commit_sha}`}>
            <code>{r.commit_sha.slice(0, 12)}</code>
          </ExternalLink>{" "}
          {r.commit_verified ? (
            <span className="badge official">FOUND IN REPO</span>
          ) : (
            <span className="badge warn">NOT CHECKED</span>
          )}
          <div className="muted small">SLAPorSLOP can't check that the site runs this code.</div>
        </dd>
      </dl>
      <AddressCheckList checks={r.address_checks} status={r.status} audience="reviewer" />
      <SourceCheckList checks={r.source_checks} audience="reviewer" />
      <RepoScanList scan={r.repo_scan} advisories={r.advisories} audience="reviewer" browser />
      <Notes notes={r.notes} />
      <HoldDecision r={r} onDone={onDone} />
    </li>
  );
}

export function ChangeCard({ c, onDone }: { c: Q["changes"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <Link to={`/app/${c.app.slug}`}>
            <b>{c.app.title}</b>
          </Link>
          <span className="muted">by {c.app.maker}</span>
          <span className="badge soon">{c.kind === "repo" ? "MOVE REPOSITORY" : "SWITCH RELEASE"}</span>
        </div>
        <span className="muted small">{formatDate(c.created_at)}</span>
      </div>
      <dl className="kv">
        {c.kind === "repo" ? (
          <>
            <dt>From</dt>
            <dd>
              <ExternalLink href={c.app.repo_url}>{c.app.repo_url.replace(/^https:\/\//, "")}</ExternalLink>
            </dd>
            <dt>To</dt>
            <dd>
              <ExternalLink href={c.repo_url ?? ""}>{(c.repo_url ?? "").replace(/^https:\/\//, "")}</ExternalLink>
            </dd>
          </>
        ) : (
          <>
            <dt>Players get</dt>
            <dd>{c.players_get ? `v${c.players_get}` : "nothing yet"}</dd>
            <dt>Change to</dt>
            <dd>{c.release_id ? `v${c.release_version} (held there)` : "the newest published release"}</dd>
          </>
        )}
        {c.note && (
          <>
            <dt>Maker says</dt>
            <dd>{c.note}</dd>
          </>
        )}
      </dl>
      <Notes notes={c.notes} />
      {c.kind === "repo" && (
        <p className="muted small">
          Check the new repository belongs to the same maker and project. Its first release will come back here for
          review.
        </p>
      )}
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <button
          className="btn slime small"
          disabled={busy !== undefined}
          onClick={() => void run(c.id, () => api.admin.approveChange(c.id))}
        >
          {busy === c.id ? "Approving…" : "Approve"}
        </button>
        <ReasonButton
          label="Turn down"
          placeholder="Reason, shown to the maker"
          danger
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(c.id, () => api.admin.rejectChange(c.id, reason))}
        />
      </div>
    </li>
  );
}

export function MediaCard({ m, onDone }: { m: Q["media"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <Link to={`/app/${m.app.slug}`}>
            <b>{m.app.title}</b>
          </Link>
          <span className="muted">by {m.app.maker}</span>
          <span className="badge soon">{m.live ? "NEW PICTURES" : "FIRST PICTURES"}</span>
        </div>
        <span className="muted small">
          <ExternalLink href={`${m.app.repo_url}/tree/${m.commit_sha}/.slop`}>
            <code>{m.commit_sha.slice(0, 7)}</code>
          </ExternalLink>{" "}
          · {formatDate(m.created_at)}
        </span>
      </div>
      {(m.lookalikes ?? []).length > 0 && (
        <div className="notice small" role="note">
          <span className="badge warn">ICON LOOKS LIKE</span>{" "}
          {m.lookalikes!.map((l, i) => (
            <span key={l.slug}>
              {i > 0 && ", "}
              <Link to={`/app/${l.slug}`}>{l.title}</Link>
              <span className="muted">{l.distance === 0 ? " (the same picture)" : ` (${l.distance} of 64 bits differ)`}</span>
            </span>
          ))}
          . Another maker's app: check it isn't pretending to be theirs.
        </div>
      )}
      <MediaReview set={m} />
      {m.live && (
        <details>
          <summary className="muted small">What the store shows now</summary>
          <MediaReview set={m.live} />
        </details>
      )}
      <Notes notes={m.notes} />
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <button
          className="btn slime small"
          disabled={busy !== undefined}
          onClick={() => void run(m.id, () => api.admin.approveMedia(m.id))}
        >
          {busy === m.id ? "Approving…" : "Approve pictures"}
        </button>
        <ReasonButton
          label="Turn down"
          placeholder="What's wrong, shown to the maker"
          danger
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(m.id, () => api.admin.rejectMedia(m.id, reason))}
        />
      </div>
    </li>
  );
}

export function PortfolioCard({ p, onDone }: { p: Q["portfolios"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <Link to={`/people/${p.handle}`}>
            <b>{p.name}</b>
          </Link>
          <span className="muted">@{p.handle}</span>
          <span className="badge neutral">AI: {p.ai_use.toUpperCase()}</span>
          <span className="badge soon">{p.live ? "NEW PORTFOLIO" : "FIRST PORTFOLIO"}</span>
        </div>
        <span className="muted small">
          <ExternalLink href={`${p.set.repo_url}/tree/${p.set.commit_sha}/.slop`}>
            <code>{p.set.commit_sha.slice(0, 7)}</code>
          </ExternalLink>{" "}
          · {formatDate(p.set.created_at)}
        </span>
      </div>
      <PortfolioReview set={p.set} />
      {p.set.design && <DesignReview set={p.set} name={p.name} services={p.services} />}
      {p.live && (
        <details>
          <summary className="muted small">What the People page shows now</summary>
          <PortfolioReview set={p.live} />
          {p.live.design && <DesignReview set={p.live} name={p.name} services={p.services} />}
        </details>
      )}
      <Notes notes={p.notes} />
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <button
          className="btn slime small"
          disabled={busy !== undefined}
          onClick={() => void run(p.set.id, () => api.admin.approvePortfolio(p.set.id))}
        >
          {busy === p.set.id ? "Approving…" : "Approve portfolio"}
        </button>
        <ReasonButton
          label="Turn down"
          placeholder="What's wrong, shown to the creative"
          danger
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(p.set.id, () => api.admin.rejectPortfolio(p.set.id, reason))}
        />
      </div>
    </li>
  );
}

export function ProblemCard({ p, onDone }: { p: Q["problems"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <b>{p.title}</b>
        <span className="muted small">
          {p.creative} and {p.client} · opened by the {p.opened_by_creative ? "creative" : "client"} ·{" "}
          {formatDate(p.created_at)}
        </span>
      </div>
      <p>
        <b>Statement:</b> {p.statement}
      </p>
      {p.reply && (
        <p>
          <b>Answer:</b> {p.reply}
        </p>
      )}
      {p.proposals.length > 0 && (
        <ul className="small">
          {p.proposals.map((x, i) => (
            <li key={i}>
              {x.kind}
              {x.amount_minor ? ` ${(x.amount_minor / 100).toFixed(2)}` : ""} — {x.note} ({x.status})
            </li>
          ))}
        </ul>
      )}
      {p.mediator_suggestion && <p className="muted">Suggested: {p.mediator_suggestion}</p>}
      <JobRecord jobId={p.job_id} />
      <Notes notes={p.notes} />
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <ReasonButton
          label="Suggest an outcome"
          placeholder="A fair outcome, citing the agreement and the record"
          disabled={busy !== undefined}
          onSubmit={(text) => void run(p.id, () => api.admin.suggest(p.id, text))}
        />
        <ReasonButton
          label="Close unresolved"
          placeholder="Why it's closed, shown to both"
          danger
          disabled={busy !== undefined}
          onSubmit={(text) => void run(p.id, () => api.admin.closeProblem(p.id, text))}
        />
      </div>
    </li>
  );
}

export function DisputeCard({ d, onDone }: { d: Q["review_disputes"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <span>
          <b>{d.client}</b> on <Link to={`/people/${d.creative_handle}`}>{d.creative}</Link>:{" "}
          <VoteLabel vote={d.vote === "slop" ? "slop" : "not_slop"} />
        </span>
        <span className="muted small">{formatDate(d.created_at)}</span>
      </div>
      <blockquote>{d.review || "(no text)"}</blockquote>
      <p>
        <b>Creative's dispute</b> ({d.reason_text}): {d.statement}
      </p>
      {d.client_statement && (
        <p>
          <b>Reviewer's answer:</b> {d.client_statement}
        </p>
      )}
      <JobRecord jobId={d.job_id} />
      <Notes notes={d.notes} />
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <ReasonButton
          label="Keep it"
          placeholder="Why it stays, shown to both"
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(d.job_id, () => api.admin.decideDispute(d.job_id, { outcome: "keep", reason }))}
        />
        <ReasonButton
          label="Keep with a note"
          placeholder="The facts from the record (shown under the review)"
          disabled={busy !== undefined}
          onSubmit={(note) =>
            void run(d.job_id, () =>
              api.admin.decideDispute(d.job_id, {
                outcome: "note",
                reason: "A factual claim differs from the job record.",
                note,
              }),
            )
          }
        />
        <ReasonButton
          label="Remove it"
          placeholder="Which rule it breaks, shown to both"
          danger
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(d.job_id, () => api.admin.decideDispute(d.job_id, { outcome: "remove", reason }))}
        />
      </div>
    </li>
  );
}

export function ReadyCard({ a, onDone }: { a: Q["ready_to_list"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <Link to={`/app/${a.slug}`}>
            <b>{a.title}</b>
          </Link>
          <span className="muted">
            {STAGE_LABEL[a.stage]} · by {a.maker} · {a.approved_releases} published release
            {a.approved_releases === 1 ? "" : "s"}
            {a.going_public && " · Private app going public: check the repository and licence from scratch"}
            {a.after_appeal && " · Reinstated on appeal"}
          </span>
        </div>
        {a.pictures_pending ? (
          <span className="muted small">Review its pictures first</span>
        ) : (
          <button
            className="btn slime small"
            disabled={busy !== undefined}
            onClick={() => void run(a.slug, () => api.admin.list(a.slug))}
          >
            {busy === a.slug ? "Listing…" : a.going_public ? "Make public and list" : "Put in the store"}
          </button>
        )}
      </div>
      {a.platforms?.includes("browser") && (
        <p className="small">
          {a.play_address ? (
            <>
              Plays at <ExternalLink href={a.play_address}>{a.play_address}</ExternalLink>: a last look at the site before it goes
              in the store.
            </>
          ) : (
            // Only a Coming soon page is listed with nothing to play (admin.rs `not_listable`).
            "Plays in the browser: Play opens no address now, which a Coming soon page doesn't need."
          )}
        </p>
      )}
      <p className="small">
        <Link to={`/admin?tab=apps&app=${encodeURIComponent(a.slug)}`}>Open its record</Link>
      </p>
      <Notes notes={a.notes} />
      <ActionMessages error={error} notice={notice} />
    </li>
  );
}

export function RescanCard({ f, onDone }: { f: Q["rescans"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <Link to={`/app/${f.slug}`}>
            <b>{f.title}</b>
          </Link>
          <span className="muted">
            {f.version} · {f.platform} · {f.file_name}
          </span>
          <span className="badge bad">{f.rescan_result === "changed" ? "FILE CHANGED" : "FOUND SOMETHING"}</span>
        </div>
        <span className="muted small">{formatDate(f.rescan_flagged_at)}</span>
      </div>
      {f.rescan_report?.detail && <p className="small">{f.rescan_report.detail}</p>}
      {f.rescan_report?.findings && (
        <ScanFindings report={{ engines: {}, capabilities: [], ...f.rescan_report } as ScanReport} />
      )}
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <ReasonButton
          label="Withdraw release"
          placeholder="Why, shown to the maker and players"
          danger
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(f.release_id, () => api.admin.revoke(f.release_id, reason))}
        />
        <NoteAction
          label="Keep it: false positive"
          noteLabel="Note (optional, logged)"
          disabled={busy !== undefined}
          onSubmit={(note) => void run(f.release_id, () => api.admin.clearRescan(f.release_id, f.platform, note))}
        />
      </div>
    </li>
  );
}

export function AbuseCard({ a, onDone }: { a: Q["abuse"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <span className="badge warn">{ABUSE_LABEL[a.kind] ?? a.kind}</span>
          {a.subject_kind === "app" && a.slug ? (
            <Link to={`/app/${a.slug}`}>
              <b>{a.subject_label}</b>
            </Link>
          ) : (
            <Link to={`/u/${a.subject_id}`}>
              <b>{a.subject_label}</b>
            </Link>
          )}
        </div>
        <span className="muted small">{formatDate(a.detected_at)}</span>
      </div>
      <p className="small muted">
        {Object.entries(a.detail)
          .map(([k, v]) => `${k.replace(/_/g, " ")}: ${String(v)}`)
          .join(" · ")}
      </p>
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <ReasonButton
          label="Dealt with"
          placeholder="What you did"
          disabled={busy !== undefined}
          onSubmit={(note) => void run(a.id, () => api.admin.closeAbuse(a.id, note, true))}
        />
        <ReasonButton
          label="Nothing wrong"
          placeholder="Why it's fine"
          disabled={busy !== undefined}
          onSubmit={(note) => void run(a.id, () => api.admin.closeAbuse(a.id, note, false))}
        />
      </div>
    </li>
  );
}

/** A sensitive action waiting for another staff member's approval (staff.rs). */
export function StaffActionCard({
  a,
  approvalsRequired,
  onDone,
}: {
  a: StaffActions["actions"][number];
  approvalsRequired: number;
  onDone: () => void;
}) {
  const { busy, error, notice, run } = useAction(onDone);
  return (
    <li className="panel panel-pad">
      <b>{a.summary}</b>
      <div className="muted small">
        Asked by {a.requested_actor} · {formatDate(a.requested_at)} · {a.approvals.length} of {a.approvals_needed}{" "}
        approvals
      </div>
      <p className="muted small">
        Sensitive actions need {approvalsRequired} different staff members, counting the one who asked. Approving makes it
        happen; requests lapse after three days.
      </p>
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <button
          className="btn small slime"
          disabled={busy !== undefined}
          onClick={() => void run(a.id, () => api.admin.approveAction(a.id))}
        >
          Approve
        </button>
        <button
          className="btn small ghost danger"
          disabled={busy !== undefined}
          onClick={() => void run(a.id, () => api.admin.cancelAction(a.id))}
        >
          Turn down
        </button>
      </div>
    </li>
  );
}


function FileRow({
  file,
  holdKinds,
  canClear,
  onClear,
  waivable = false,
  onWaive,
}: {
  file: AdminQueue["releases"][number]["files"][number];
  /** What the hold is about: a file is cleared only for what holds it. */
  holdKinds: string[];
  canClear: boolean;
  onClear: (reason: string) => void;
  /** Its Defender check is missing and can be waived (Slop Check can't run for the repository). */
  waivable?: boolean;
  onWaive?: (reason: string) => void;
}) {
  const scanHeld =
    !file.cleared &&
    holdKinds.includes("scan") &&
    file.scan_result !== null &&
    !["passed", "note", "skipped"].includes(file.scan_result);
  // Defender's detections are cleared on their own; a missing check is waived, never cleared.
  const defenderHeld =
    !file.defender_cleared &&
    holdKinds.includes("defender") &&
    file.platform === "windows" &&
    Boolean(file.defender && file.defender.detections.length > 0);
  const clearable = scanHeld || defenderHeld;
  const label = fileLabel(file);
  return (
    <>
      <dt>{file.platform}</dt>
      <dd>
        {file.file_name}
        {file.exec && (
          <>
            {" "}
            · starts <code>{file.exec}</code>
          </>
        )}
        <div className="small">
          {file.sha256 ? (
            <span className="faint">
              SHA-256 <code>{file.sha256.slice(0, 16)}…</code> ·{" "}
              {file.size_bytes?.toLocaleString()} bytes ·{" "}
            </span>
          ) : (
            <span className="faint">not hashed yet · </span>
          )}
          {file.provenance ? (
            <span className="badge official">
              {file.provenance.built_by_slop_build ? "SLOP BUILD" : "SIGNED"}
            </span>
          ) : (
            <span className="badge warn" title={file.provenance_problem ?? ""}>
              NOT SIGNED
            </span>
          )}{" "}
          <span
            className={`badge ${file.scan_result === "passed" ? "official" : file.scan_result === "skipped" ? "neutral" : "bad"}`}
          >
            {file.scan_result === "skipped"
              ? "NOT SCANNED"
              : `SCAN: ${(file.scan_result ?? "pending").replace("_", " ").toUpperCase()}`}
          </span>{" "}
          {file.scan_detail && (
            <span className="muted">{file.scan_detail}</span>
          )}
        </div>
        <FileChecks file={file} audience="reviewer" />
        {clearable && (
          <div className="row-actions">
            <NoteAction
              label="Clear as a false positive"
              noteLabel={`Note for the maker about the ${label} build (optional)`}
              disabled={!canClear}
              onSubmit={onClear}
            />
          </div>
        )}
        {waivable && onWaive && (
          <>
            <p className="muted small">
              Slop Check can't run for a repository outside GitHub, so Microsoft Defender hasn't checked this build. Check it by
              hand first; waiving lets the release go on without that check.
            </p>
            <div className="row-actions">
              <NoteAction
                label="Waive Microsoft Defender's check"
                noteLabel={`Note for the maker about the ${label} build (optional)`}
                disabled={!canClear}
                onSubmit={onWaive}
              />
            </div>
          </>
        )}
      </dd>
    </>
  );
}


/** Alerts from ops.rs; each admin also gets them under the 🔔 and by push. */
export function OpsAlerts() {
  const alerts = useLoad(() => api.admin.ops(), []);
  if (!alerts.data?.length) return null;
  return (
    <div className="notice" role="status">
      <b>Needs attention</b>
      <ul className="small">
        {alerts.data.map((a) => (
          <li key={a.key}>
            <b>
              {a.link && a.link !== "/admin" && a.link !== "/admin?tab=queue" ? (
                <Link to={a.link}>{a.title}</Link>
              ) : (
                a.title
              )}
            </b>{" "}
            <span className="muted">since {formatDate(a.first_seen)}</span>.{" "}
            {a.detail}
          </li>
        ))}
      </ul>
    </div>
  );
}

export function ServerChecks({ checks }: { checks: AdminQueue["checks"] }) {
  const notes = [
    !checks.signing_key &&
      "No signing key is set (SLOP_SIGNING_KEY), so makers can't publish releases.",
    !checks.scanning && "Scanning is off: files are not scanned.",
    !checks.maker_signatures_required &&
      "Maker signatures aren't required on this server.",
    !checks.github_app &&
      "SLAPorSLOP's GitHub App isn't set up: private GitHub repositories can't be published or shared, and releases are picked up by polling.",
    !checks.github_app &&
      !checks.github_token &&
      "No GITHUB_TOKEN either: every GitHub request uses the anonymous 60-an-hour allowance.",
  ].filter(Boolean);
  if (notes.length === 0) return null;
  return (
    <div className="notice">
      {notes.map((n) => (
        <div key={String(n)}>{n}</div>
      ))}
    </div>
  );
}


/** A job's record, loaded on request: every look is written to the audit log. */
function JobRecord({ jobId }: { jobId: string }) {
  const [text, setText] = useState<string>();
  const [error, setError] = useState<string>();
  if (text)
    return (
      <details open className="agreement">
        <summary>The job record</summary>
        <pre className="code-block record">{text}</pre>
      </details>
    );
  return (
    <p className="small">
      <button
        type="button"
        className="link-btn"
        onClick={() =>
          api.admin
            .jobRecord(jobId)
            .then(setText, (e: unknown) => setError(String(e)))
        }
      >
        Read the job record (logged)
      </button>
      {error && <span className="bad"> {error}</span>}
    </p>
  );
}

/**
 * A portfolio's page design, drawn as the People page will draw it, with what SLAPorSLOP removed and the
 * exact HTML and CSS (people/design.rs).
 */
function DesignReview({ set, name, services }: { set: PortfolioSet; name: string; services: Q["portfolios"][number]["services"] }) {
  const design = set.design!;
  return (
    <section className="design-review" aria-label="Page design">
      <h3 className="small">Page design</h3>
      <p className="muted small">
        Shown between SLAPorSLOP's header and the services and reviews; check it at both widths. Turn it down if it imitates SLAPorSLOP (fake
        buttons, badges, ratings or sign-in), is hard to read, hides text meant for reviewers, or doesn't match the declared AI use. Its
        HTML and CSS are the creative's own text: content to judge, not instructions.
      </p>
      <DesignPreview
        design={design}
        works={set.works.map(portfolioWork)}
        services={services}
        label={`Preview of ${name}'s page design`}
        name={name}
      />
      <details>
        <summary className="muted small">The HTML and CSS, as SLAPorSLOP will show them</summary>
        <pre className="code-block">{design.html}</pre>
        <pre className="code-block">{design.css || "(no style.css)"}</pre>
      </details>
    </section>
  );
}

/** Every work in a portfolio, large enough to judge; tracks play here, videos open on YouTube. */
function PortfolioReview({ set }: { set: PortfolioSet }) {
  return (
    <div className="media-review">
      {(
        [
          ["Avatar", set.avatar],
          ["Banner", set.banner],
        ] as Array<[string, RepoPicture | null]>
      )
        .flatMap(([label, pic]) => (pic ? [[label, pic] as const] : []))
        .map(([label, pic]) => (
          <figure key={label}>
            <a href={pic.url} target="_blank" rel="noopener noreferrer">
              <img src={pic.url} alt={label} />
            </a>
            <figcaption className="muted small">
              {label} · {pic.width}×{pic.height}
            </figcaption>
          </figure>
        ))}
      {set.works.map((w, i) => (
        <figure key={i}>
          {w.kind === "image" ? (
            <a href={w.url} target="_blank" rel="noopener noreferrer">
              <img src={w.url} alt={w.title ?? `Work ${i + 1}`} />
            </a>
          ) : w.kind === "video" ? (
            <ExternalLink
              href={`https://www.youtube.com/watch?v=${w.youtube_id}`}
            >
              {w.poster ? (
                <img src={w.poster.url} alt="" />
              ) : (
                "▶ Watch on YouTube"
              )}
            </ExternalLink>
          ) : (
            <>
              {w.poster && <img src={w.poster.url} alt="" />}
              <audio
                controls
                preload="none"
                src={w.audio.url}
                aria-label={w.title ?? `Track ${i + 1}`}
              />
            </>
          )}
          <figcaption className="muted small">
            {i + 1}. {w.title ?? w.kind} ·{" "}
            {w.kind === "image"
              ? `${w.width}×${w.height}`
              : w.kind === "audio"
                ? w.audio.format
                : "YouTube"}
            {w.caption && <div>“{w.caption}”</div>}
          </figcaption>
        </figure>
      ))}
    </div>
  );
}

/** Every picture in a set, large enough to judge, with each video linked. */
function MediaReview({ set }: { set: MediaSet }) {
  // Icons: shown small, as players see them, and full size; with any other maker's like it.
  const iconLike = (set.lookalikes ?? []).filter((l) => l.kind === "icon");
  return (
    <div className="media-review">
      {set.icon && (
        <figure className="media-review-icon">
          <a href={set.icon.url} target="_blank" rel="noopener noreferrer">
            <img src={set.icon.url} alt="Icon" />
          </a>
          <span className="icon-sizes" aria-hidden="true">
            {[32, 64].map((size) => (
              <img key={size} src={set.icon!.url} alt="" width={size} height={size} />
            ))}
          </span>
          <figcaption className="muted small">
            Icon · {set.icon.width}×{set.icon.height}
            {iconLike.length > 0 && (
              <div>
                Looks like the icon of{" "}
                {iconLike.map((l, i) => (
                  <span key={l.slug}>
                    {i > 0 && ", "}
                    <Link to={`/app/${l.slug}`}>{l.title}</Link>
                  </span>
                ))}
              </div>
            )}
          </figcaption>
        </figure>
      )}
      {set.thumbnail && (
        <figure>
          <a href={set.thumbnail.url} target="_blank" rel="noopener noreferrer">
            <img src={set.thumbnail.url} alt="Thumbnail" />
          </a>
          <figcaption className="muted small">
            Thumbnail · {set.thumbnail.width}×{set.thumbnail.height}
          </figcaption>
        </figure>
      )}
      {set.gallery.map((item, i) => (
        <figure key={i}>
          {item.kind === "image" ? (
            <a href={item.url} target="_blank" rel="noopener noreferrer">
              <img src={item.url} alt={`Screenshot ${i + 1}`} />
            </a>
          ) : (
            <ExternalLink
              href={`https://www.youtube.com/watch?v=${item.youtube_id}`}
            >
              <ItemThumb item={item} />
            </ExternalLink>
          )}
          <figcaption className="muted small">
            {i + 1}.{" "}
            {item.kind === "image"
              ? `${item.path} · ${item.width}×${item.height}`
              : "YouTube video: watch it"}
            {item.caption && <div>“{item.caption}”</div>}
          </figcaption>
        </figure>
      ))}
      {!set.thumbnail && !set.icon && set.gallery.length === 0 && (
        <p className="muted small">No pictures: this set removes them all.</p>
      )}
    </div>
  );
}

const VERDICT: Record<
  NonNullable<ReviewNote["verdict"]>,
  { label: string; badge: string }
> = {
  pass: { label: "PASS", badge: "official" },
  concern: { label: "CONCERN", badge: "bad" },
  unsure: { label: "UNSURE", badge: "warn" },
};

/** Notes reviewers (often AI reviewers) left on an item. */
export function Notes({ notes }: { notes: ReviewNote[] }) {
  if (notes.length === 0) return null;
  return (
    <ul className="review-notes">
      {notes.map((n) => (
        <li key={n.id}>
          {n.verdict && (
            <span className={`badge ${VERDICT[n.verdict].badge}`}>
              {VERDICT[n.verdict].label}
            </span>
          )}{" "}
          {n.note}
          <div className="faint small">
            {n.author} · {formatDate(n.created_at)}
          </div>
        </li>
      ))}
    </ul>
  );
}


const ACTION_LABEL: Record<ModerationDecision["action"], string> = {
  delist_app: "Take the app out of the store",
  hide_review: "Hide the review",
  remove_reply: "Remove the maker's reply under it",
  hide_profile: "Hide the profile",
  hide_team: "Hide the team",
  hide_bounty: "Hide the bounty",
  hide_news: "Hide the news post",
  hide_work_request: "Hide the request",
  warn: "Warn them (no restriction)",
  suspend_account: "Suspend their account",
};

const PRIORITY_LABEL = ["URGENT", "SERIOUS", "NORMAL"] as const;


export function ReportCard({
  r,
  rules,
  onDone,
}: {
  r: QueuedReport;
  rules: Array<[string, string]>;
  onDone: () => void;
}) {
  const { busy, error, run } = useAction(onDone);
  const [deciding, setDeciding] = useState(false);
  const [action, setAction] = useState<string>(r.actions[0]);
  const [rule, setRule] = useState(r.suggested_rule ?? "");
  const [law, setLaw] = useState("");
  const [facts, setFacts] = useState("");
  const [outcome, setOutcome] = useState("");
  const [external, setExternal] = useState("");
  const askExternal =
    r.preserve || r.reason === "threat" || r.reason === "illegal";
  return (
    <li className={`panel panel-pad report-card${r.overdue ? " overdue" : ""}`}>
      <div className="title-row spread">
        <div className="title-row">
          <span
            className={`badge ${r.priority === 0 ? "bad" : r.priority === 1 ? "warn" : "neutral"}`}
          >
            {PRIORITY_LABEL[r.priority]}
          </span>
          <b>{r.reason_label}</b>
          <span className="muted">
            ·{" "}
            {r.link ? (
              <Link to={r.link}>{r.subject_label}</Link>
            ) : (
              r.subject_label
            )}
          </span>
        </div>
        <span className={`small ${r.overdue ? "bad" : "muted"}`}>
          {r.reference} · due {formatDate(r.due_at)}
          {r.overdue && " (overdue)"}
        </span>
      </div>
      <dl className="kv">
        <dt>Report</dt>
        <dd className="pre">{r.details}</dd>
        {r.quoted && (
          <>
            <dt>{r.subject_kind === "news" ? "The post" : r.subject_kind === "work_request" ? "The request" : "The message"}</dt>
            <dd>
              <blockquote className="pre">{r.quoted}</blockquote>
            </dd>
          </>
        )}
        <dt>From</dt>
        <dd>
          {r.reporter} · {formatDate(r.created_at)}
          {r.same_subject > 0 && (
            <span className="muted">
              {" "}
              · {r.same_subject} more open about this
            </span>
          )}
        </dd>
        {r.account && (
          <>
            <dt>About</dt>
            <dd>
              {r.account.name}
              {r.account.suspended && (
                <span className="badge bad"> SUSPENDED</span>
              )}
              {r.account.closed && (
                <span className="badge neutral"> CLOSED</span>
              )}
              <span className="muted">
                {" "}
                · {r.account.decisions} earlier decision
                {r.account.decisions === 1 ? "" : "s"}
                {r.account.copyright_strikes > 0 &&
                  `, ${r.account.copyright_strikes} copyright`}
              </span>
              {r.account.copyright_strikes >= 2 && r.reason === "copyright" && (
                <div className="warn small">
                  One more upheld copyright decision ends the account
                  (repeat-infringer policy).
                </div>
              )}
            </dd>
          </>
        )}
        {r.copyright && (
          <>
            <dt>Copyright notice</dt>
            <dd className="small">
              Work: {r.copyright.work}
              {r.copyright.owner && <> · owner: {r.copyright.owner}</>}
              <br />
              {r.copyright.signature}, {r.copyright.address},{" "}
              {r.copyright.phone} · both statements made
            </dd>
          </>
        )}
        {r.preserve && (
          <>
            <dt>Preserve</dt>
            <dd className="warn small">
              Child abuse report: kept for a year, never deleted with the
              content. File it with NCMEC (US) and the NCA (UK) and note the
              references below.
            </dd>
          </>
        )}
      </dl>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {!deciding ? (
        <div className="row-actions">
          <button className="btn small" onClick={() => setDeciding(true)}>
            Decide
          </button>
        </div>
      ) : (
        <form
          className="form decide-form"
          onSubmit={(e) => {
            e.preventDefault();
            void run(r.id, () =>
              api.admin.act(r.id, {
                action,
                rule,
                law: law || undefined,
                facts,
                outcome,
                external_reference: external || undefined,
              }),
            );
          }}
        >
          <label>
            What to do
            <select value={action} onChange={(e) => setAction(e.target.value)}>
              {r.actions.map((a) => (
                <option key={a} value={a}>
                  {ACTION_LABEL[a]}
                </option>
              ))}
            </select>
          </label>
          <RuleSelect rules={rules} value={rule} onChange={setRule} />
          <label>
            The law, if it's the law and not only SLAPorSLOP's rules{" "}
            <span className="muted">(optional)</span>
            <input
              value={law}
              maxLength={300}
              placeholder="e.g. 17 U.S.C. §512"
              onChange={(e) => setLaw(e.target.value)}
            />
          </label>
          <label>
            The facts, as {r.account?.name ?? "they"} will read them
            <textarea
              value={facts}
              rows={3}
              maxLength={4000}
              onChange={(e) => setFacts(e.target.value)}
            />
          </label>
          <label>
            What the reporter is told
            <textarea
              value={outcome}
              rows={2}
              maxLength={2000}
              onChange={(e) => setOutcome(e.target.value)}
            />
          </label>
          {askExternal && (
            <label>
              Passed on to (NCMEC, NCA, police) and reference{" "}
              <span className="muted">(optional)</span>
              <input
                value={external}
                maxLength={200}
                onChange={(e) => setExternal(e.target.value)}
              />
            </label>
          )}
          <div className="row-actions">
            <button
              className="btn small danger"
              disabled={
                busy !== undefined || !rule || !facts.trim() || !outcome.trim()
              }
            >
              {ACTION_LABEL[action as ModerationDecision["action"]]}
            </button>
            <button
              type="button"
              className="btn small"
              disabled={busy !== undefined || !outcome.trim()}
              title="Close the report without action; the reporter is told what you wrote above"
              onClick={() =>
                void run(r.id, () =>
                  api.admin.dismiss(r.id, outcome, external || undefined),
                )
              }
            >
              No action
            </button>
            <button
              type="button"
              className="btn small ghost"
              onClick={() => setDeciding(false)}
            >
              Cancel
            </button>
          </div>
        </form>
      )}
    </li>
  );
}


export function AppealCard({
  a,
  onDone,
  onNotice,
}: {
  a: QueuedAppeal;
  onDone: () => void;
  onNotice: (notice: string | undefined) => void;
}) {
  const { busy, error, run } = useAction(onDone);
  const [reason, setReason] = useState("");
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          <b>{a.action_label}</b>
          <span className="muted">
            ·{" "}
            {a.link ? (
              <Link to={a.link}>{a.subject_label}</Link>
            ) : (
              a.subject_label
            )}
          </span>
        </div>
        <span className="muted small">
          {a.reference} · appealed {formatDate(a.appealed_at!)}
        </span>
      </div>
      <dl className="kv">
        <dt>Decision</dt>
        <dd>
          {a.rule_title}: {a.facts}
          <div className="muted small">
            by {a.decided_by}, {formatDate(a.created_at)}
          </div>
        </dd>
        {/* The page no longer shows what was hidden: this is what staff read (a news post's text). */}
        {a.quoted && (
          <>
            <dt>What was hidden</dt>
            <dd>
              <blockquote className="pre">{a.quoted}</blockquote>
            </dd>
          </>
        )}
        <dt>Appeal</dt>
        <dd className="pre">{a.appeal}</dd>
        <dt>Account</dt>
        <dd>
          {a.account.name} · {a.account.decisions} decision
          {a.account.decisions === 1 ? "" : "s"}
        </dd>
        {a.counter_notice && (
          <>
            <dt>Counter-notice</dt>
            <dd className="small">
              {a.counter_notice.signature}, {a.counter_notice.address},{" "}
              {a.counter_notice.phone} · both statements made.
              <div className="warn">
                Send these details to the person who reported it. Reverse
                (restore) 10 to 14 working days from now unless they tell you
                they've started a court case.
              </div>
            </dd>
          </>
        )}
      </dl>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <form className="form decide-form" onSubmit={(e) => e.preventDefault()}>
        <label>
          Why (the person reads this)
          <textarea
            value={reason}
            rows={2}
            maxLength={2000}
            onChange={(e) => setReason(e.target.value)}
          />
        </label>
        <div className="row-actions">
          <button
            type="button"
            className="btn small slime"
            disabled={busy !== undefined || !reason.trim()}
            onClick={() =>
              void run(a.id, async () => {
                const { stays_out } = await api.admin.decideAppeal(
                  a.id,
                  true,
                  reason,
                );
                onNotice(
                  stays_out
                    ? `Reversed, but ${a.subject_label} can't go back in the store yet. The maker was told: ${stays_out}`
                    : undefined,
                );
              })
            }
          >
            Reverse the decision
          </button>
          <button
            type="button"
            className="btn small"
            disabled={busy !== undefined || !reason.trim()}
            onClick={() =>
              void run(a.id, () => api.admin.decideAppeal(a.id, false, reason))
            }
          >
            Keep it
          </button>
        </div>
      </form>
    </li>
  );
}
