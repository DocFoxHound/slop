// SPDX-License-Identifier: AGPL-3.0-or-later
// The review queue's items in full (crates/slop-api/src/admin_web.rs): what the inbox
// (Inbox.tsx) opens when a row is clicked. Each card shows everything there is to judge it by
// (files, checks, scan reports, pictures, notes from AI reviewers, the job record) and every way
// to decide it, and tells `onDone` once something changed.

import { useState } from "react";
import { Link } from "react-router";

import {
  api,
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
} from "../../api";
import { FileChecks, RepoScanList, ScanFindings, SourceCheckList } from "../../components/Checks";
import { ItemThumb } from "../../components/MediaEditor";
import { ExternalLink } from "../../components/ui";
import { formatDate, STAGE_LABEL } from "../../format";
import { useLoad } from "../../hooks";
import { ActionMessages, ReasonButton, RuleSelect, useAction } from "./shared";

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

export function ReleaseCard({ r, onDone }: { r: Q["releases"][number]; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
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
          <span className={`badge ${r.status === "failed" ? "bad" : r.status === "fetching" ? "neutral" : "soon"}`}>
            {r.status === "awaiting_review" ? "AWAITING REVIEW" : r.status.toUpperCase()}
          </span>
        </div>
        <span className="muted small">{formatDate(r.created_at)}</span>
      </div>
      {r.status_reason && <p className="notice">{r.status_reason}</p>}
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
            canClear={r.status === "awaiting_review" && busy === undefined}
            onClear={(reason) => void run(`${r.id}/${f.platform}`, () => api.admin.clearScan(r.id, f.platform, reason))}
          />
        ))}
      </dl>
      <SourceCheckList checks={r.source_checks} audience="reviewer" />
      <RepoScanList scan={r.repo_scan} advisories={r.advisories} audience="reviewer" />
      <Notes notes={r.notes} />
      <ActionMessages error={error} notice={notice} />
      <div className="row-actions">
        <button
          className="btn slime small"
          disabled={busy !== undefined || r.status !== "awaiting_review"}
          onClick={() => void run(r.id, () => api.admin.approve(r.id))}
        >
          {busy === r.id ? "Approving…" : "Approve and sign"}
        </button>
        <ReasonButton
          label="Reject"
          placeholder="Reason, shown to the maker"
          danger
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(r.id, () => api.admin.reject(r.id, reason))}
        />
      </div>
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
            <dd>{c.release_id ? `v${c.release_version} (held there)` : "the newest approved release"}</dd>
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
      {p.live && (
        <details>
          <summary className="muted small">What the People page shows now</summary>
          <PortfolioReview set={p.live} />
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
          {d.vote === "slop" ? "👎 Slop" : "👍 Not Slop"}
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
            {STAGE_LABEL[a.stage]} · by {a.maker} · {a.approved_releases} approved release
            {a.approved_releases === 1 ? "" : "s"}
            {a.going_public && " · Private Slop going public: check the repository and licence from scratch"}
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
        <ReasonButton
          label="Keep it: false positive"
          placeholder="Why it's a false positive (logged, at least 10 characters)"
          disabled={busy !== undefined}
          onSubmit={(reason) => void run(f.release_id, () => api.admin.clearRescan(f.release_id, f.platform, reason))}
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
  canClear,
  onClear,
}: {
  file: AdminQueue["releases"][number]["files"][number];
  canClear: boolean;
  onClear: (reason: string) => void;
}) {
  const scanHeld =
    file.scan_result !== null &&
    !["passed", "note", "skipped"].includes(file.scan_result);
  const defenderMissing =
    file.platform === "windows" &&
    (!file.defender || file.defender.detections.length > 0);
  const clearable = !file.cleared_reason && (scanHeld || defenderMissing);
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
            <ReasonButton
              label="Clear as a false positive"
              placeholder="Why it's a false positive (logged, at least 10 characters)"
              disabled={!canClear}
              onSubmit={onClear}
            />
          </div>
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
            <b>{a.title}</b>{" "}
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
      "No signing key is set (SLOP_SIGNING_KEY), so nothing can be approved.",
    !checks.scanning && "Scanning is off: files are not scanned.",
    !checks.maker_signatures_required &&
      "Maker signatures aren't required on this server.",
    !checks.github_app &&
      "Slop's GitHub App isn't set up: private GitHub repositories can't be published or shared, and releases are picked up by polling.",
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
            <dt>The message</dt>
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
            The law, if it's the law and not only Slop's rules{" "}
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
