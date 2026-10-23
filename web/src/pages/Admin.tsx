// SPDX-License-Identifier: AGPL-3.0-or-later
// The admin panel for Slop staff: review releases, list and moderate apps, read the audit log.
// Every action is recorded in the audit log under the admin's name (crates/slop-api/src/admin_web.rs).

import { useState } from "react";
import { Link, useSearchParams } from "react-router";

import {
  api,
  type ModerationDecision,
  type QueuedAppeal,
  type QueuedReport,
  type AdminApp,
  type StaffRole,
  type AdminQueue,
  type AdminToken,
  type AuditEntry,
  type AppStage,
  type AppStatus,
  type MediaSet,
  type PortfolioSet,
  type RepoPicture,
  type ReviewNote,
  type ScanReport,
} from "../api";
import {
  FileChecks,
  RepoScanList,
  ScanFindings,
  SourceCheckList,
} from "../components/Checks";
import { StepUpPrompt } from "../components/PasskeyStepUp";
import { ItemThumb } from "../components/MediaEditor";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { accountId, formatDate, STAGE_LABEL } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { NotFound } from "./Listing";
import { LegalRequestsTab } from "../components/LegalRequests";
import { AiMessagesTab } from "./admin/AiMessages";
import { AppRecord, AuditRows } from "./admin/AppRecord";
import { AdminHome } from "./admin/Home";
import { LegalDocsTab } from "./admin/LegalDocs";
import { StoreTab } from "./admin/StoreTab";
import { MoneyTab } from "./admin/Money";
import { DelistButton, ReasonButton, RuleSelect, useAction } from "./admin/shared";
import { UsersTab } from "./admin/Users";

type Tab =
  | "home"
  | "queue"
  | "apps"
  | "users"
  | "store"
  | "money"
  | "audit"
  | "ai-messages"
  | "ai"
  | "staff"
  | "docs"
  | "legal";

