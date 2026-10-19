// The admin panel for Slop staff: review releases, list and moderate apps, read the audit log.
// Every action is recorded in the audit log under the admin's name (crates/slop-api/src/admin_web.rs).

import { useState } from "react";
import { Link, useSearchParams } from "react-router";

import { api, ApiError, type AdminApp, type AdminQueue, type AppStage, type AppStatus } from "../api";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { formatDate, STAGE_LABEL } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { NotFound } from "./Listing";

type Tab = "queue" | "apps" | "audit";

export function Admin() {
  useDocumentTitle("Admin");
  const { me } = useSession();
  const [params, setParams] = useSearchParams();
  const tab = (params.get("tab") as Tab | null) ?? "queue";

  if (me === undefined) return <Loading />;
  if (!me?.is_admin) return <NotFound />;

  return (
    <div className="page admin">
      <h1 className="page-title">Admin</h1>
      <div className="seg" role="tablist">
        {(
          [
            ["queue", "Review queue"],
            ["apps", "Apps"],
            ["audit", "Audit log"],
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
      {tab === "queue" && <QueueTab />}
      {tab === "apps" && <AppsTab />}
      {tab === "audit" && <AuditTab />}
    </div>
  );
}

/** Runs an admin action, showing its error (e.g. "approve a release first") if refused. */
function useAction(onDone: () => void) {
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();
  const run = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    setError(undefined);
    try {
      await action();
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(undefined);
    }
  };
  return { busy, error, run };
}

function ReasonButton({
  label,
  placeholder,
  danger,
  disabled,
  onSubmit,
}: {
  label: string;
  placeholder: string;
  danger?: boolean;
  disabled?: boolean;
  onSubmit: (reason: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  if (!open) {
    return (
      <button className={`btn small ${danger ? "ghost danger" : ""}`} disabled={disabled} onClick={() => setOpen(true)}>
        {label}
      </button>
    );
  }
  return (
    <form
      className="reason-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (reason.trim()) onSubmit(reason.trim());
      }}
    >
      <input aria-label={`${label}: reason`} autoFocus value={reason} placeholder={placeholder} onChange={(e) => setReason(e.target.value)} />
      <button className={`btn small ${danger ? "danger" : ""}`} disabled={disabled || !reason.trim()}>
        {label}
      </button>
      <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
        Cancel
      </button>
    </form>
  );
}

function QueueTab() {
  const queue = useLoad(() => api.admin.queue(), []);
  const { busy, error, run } = useAction(queue.reload);
  if (queue.error) return <ErrorState error={queue.error} onRetry={queue.reload} />;
  if (!queue.data) return <Loading />;
  const q: AdminQueue = queue.data;

  return (
    <>
      <ServerChecks checks={q.checks} />
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      <h2 className="section-title">Releases to review ({q.releases.length})</h2>
      <p className="muted small">
        Approving a release signs it for players. A Beta or Released app needs one approved release before it can go in
        the store.
      </p>
      {q.releases.length === 0 && <div className="empty-state">Nothing waiting for review.</div>}
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
                {r.commit_verified ? <span className="badge official">FOUND IN REPO</span> : <span className="badge warn">NOT CHECKED</span>}
              </dd>
              {r.files.map((f) => (
                <FileRow key={f.platform} file={f} />
              ))}
            </dl>
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
        ))}
      </ul>

      <h2 className="section-title">Changes to review ({q.changes.length})</h2>
      <p className="muted small">
        Makers asking to change what players download: holding players at a different release, or moving the app to
        another repository. Approving makes the change straight away.
      </p>
      {q.changes.length === 0 && <div className="empty-state">No changes waiting.</div>}
      <ul className="release-list">
        {q.changes.map((c) => (
          <li key={c.id} className="panel panel-pad">
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
            {c.kind === "repo" && (
              <p className="muted small">
                Check the new repository belongs to the same maker and project. Its first release will come back here for
                review.
              </p>
            )}
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
        ))}
      </ul>

      <h2 className="section-title">Ready for the store ({q.ready_to_list.length})</h2>
      {q.ready_to_list.length === 0 && <div className="empty-state">No drafts are ready to list.</div>}
      <ul className="release-list">
        {q.ready_to_list.map((a) => (
          <li key={a.slug} className="panel panel-pad title-row spread">
            <div className="title-row">
              <Link to={`/app/${a.slug}`}>
                <b>{a.title}</b>
              </Link>
              <span className="muted">
                {STAGE_LABEL[a.stage]} · by {a.maker} · {a.approved_releases} approved release
                {a.approved_releases === 1 ? "" : "s"}
              </span>
            </div>
            <button
              className="btn slime small"
              disabled={busy !== undefined}
              onClick={() => void run(a.slug, () => api.admin.list(a.slug))}
            >
              {busy === a.slug ? "Listing…" : "Put in the store"}
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}

function FileRow({ file }: { file: AdminQueue["releases"][number]["files"][number] }) {
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
              SHA-256 <code>{file.sha256.slice(0, 16)}…</code> · {file.size_bytes?.toLocaleString()} bytes ·{" "}
            </span>
          ) : (
            <span className="faint">not hashed yet · </span>
          )}
          {file.provenance ? (
            <span className="badge official">{file.provenance.built_by_slop_build ? "SLOP BUILD" : "SIGNED"}</span>
          ) : (
            <span className="badge warn" title={file.provenance_problem ?? ""}>
              NOT SIGNED
            </span>
          )}{" "}
          <span className={`badge ${file.scan_result === "passed" ? "official" : file.scan_result === "skipped" ? "neutral" : "bad"}`}>
            {file.scan_result === "skipped" ? "NOT SCANNED" : `SCAN: ${(file.scan_result ?? "pending").toUpperCase()}`}
          </span>
        </div>
      </dd>
    </>
  );
}