export function Admin() {
  useDocumentTitle("Admin");
  const { me } = useSession();
  const [params, setParams] = useSearchParams();
  const tab = (params.get("tab") as Tab | null) ?? "home";

  if (me === undefined) return <Loading />;
  if (!me?.is_admin) return <NotFound />;

  return (
    <div className="page admin">
      <h1 className="page-title">Admin</h1>
      <div className="seg" role="tablist">
        {(
          [
            ["home", "Home"],
            ["queue", "Review queue"],
            ["apps", "Apps"],
            ["users", "Accounts"],
            ["store", "Front page"],
            ["money", "Money"],
            ["audit", "Audit log"],
            ["ai-messages", "AI messages"],
            ["ai", "AI reviewers"],
            ["staff", "Staff"],
            ["docs", "Terms & policies"],
            ["legal", "Legal requests"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            role="tab"
            aria-selected={tab === key}
            className={tab === key ? "on" : ""}
            onClick={() => setParams({ tab: key }, { replace: true })}
          >
            {label}
          </button>
        ))}
      </div>
      {tab === "home" && <AdminHome />}
      {tab === "queue" && <QueueTab />}
      {tab === "apps" && (params.get("app") ? <AppRecord slug={params.get("app")!} /> : <AppsTab />)}
      {tab === "users" && <UsersTab />}
      {tab === "store" && <StoreTab />}
      {tab === "money" && <MoneyTab />}
      {tab === "audit" && <AuditTab />}
      {tab === "ai-messages" && <AiMessagesTab />}
      {tab === "ai" && <TokensTab />}
      {tab === "staff" && <StaffTab />}
      {tab === "docs" && <LegalDocsTab />}
      {tab === "legal" && <LegalRequestsTab />}
    </div>
  );
}

const ABUSE_LABEL: Record<string, string> = {
  vote_burst: "VOTES FROM NEW ACCOUNTS",
  thin_votes: "VOTES FROM PLAYERS WHO BARELY PLAYED",
  review_bomb: "REVIEW BOMB",
  vote_swing: "VOTE SWING",
  lookalike_name: "LOOKALIKE NAME",
  rate_limited: "HITTING RATE LIMITS",
  malicious_dependency: "MALICIOUS DEPENDENCY",
  bad_link: "LINK TO A KNOWN-BAD ADDRESS",
};

function QueueTab() {
  const queue = useLoad(() => api.admin.queue(), []);
  const { busy, error, notice, run } = useAction(queue.reload);
  if (queue.error)
    return <ErrorState error={queue.error} onRetry={queue.reload} />;
  if (!queue.data) return <Loading />;
  const q: AdminQueue = queue.data;

  return (
    <>
      <OpsAlerts />
      <PendingActions />
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      <ServerChecks checks={q.checks} />
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      <ReportsSection q={q} onDone={queue.reload} />
      <AppealsSection q={q} onDone={queue.reload} />

      {q.abuse.length > 0 && (
        <>
          <h2 className="section-title">Possible abuse ({q.abuse.length})</h2>
          <p className="muted small">
            Patterns from the hourly checks. Nothing has changed: look, act
            through the usual tools if it's real, then close it with what you
            found.
          </p>
          <ul className="release-list">
            {q.abuse.map((a) => (
              <li key={a.id} className="panel panel-pad">
                <div className="title-row spread">
                  <div className="title-row">
                    <span className="badge warn">
                      {ABUSE_LABEL[a.kind] ?? a.kind}
                    </span>
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
                  <span className="muted small">
                    {formatDate(a.detected_at)}
                  </span>
                </div>
                <p className="small muted">
                  {Object.entries(a.detail)
                    .map(([k, v]) => `${k.replace(/_/g, " ")}: ${String(v)}`)
                    .join(" · ")}
                </p>
                <div className="row-actions">
                  <ReasonButton
                    label="Dealt with"
                    placeholder="What you did"
                    disabled={busy !== undefined}
                    onSubmit={(note) =>
                      void run(a.id, () =>
                        api.admin.closeAbuse(a.id, note, true),
                      )
                    }
                  />
                  <ReasonButton
                    label="Nothing wrong"
                    placeholder="Why it's fine"
                    disabled={busy !== undefined}
                    onSubmit={(note) =>
                      void run(a.id, () =>
                        api.admin.closeAbuse(a.id, note, false),
                      )
                    }
                  />
                </div>
              </li>
            ))}
          </ul>
        </>
      )}

      {q.rescans.length > 0 && (
        <>
          <h2 className="section-title">
            Live releases flagged by a re-scan ({q.rescans.length})
          </h2>
          <p className="muted small">
            Players still have these. Withdraw the release if it's harmful (it
            stops launching for everyone), or keep it if the finding is a false
            positive; the same finding isn't raised again.
          </p>
          <ul className="release-list">
            {q.rescans.map((f) => (
              <li
                key={`${f.release_id}/${f.platform}`}
                className="panel panel-pad"
              >
                <div className="title-row spread">
                  <div className="title-row">
                    <Link to={`/app/${f.slug}`}>
                      <b>{f.title}</b>
                    </Link>
                    <span className="muted">
                      {f.version} · {f.platform} · {f.file_name}
                    </span>
                    <span className="badge bad">
                      {f.rescan_result === "changed"
                        ? "FILE CHANGED"
                        : "FOUND SOMETHING"}
                    </span>
                  </div>
                  <span className="muted small">
                    {formatDate(f.rescan_flagged_at)}
                  </span>
                </div>
                {f.rescan_report?.detail && (
                  <p className="small">{f.rescan_report.detail}</p>
                )}
                {f.rescan_report?.findings && (
                  <ScanFindings
                    report={
                      {
                        engines: {},
                        capabilities: [],
                        ...f.rescan_report,
                      } as ScanReport
                    }
                  />
                )}
                <div className="row-actions">
                  <ReasonButton
                    label="Withdraw release"
                    placeholder="Why, shown to the maker and players"
                    danger
                    disabled={busy !== undefined}
                    onSubmit={(reason) =>
                      void run(f.release_id, () =>
                        api.admin.revoke(f.release_id, reason),
                      )
                    }
                  />
                  <ReasonButton
                    label="Keep it: false positive"
                    placeholder="Why it's a false positive (logged, at least 10 characters)"
                    disabled={busy !== undefined}
                    onSubmit={(reason) =>
                      void run(f.release_id, () =>
                        api.admin.clearRescan(f.release_id, f.platform, reason),
                      )
                    }
                  />
                </div>
              </li>
            ))}
          </ul>
        </>
      )}

      <h2 className="section-title">
        Releases to review ({q.releases.length})
      </h2>
      <p className="muted small">
        Approving a release signs it for players. A Beta or Released app needs
        one approved release before it can go in the store.
      </p>
      {q.releases.length === 0 && (
        <div className="empty-state">Nothing waiting for review.</div>
      )}
      <ul className="release-list">
        {q.releases.map((r) => (
          <li key={r.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <Link to={`/app/${r.app.slug}`}>
                  <b>{r.app.title}</b>
                </Link>
                <span className="muted">
                  {r.version} · by {r.app.maker}
                </span>
                <span
                  className={`badge ${r.status === "failed" ? "bad" : r.status === "fetching" ? "neutral" : "soon"}`}
                >
                  {r.status === "awaiting_review"
                    ? "AWAITING REVIEW"
                    : r.status.toUpperCase()}
                </span>
              </div>
              <span className="muted small">{formatDate(r.created_at)}</span>
            </div>
            {r.status_reason && <p className="notice">{r.status_reason}</p>}
            <dl className="kv">
              <dt>Repository</dt>
              <dd>
                <ExternalLink href={r.app.repo_url}>
                  {r.app.repo_url.replace(/^https:\/\//, "")}
                </ExternalLink>
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
                  canClear={
                    r.status === "awaiting_review" && busy === undefined
                  }
                  onClear={(reason) =>
                    void run(`${r.id}/${f.platform}`, () =>
                      api.admin.clearScan(r.id, f.platform, reason),
                    )
                  }
                />
              ))}
            </dl>
            <SourceCheckList checks={r.source_checks} audience="reviewer" />
            <RepoScanList
              scan={r.repo_scan}
              advisories={r.advisories}
              audience="reviewer"
            />
            <Notes notes={r.notes} />
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
                onSubmit={(reason) =>
                  void run(r.id, () => api.admin.reject(r.id, reason))
                }
              />
            </div>
          </li>
        ))}
      </ul>

      <h2 className="section-title">Changes to review ({q.changes.length})</h2>
      <p className="muted small">
        Makers asking to change what players download: holding players at a
        different release, or moving the app to another repository. Approving
        makes the change straight away.
      </p>
      {q.changes.length === 0 && (
        <div className="empty-state">No changes waiting.</div>
      )}
      <ul className="release-list">
        {q.changes.map((c) => (
          <li key={c.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <Link to={`/app/${c.app.slug}`}>
                  <b>{c.app.title}</b>
                </Link>
                <span className="muted">by {c.app.maker}</span>
                <span className="badge soon">
                  {c.kind === "repo" ? "MOVE REPOSITORY" : "SWITCH RELEASE"}
                </span>
              </div>
              <span className="muted small">{formatDate(c.created_at)}</span>
            </div>
            <dl className="kv">
              {c.kind === "repo" ? (
                <>
                  <dt>From</dt>
                  <dd>
                    <ExternalLink href={c.app.repo_url}>
                      {c.app.repo_url.replace(/^https:\/\//, "")}
                    </ExternalLink>
                  </dd>
                  <dt>To</dt>
                  <dd>
                    <ExternalLink href={c.repo_url ?? ""}>
                      {(c.repo_url ?? "").replace(/^https:\/\//, "")}
                    </ExternalLink>
                  </dd>
                </>
              ) : (
                <>
                  <dt>Players get</dt>
                  <dd>{c.players_get ? `v${c.players_get}` : "nothing yet"}</dd>
                  <dt>Change to</dt>
                  <dd>
                    {c.release_id
                      ? `v${c.release_version} (held there)`
                      : "the newest approved release"}
                  </dd>
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
                Check the new repository belongs to the same maker and project.
                Its first release will come back here for review.
              </p>
            )}
            <div className="row-actions">
              <button
                className="btn slime small"
                disabled={busy !== undefined}
                onClick={() =>
                  void run(c.id, () => api.admin.approveChange(c.id))
                }
              >
                {busy === c.id ? "Approving…" : "Approve"}
              </button>
              <ReasonButton
                label="Turn down"
                placeholder="Reason, shown to the maker"
                danger
                disabled={busy !== undefined}
                onSubmit={(reason) =>
                  void run(c.id, () => api.admin.rejectChange(c.id, reason))
                }
              />
            </div>
          </li>
        ))}
      </ul>

      <h2 className="section-title">Pictures to review ({q.media.length})</h2>
      <p className="muted small">
        Store-page pictures and videos from makers' repositories, fixed at a
        commit. Look at every one: no sexually explicit content, nothing that
        doesn't fit the age rating. Approving shows them in the store; an app
        can't be put in the store while its pictures wait here.
      </p>
      {q.media.length === 0 && (
        <div className="empty-state">No pictures waiting.</div>
      )}
      <ul className="release-list">
        {q.media.map((m) => (
          <li key={m.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <Link to={`/app/${m.app.slug}`}>
                  <b>{m.app.title}</b>
                </Link>
                <span className="muted">by {m.app.maker}</span>
                <span className="badge soon">
                  {m.live ? "NEW PICTURES" : "FIRST PICTURES"}
                </span>
              </div>
              <span className="muted small">
                <ExternalLink
                  href={`${m.app.repo_url}/tree/${m.commit_sha}/.slop`}
                >
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
                <summary className="muted small">
                  What the store shows now
                </summary>
                <MediaReview set={m.live} />
              </details>
            )}
            <Notes notes={m.notes} />
            <div className="row-actions">
              <button
                className="btn slime small"
                disabled={busy !== undefined}
                onClick={() =>
                  void run(m.id, () => api.admin.approveMedia(m.id))
                }
              >
                {busy === m.id ? "Approving…" : "Approve pictures"}
              </button>
              <ReasonButton
                label="Turn down"
                placeholder="What's wrong, shown to the maker"
                danger
                disabled={busy !== undefined}
                onSubmit={(reason) =>
                  void run(m.id, () => api.admin.rejectMedia(m.id, reason))
                }
              />
            </div>
          </li>
        ))}
      </ul>

      <h2 className="section-title">
        People portfolios to review ({q.portfolios.length})
      </h2>
      <p className="muted small">
        Creatives' portfolios from their GitHub repositories, fixed at a commit.
        A profile goes live with its first approved portfolio. Look at every
        picture and listen to tracks: no sexually explicit content, and the work
        should match the declared AI use.
      </p>
      {q.portfolios.length === 0 && (
        <div className="empty-state">No portfolios waiting.</div>
      )}
      <ul className="release-list">
        {q.portfolios.map((p) => (
          <li key={p.set.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <Link to={`/people/${p.handle}`}>
                  <b>{p.name}</b>
                </Link>
                <span className="muted">@{p.handle}</span>
                <span className="badge neutral">
                  AI: {p.ai_use.toUpperCase()}
                </span>
                <span className="badge soon">
                  {p.live ? "NEW PORTFOLIO" : "FIRST PORTFOLIO"}
                </span>
              </div>
              <span className="muted small">
                <ExternalLink
                  href={`${p.set.repo_url}/tree/${p.set.commit_sha}/.slop`}
                >
                  <code>{p.set.commit_sha.slice(0, 7)}</code>
                </ExternalLink>{" "}
                · {formatDate(p.set.created_at)}
              </span>
            </div>
            <PortfolioReview set={p.set} />
            <Notes notes={p.notes} />
            <div className="row-actions">
              <button
                className="btn slime small"
                disabled={busy !== undefined}
                onClick={() =>
                  void run(p.set.id, () => api.admin.approvePortfolio(p.set.id))
                }
              >
                {busy === p.set.id ? "Approving…" : "Approve portfolio"}
              </button>
              <ReasonButton
                label="Turn down"
                placeholder="What's wrong, shown to the creative"
                danger
                disabled={busy !== undefined}
                onSubmit={(reason) =>
                  void run(p.set.id, () =>
                    api.admin.rejectPortfolio(p.set.id, reason),
                  )
                }
              />
            </div>
          </li>
        ))}
      </ul>

      <h2 className="section-title">People problems ({q.problems.length})</h2>
      <p className="muted small">
        Problems the two sides haven't settled. Read the record (opening it is
        logged), then suggest a fair outcome. Suggestions bind no one: only what
        both sides accept happens. Close it unresolved when they can't agree;
        both keep the record for a card dispute or a court.
      </p>
      {q.problems.length === 0 && (
        <div className="empty-state">No open problems.</div>
      )}
      <ul className="release-list">
        {q.problems.map((p) => (
          <li key={p.id} className="panel panel-pad">
            <div className="title-row spread">
              <b>{p.title}</b>
              <span className="muted small">
                {p.creative} and {p.client} · opened by the{" "}
                {p.opened_by_creative ? "creative" : "client"} ·{" "}
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
                    {x.amount_minor
                      ? ` ${(x.amount_minor / 100).toFixed(2)}`
                      : ""}{" "}
                    — {x.note} ({x.status})
                  </li>
                ))}
              </ul>
            )}
            {p.mediator_suggestion && (
              <p className="muted">Suggested: {p.mediator_suggestion}</p>
            )}
            <JobRecord jobId={p.job_id} />
            <Notes notes={p.notes} />
            <div className="row-actions">
              <ReasonButton
                label="Suggest an outcome"
                placeholder="A fair outcome, citing the agreement and the record"
                disabled={busy !== undefined}
                onSubmit={(text) =>
                  void run(p.id, () => api.admin.suggest(p.id, text))
                }
              />
              <ReasonButton
                label="Close unresolved"
                placeholder="Why it's closed, shown to both"
                danger
                disabled={busy !== undefined}
                onSubmit={(text) =>
                  void run(p.id, () => api.admin.closeProblem(p.id, text))
                }
              />
            </div>
          </li>
        ))}
      </ul>

      <h2 className="section-title">
        Review disputes ({q.review_disputes.length})
      </h2>
      <p className="muted small">
        Decide on the rules, never on whether the review is negative. Remove it
        only if it breaks the rules, isn't about the job, or was used as a
        threat; add a factual note when it states something the record shows is
        false; otherwise keep it. The reason is shown to both.
      </p>
      {q.review_disputes.length === 0 && (
        <div className="empty-state">No review disputes.</div>
      )}
      <ul className="release-list">
        {q.review_disputes.map((d) => (
          <li key={d.job_id} className="panel panel-pad">
            <div className="title-row spread">
              <span>
                <b>{d.client}</b> on{" "}
                <Link to={`/people/${d.creative_handle}`}>{d.creative}</Link>:{" "}
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
            <div className="row-actions">
              <ReasonButton
                label="Keep it"
                placeholder="Why it stays, shown to both"
                disabled={busy !== undefined}
                onSubmit={(reason) =>
                  void run(d.job_id, () =>
                    api.admin.decideDispute(d.job_id, {
                      outcome: "keep",
                      reason,
                    }),
                  )
                }
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
                onSubmit={(reason) =>
                  void run(d.job_id, () =>
                    api.admin.decideDispute(d.job_id, {
                      outcome: "remove",
                      reason,
                    }),
                  )
                }
              />
            </div>
          </li>
        ))}
      </ul>

      <h2 className="section-title">
        Ready for the store ({q.ready_to_list.length})
      </h2>
      {q.ready_to_list.length === 0 && (
        <div className="empty-state">No drafts are ready to list.</div>
      )}
      <ul className="release-list">
        {q.ready_to_list.map((a) => (
          <li key={a.slug} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <Link to={`/app/${a.slug}`}>
                  <b>{a.title}</b>
                </Link>
                <span className="muted">
                  {STAGE_LABEL[a.stage]} · by {a.maker} · {a.approved_releases}{" "}
                  approved release
                  {a.approved_releases === 1 ? "" : "s"}
                  {a.going_public &&
                    " · Private Slop going public: check the repository and licence from scratch"}
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
                  {busy === a.slug
                    ? "Listing…"
                    : a.going_public
                      ? "Make public and list"
                      : "Put in the store"}
                </button>
              )}
            </div>
            <Notes notes={a.notes} />
          </li>
        ))}
      </ul>
    </>
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

/** Sensitive actions waiting for another staff member's approval (staff.rs). */
function PendingActions() {
  const actions = useLoad(() => api.admin.actions(), []);
  const { busy, error, stepUp, run } = useAction(actions.reload);
  const list = actions.data?.actions ?? [];
  if (!list.length) return null;
  return (
    <section className="panel panel-pad">
      <h2>Waiting for approval ({list.length})</h2>
      <p className="muted small">
        Sensitive actions need {actions.data?.approvals_required} different
        staff members, counting the one who asked. Approving makes it happen;
        requests lapse after three days.
      </p>
      <ul className="release-list">
        {list.map((a) => (
          <li key={a.id}>
            <b>{a.summary}</b>
            <div className="muted small">
              Asked by {a.requested_actor} · {formatDate(a.requested_at)} ·{" "}
              {a.approvals.length} of {a.approvals_needed} approvals
            </div>
            <div className="row-actions">
              <button
                className="btn small slime"
                disabled={busy !== undefined}
                onClick={() =>
                  void run(a.id, () => api.admin.approveAction(a.id))
                }
              >
                Approve
              </button>
              <button
                className="btn small ghost danger"
                disabled={busy !== undefined}
                onClick={() =>
                  void run(a.id, () => api.admin.cancelAction(a.id))
                }
              >
                Turn down
              </button>
            </div>
          </li>
        ))}
      </ul>
      {error && (
        <p className="form-error" role="alert">
          {stepUp ? (
            <StepUpPrompt signInAgain="/signin?return_to=%2Fadmin" />
          ) : (
            error
          )}
        </p>
      )}
    </section>
  );
}