function ServerChecks({ checks }: { checks: AdminQueue["checks"] }) {
  const notes = [
    !checks.signing_key && "No signing key is set (SLOP_SIGNING_KEY), so nothing can be approved.",
    !checks.scanning && "Scanning is off: files are not scanned.",
    !checks.maker_signatures_required && "Maker signatures aren't required on this server.",
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
  const [status, setStatus] = useState<AppStatus | "">("");
  const apps = useLoad(() => api.admin.apps(status || undefined), [status]);
  const { busy, error, run } = useAction(apps.reload);

  return (
    <>
      <div className="filters">
        <label>
          Show{" "}
          <select value={status} onChange={(e) => setStatus(e.target.value as AppStatus | "")}>
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {s ? s[0].toUpperCase() + s.slice(1) : "All apps"}
              </option>
            ))}
          </select>
        </label>
      </div>
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
                <Link to={`/app/${a.slug}`}>
                  <b>{a.title}</b>
                </Link>
                <span className="muted">by {a.maker}</span>
                <span className={`badge ${a.status === "listed" ? "official" : a.status === "delisted" ? "bad" : "neutral"}`}>
                  {a.status.toUpperCase()}
                </span>
              </div>
              <label className="small">
                Stage{" "}
                <select
                  value={a.stage}
                  disabled={busy !== undefined}
                  onChange={(e) => void run(a.slug, () => api.admin.stage(a.slug, e.target.value as AppStage))}
                >
                  <option value="coming_soon">Coming soon</option>
                  <option value="beta">Beta</option>
                  <option value="released">Released</option>
                </select>
              </label>
            </div>
            <div className="row-actions">
              {a.status === "listed" ? (
                <ReasonButton
                  label="Take out of the store"
                  placeholder="Reason (kept in the audit log)"
                  danger
                  disabled={busy !== undefined}
                  onSubmit={(reason) => void run(a.slug, () => api.admin.delist(a.slug, reason))}
                />
              ) : a.stage !== "coming_soon" && a.releases.length === 0 ? (
                <span className="muted small">
                  {a.pending_releases > 0
                    ? "Can't list yet: approve its release under Releases to review first."
                    : "Can't list yet: the maker hasn't submitted a release. Set the stage to Coming soon to list the page without one."}
                </span>
              ) : (
                <button className="btn small" disabled={busy !== undefined} onClick={() => void run(a.slug, () => api.admin.list(a.slug))}>
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
                  onSubmit={(reason) => void run(r.id, () => api.admin.revoke(r.id, reason))}
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
  const log = useLoad(() => api.admin.audit(), []);
  if (log.error) return <ErrorState error={log.error} onRetry={log.reload} />;
  if (!log.data) return <Loading />;
  if (log.data.length === 0) return <div className="empty-state">Nothing recorded yet.</div>;
  return (
    <table className="audit">
      <thead>
        <tr>
          <th>When</th>
          <th>Who</th>
          <th>What</th>
          <th>Details</th>
        </tr>
      </thead>
      <tbody>
        {log.data.map((e) => (
          <tr key={e.id}>
            <td>{new Date(e.at).toLocaleString()}</td>
            <td>{e.actor}</td>
            <td>
              <code>{e.action}</code> {e.subject}
            </td>
            <td className="muted small">
              {Object.entries(e.detail)
                .map(([k, v]) => `${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`)
                .join(" · ")}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