/** Staff and their roles (staff.rs). Changing a role is itself a sensitive action. */
function StaffTab() {
  const staff = useLoad(() => api.admin.staff(), []);
  const history = useLoad(() => api.admin.actions(true), []);
  const { busy, error, notice, stepUp, run } = useAction(() => {
    staff.reload();
    history.reload();
  });
  const [newId, setNewId] = useState("");
  const [newIdError, setNewIdError] = useState<string>();
  const [newRole, setNewRole] = useState<StaffRole>("reviewer");
  if (staff.error)
    return <ErrorState error={staff.error} onRetry={staff.reload} />;
  if (!staff.data) return <Loading />;
  const s = staff.data;
  const canManage =
    s.my_roles.includes("owner") || s.my_roles.includes("security");
  const ROLE_HELP: Record<StaffRole, string> = {
    owner: "everything",
    moderator: "reviews, reports, appeals and sensitive actions",
    reviewer: "releases, changes, pictures, portfolios and listings",
    support: "reports, appeals and People problems",
    finance: "payments",
    security: "tokens, staff roles and the audit log",
  };
  return (
    <>
      <p className="muted">
        Sensitive actions (withdrawing a release, taking an app out of the
        store, suspending or reinstating an account, changing roles) need{" "}
        <b>{s.approvals_required}</b> different staff member
        {s.approvals_required === 1 ? "" : "s"}
        {s.approvals_required === 1
          ? ": they happen at once but are recorded as approvals. Set SLOP_APPROVALS_REQUIRED=2 on the server once there's a second person."
          : ", counting the one who asks."}
      </p>
      <ul className="small">
        {s.roles.map((r) => (
          <li key={r}>
            <b>{r}</b>: {ROLE_HELP[r]}
          </li>
        ))}
      </ul>
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {stepUp ? (
            <StepUpPrompt signInAgain="/signin?return_to=%2Fadmin%3Ftab%3Dstaff" />
          ) : (
            error
          )}
        </p>
      )}
      <table className="kv-table">
        <thead>
          <tr>
            <th scope="col">Staff</th>
            {s.roles.map((r) => (
              <th scope="col" key={r}>
                {r}
              </th>
            ))}
            <th scope="col">Name shown to AI agents</th>
            {canManage && <th scope="col">Remove</th>}
          </tr>
        </thead>
        <tbody>
          {s.staff.map((m) => (
            <tr key={m.id}>
              <td>
                <Link to={`/admin?tab=users&user=${m.id}`}>{m.display_name}</Link>
                {m.suspended && <span className="badge bad">SUSPENDED</span>}
              </td>
              {s.roles.map((r) => {
                const on = m.roles.includes(r);
                return (
                  <td key={r}>
                    <input
                      type="checkbox"
                      aria-label={`${m.display_name}: ${r}`}
                      checked={on}
                      disabled={
                        !canManage ||
                        busy !== undefined ||
                        (r === "owner" && !s.my_roles.includes("owner"))
                      }
                      onChange={() =>
                        void run(`${m.id}/${r}`, () =>
                          api.admin.changeRole(m.id, r, !on),
                        )
                      }
                    />
                  </td>
                );
              })}
              <td>
                <input
                  type="checkbox"
                  aria-label={`${m.display_name}: name shown to AI agents`}
                  checked={m.listed_by_name}
                  disabled={busy !== undefined || (m.id !== s.me && !s.my_roles.includes("owner"))}
                  onChange={() => void run(`${m.id}/listed`, () => api.admin.setListedByName(m.id, !m.listed_by_name))}
                />
              </td>
              {canManage && (
                <td>
                  <ReasonButton
                    label={`Remove ${m.display_name}`}
                    placeholder="Why (10+ characters): every role and API token ends"
                    danger
                    disabled={busy !== undefined}
                    onSubmit={(reason) => void run(`${m.id}/remove`, () => api.admin.removeStaff(m.id, reason))}
                  />
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {canManage && (
        <form
          className="reason-form"
          onSubmit={(e) => {
            e.preventDefault();
            const id = accountId(newId);
            setNewIdError(
              id ? undefined : "Paste the account's ID, or the link to their profile (…/u/…).",
            );
            if (!id) return;
            void run("new", () => api.admin.changeRole(id, newRole, true)).then(
              () => setNewId(""),
            );
          }}
        >
          <input
            aria-label="Account ID or profile link"
            placeholder="Account ID or profile link"
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
          />
          <select
            aria-label="Role"
            value={newRole}
            onChange={(e) => setNewRole(e.target.value as StaffRole)}
          >
            {s.roles.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <button
            className="btn small"
            disabled={!newId.trim() || busy !== undefined}
          >
            Add to staff
          </button>
          {newIdError && (
            <p className="form-error" role="alert">
              {newIdError}
            </p>
          )}
        </form>
      )}
      <h2 className="section-title">Sensitive actions</h2>
      {history.data?.actions.length === 0 && (
        <div className="empty-state">None yet.</div>
      )}
      <ul className="release-list">
        {history.data?.actions.map((a) => (
          <li key={a.id} className="small">
            <span
              className={`badge ${a.status === "done" ? "official" : a.status === "pending" ? "soon" : "neutral"}`}
            >
              {a.status.toUpperCase()}
            </span>{" "}
            {a.summary}{" "}
            <span className="muted">
              · {a.approvals.join(", ")} · {formatDate(a.requested_at)}
            </span>
          </li>
        ))}
      </ul>
    </>
  );
}

/** Alerts from ops.rs; each admin also gets them under the 🔔 and by push. */
function OpsAlerts() {
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

function ServerChecks({ checks }: { checks: AdminQueue["checks"] }) {
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

const STATUSES: Array<AppStatus | ""> = ["", "draft", "listed", "delisted"];

function AppsTab() {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const [text, setText] = useState(q);
  const [status, setStatus] = useState<AppStatus | "">("");
  const apps = useLoad(() => api.admin.apps(status || undefined, q), [status, q]);
  // The Content Policy rules, for the statement of reasons when taking an app out.
  const rules = useLoad(() => api.admin.queue().then((q) => q.rules), []);
  const { busy, error, notice, run } = useAction(apps.reload);

  return (
    <>
      <form
        className="filters"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          setParams({ tab: "apps", q: text.trim() }, { replace: true });
        }}
      >
        <label>
          Find{" "}
          <input value={text} placeholder="Title, address or maker" onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn small">Search</button>
        <label>
          Show{" "}
          <select
            value={status}
            onChange={(e) => setStatus(e.target.value as AppStatus | "")}
          >
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {s ? s[0].toUpperCase() + s.slice(1) : "All apps"}
              </option>
            ))}
          </select>
        </label>
      </form>
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {apps.error && <ErrorState error={apps.error} onRetry={apps.reload} />}
      {!apps.data && !apps.error && <Loading />}
      {apps.data?.length === 0 && <div className="empty-state">No apps.</div>}
      <ul className="release-list">
        {apps.data?.map((a: AdminApp) => (
          <li key={a.slug} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <Link to={`/admin?tab=apps&app=${encodeURIComponent(a.slug)}`}>
                  <b>{a.title}</b>
                </Link>
                <span className="muted">by {a.maker}</span>
                <Link className="small" to={`/app/${a.slug}`}>
                  store page
                </Link>
                <span
                  className={`badge ${a.status === "listed" ? "official" : a.status === "delisted" ? "bad" : "neutral"}`}
                >
                  {a.status.toUpperCase()}
                </span>
              </div>
              <label className="small">
                Stage{" "}
                <select
                  value={a.stage}
                  disabled={busy !== undefined}
                  onChange={(e) =>
                    void run(a.slug, () =>
                      api.admin.stage(a.slug, e.target.value as AppStage),
                    )
                  }
                >
                  <option value="coming_soon">Coming soon</option>
                  <option value="beta">Beta</option>
                  <option value="released">Released</option>
                </select>
              </label>
            </div>
            <div className="row-actions">
              {a.status === "listed" ? (
                <DelistButton
                  rules={rules.data ?? []}
                  disabled={busy !== undefined}
                  onSubmit={(rule, reason) =>
                    void run(a.slug, () =>
                      api.admin.delist(a.slug, rule, reason),
                    )
                  }
                />
              ) : a.stage !== "coming_soon" && a.releases.length === 0 ? (
                <span className="muted small">
                  {a.pending_releases > 0
                    ? "Can't list yet: approve its release under Releases to review first."
                    : "Can't list yet: the maker hasn't submitted a release. Set the stage to Coming soon to list the page without one."}
                </span>
              ) : (
                <button
                  className="btn small"
                  disabled={busy !== undefined}
                  onClick={() => void run(a.slug, () => api.admin.list(a.slug))}
                >
                  Put in the store
                </button>
              )}
              {a.releases.map((r) => (
                <ReasonButton
                  key={r.id}
                  label={`Withdraw ${r.version}`}
                  placeholder="Reason, shown to players (stops it launching)"
                  danger
                  disabled={busy !== undefined}
                  onSubmit={(reason) =>
                    void run(r.id, () => api.admin.revoke(r.id, reason))
                  }
                />
              ))}
            </div>
          </li>
        ))}
      </ul>
    </>
  );
}

function AuditTab() {
  const [text, setText] = useState("");
  const [q, setQ] = useState("");
  const [pages, setPages] = useState<AuditEntry[][]>([]);
  const first = useLoad(() => api.admin.audit(q), [q]);
  const [more, setMore] = useState<"loading" | "done" | undefined>();
  if (first.error) return <ErrorState error={first.error} onRetry={first.reload} />;
  if (!first.data) return <Loading />;
  const rows = [first.data, ...pages].flat();
  const older = async () => {
    setMore("loading");
    const page = await api.admin.audit(q, rows[rows.length - 1]?.id);
    setPages((p) => [...p, page]);
    setMore(page.length < 100 ? "done" : undefined);
  };
  return (
    <>
      <form
        className="filters"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          setPages([]);
          setMore(undefined);
          setQ(text.trim());
        }}
      >
        <label>
          Find{" "}
          <input value={text} placeholder="Who, what (e.g. donations.hold) or an app's address" onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn small">Search</button>
      </form>
      {rows.length === 0 ? (
        <div className="empty-state">Nothing recorded{q && " that matches"}.</div>
      ) : (
        <AuditRows rows={rows} />
      )}
      {first.data.length === 100 && more !== "done" && (
        <button className="btn small ghost" disabled={more === "loading"} onClick={() => void older()}>
          Older entries
        </button>
      )}
    </>
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
  return (
    <div className="media-review">
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
      {!set.thumbnail && set.gallery.length === 0 && (
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
function Notes({ notes }: { notes: ReviewNote[] }) {
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

function TokensTab() {
  const tokens = useLoad(() => api.admin.tokens(), []);
  const { busy, error, run } = useAction(tokens.reload);
  const [name, setName] = useState("");
  const [scope, setScope] = useState<AdminToken["scope"]>("read");
  const [days, setDays] = useState(90);
  const [created, setCreated] = useState<string>();
  const origin = window.location.origin;
  const prompt = created
    ? `You help run the Slop store for me. Read ${origin}/v1/admin/review/guide and follow it. ` +
      `Send this header with every request: "Authorization: Bearer ${created}". ` +
      `Start with GET ${origin}/v1/admin/status, then go through GET ${origin}/v1/admin/review. ` +
      `Only suspend accounts, withdraw releases or take apps out of the store when I ask you to.`
    : "";
  // Claude Code (or any MCP client) gets the same API as tools.
  const mcp = created
    ? `claude mcp add --transport http slop-admin ${origin}/v1/admin/mcp --header "Authorization: Bearer ${created}"`
    : "";

  return (
    <>
      <p className="muted">
        An AI reviewer gets an API token that acts for you on the admin API. It
        reads everything waiting (releases, change requests, pictures, apps
        ready for the store) at <code>/v1/admin/review</code>, follows the guide
        at <code>/v1/admin/review/guide</code>, and leaves notes you see in the
        review queue. It can watch the server at <code>/v1/admin/status</code>{" "}
        (alerts, queues, backups) and, at your request, act on reports and
        accounts. A <b>read</b> token can only look and leave notes; a{" "}
        <b>review</b> token can also approve and reject; a <b>moderate</b> token
        can also suspend and reinstate accounts, withdraw releases and take apps
        out of the store. Everything it does is in the audit log under the
        token's name. Agents that speak MCP (such as Claude) can connect to{" "}
        <code>/v1/admin/mcp</code> and get all of this as tools.
      </p>
      <form
        className="panel panel-pad form"
        onSubmit={(e) => {
          e.preventDefault();
          void run("create", async () => {
            const t = await api.admin.createToken({ name, scope, days });
            setCreated(t.token);
            setName("");
          });
        }}
      >
        <h2>New token</h2>
        <div className="row-2">
          <label>
            Name
            <input
              value={name}
              maxLength={60}
              required
              placeholder="e.g. Claude, picture checks"
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            Can
            <select
              value={scope}
              onChange={(e) => setScope(e.target.value as AdminToken["scope"])}
            >
              <option value="read">Look and leave notes (read)</option>
              <option value="review">Also approve and reject (review)</option>
              <option value="moderate">
                Also suspend accounts and withdraw releases (moderate)
              </option>
            </select>
          </label>
        </div>
        <label>
          Expires after
          <select
            value={days}
            onChange={(e) => setDays(Number(e.target.value))}
          >
            {[7, 30, 90, 365].map((d) => (
              <option key={d} value={d}>
                {d} days
              </option>
            ))}
          </select>
        </label>
        <div className="row-actions">
          <button
            className="btn slime"
            disabled={busy !== undefined || !name.trim()}
          >
            Make token
          </button>
        </div>
      </form>
      {created && (
        <div className="panel panel-pad ai-notice">
          <b>Copy it now: Slop won't show this token again.</b>
          <pre className="code-block">{created}</pre>
          <p className="small">Give your AI this prompt:</p>
          <pre className="code-block ai-prompt">{prompt}</pre>
          <p className="small">
            Or connect Claude Code to Slop's admin tools (MCP):
          </p>
          <pre className="code-block ai-prompt">{mcp}</pre>
          <div className="row-actions">
            <button
              className="btn small"
              onClick={() => void navigator.clipboard.writeText(prompt)}
            >
              Copy prompt
            </button>
            <button
              className="btn small"
              onClick={() => void navigator.clipboard.writeText(mcp)}
            >
              Copy MCP command
            </button>
            <button
              className="btn small ghost"
              onClick={() => setCreated(undefined)}
            >
              Done
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <h2 className="section-title">Tokens</h2>
      {tokens.error && (
        <ErrorState error={tokens.error} onRetry={tokens.reload} />
      )}
      {tokens.data?.length === 0 && (
        <div className="empty-state">No tokens yet.</div>
      )}
      <ul className="release-list">
        {tokens.data?.map((t) => {
          const expired = new Date(t.expires_at) < new Date();
          return (
            <li key={t.id} className="panel panel-pad title-row spread">
              <div>
                <div className="title-row">
                  <b>{t.name}</b>
                  <span
                    className={`badge ${t.scope === "moderate" ? "bad" : t.scope === "review" ? "soon" : "neutral"}`}
                  >
                    {t.scope.toUpperCase()}
                  </span>
                  {t.revoked_at ? (
                    <span className="badge bad">REVOKED</span>
                  ) : (
                    expired && <span className="badge bad">EXPIRED</span>
                  )}
                </div>
                <div className="muted small">
                  for {t.admin} · made {formatDate(t.created_at)} · expires{" "}
                  {formatDate(t.expires_at)} ·{" "}
                  {t.last_used_at
                    ? `last used ${formatDate(t.last_used_at)}`
                    : "never used"}
                </div>
              </div>
              {!t.revoked_at && !expired && (
                <button
                  className="btn small ghost danger"
                  disabled={busy !== undefined}
                  onClick={() =>
                    void run(t.id, () => api.admin.revokeToken(t.id))
                  }
                >
                  Revoke
                </button>
              )}
            </li>
          );
        })}
      </ul>
    </>
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

function ReportsSection({ q, onDone }: { q: AdminQueue; onDone: () => void }) {
  const overdue = q.reports.filter((r) => r.overdue).length;
  return (
    <>
      <h2 className="section-title" id="reports">
        Reports ({q.reports.length})
        {overdue > 0 && <span className="badge bad"> {overdue} OVERDUE</span>}
      </h2>
      <p className="muted small">
        Urgent first, then by when they're due: urgent and serious reports
        within 24 hours, the rest within 7 days. Every action sends the person a
        statement of reasons they can appeal; never hide something for being
        negative.
      </p>
      {q.reports.length === 0 && (
        <div className="empty-state">No open reports.</div>
      )}
      <ul className="release-list">
        {q.reports.map((r) => (
          <ReportCard key={r.id} r={r} rules={q.rules} onDone={onDone} />
        ))}
      </ul>
    </>
  );
}

function ReportCard({
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

function AppealsSection({ q, onDone }: { q: AdminQueue; onDone: () => void }) {
  const [notice, setNotice] = useState<string>();
  return (
    <>
      <h2 className="section-title" id="appeals">
        Appeals ({q.appeals.length})
      </h2>
      <p className="muted small">
        Look at the decision again with fresh eyes, ideally not the person who
        made it. Keeping or reversing it, the person is told why.
      </p>
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {q.appeals.length === 0 && (
        <div className="empty-state">No appeals waiting.</div>
      )}
      <ul className="release-list">
        {q.appeals.map((a) => (
          <AppealCard
            key={a.id}
            a={a}
            onDone={onDone}
            onNotice={setNotice}
          />
        ))}
      </ul>
    </>
  );
}

function AppealCard({
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
