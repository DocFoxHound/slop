// Manage an app: which release players get, the repository, update settings and the requests
// that change them (crates/slop-api/src/manage.rs). Anything that would have players download
// something different is checked before it happens; everything else the maker changes directly.

import { useState, type FormEvent } from "react";
import { Link, useParams } from "react-router";

import { api, ApiError, type AppChange, type ManageOverview } from "../api";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { UpdatesPanel } from "./Releases";

/** The value of the "follow the newest" choice in the release list. */
const NEWEST = "newest";

export function Manage() {
  const { slug = "" } = useParams();
  const { me } = useSession();
  const app = useLoad(() => (me ? api.app(slug) : Promise.resolve(undefined)), [slug, me?.id]);
  const overview = useLoad(() => (me ? api.manage(slug) : Promise.resolve(undefined)), [slug, me?.id]);
  useDocumentTitle(app.data ? `Manage ${app.data.title}` : "Manage");
  const [settingsNonce, setSettingsNonce] = useState(0);

  if (me === undefined || app.loading) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <p>
          <Link to={`/signin?return_to=/publish/${slug}/manage`}>Sign in</Link> to manage your app.
        </p>
      </div>
    );
  }
  if (app.error || (app.data && !app.data.is_mine)) {
    return (
      <div className="page">
        <ErrorState error={app.error ?? new ApiError(404, "not_found", "")} />
      </div>
    );
  }
  if (overview.error) {
    return (
      <div className="page">
        <ErrorState error={overview.error} onRetry={overview.reload} />
      </div>
    );
  }
  if (!app.data || !overview.data) return <Loading />;
  const m = overview.data;
  const changed = () => {
    overview.reload();
    setSettingsNonce((n) => n + 1);
  };

  return (
    <div className="page narrow manage">
      <div className="crumbs">
        <Link to="/publish">Your apps</Link> › <Link to={`/app/${slug}`}>{app.data.title}</Link> › Manage
      </div>
      <div className="title-row spread">
        <h1 className="page-title">Manage {app.data.title}</h1>
        <div className="row-actions">
          <Link className="btn small" to={`/publish/${slug}`}>
            Edit store page
          </Link>
          <Link className="btn small" to={`/publish/${slug}/releases`}>
            Releases &amp; progress
          </Link>
        </div>
      </div>
      <p className="muted">
        Changes that would have players download something different, like switching the release they get or moving the
        app to another repository, are checked before they happen. Everything else (the store page, platforms, update
        settings) changes straight away.
      </p>

      <ReleaseChoice slug={slug} overview={m} onChanged={changed} />
      <UpdatesPanel key={settingsNonce} slug={slug} />
      <RepoMove slug={slug} overview={m} onChanged={changed} />
      <Requests slug={slug} changes={m.changes} onChanged={changed} />
    </div>
  );
}

/** Runs a request, keeping its error for the form to show. */
function useSubmit(onDone: () => void) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<AppChange>();
  const submit = async (action: () => Promise<AppChange | void>) => {
    setBusy(true);
    setError(undefined);
    setResult(undefined);
    try {
      const change = await action();
      if (change) setResult(change);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, result, submit };
}

function Outcome({ change }: { change: AppChange }) {
  return change.status === "approved" ? (
    <p className="notice">Done. {change.status_reason}</p>
  ) : (
    <p className="notice">Sent for review. {change.status_reason} You'll see the decision below.</p>
  );
}

/** Which approved release players get: the newest, or one the maker holds them at. */
function ReleaseChoice({ slug, overview: m, onChanged }: { slug: string; overview: ManageOverview; onChanged: () => void }) {
  const current = m.held && m.serving ? m.serving.id : NEWEST;
  const [choice, setChoice] = useState(current);
  const [note, setNote] = useState("");
  const { busy, error, result, submit } = useSubmit(() => {
    setNote("");
    onChanged();
  });
  const pending = m.changes.find((c) => c.kind === "release" && c.status === "pending");
  const newest = m.approved[0];

  if (m.approved.length === 0) {
    return (
      <section className="panel panel-pad">
        <h2>What players get</h2>
        <p className="muted">
          Nothing yet: no release has been approved. <Link to={`/publish/${slug}/releases`}>Submit a release</Link> first.
        </p>
      </section>
    );
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void submit(() =>
      api.requestChange(slug, { kind: "release", release_id: choice === NEWEST ? null : choice, note: note || undefined }),
    );
  };
  // What players would get after the change, to say whether it's checked first.
  const after = choice === NEWEST ? newest?.id : choice;
  const downloads = after !== m.serving?.id;

  return (
    <section className="panel panel-pad">
      <h2>What players get</h2>
      <p>
        Players get <b>v{m.serving?.version ?? "—"}</b>{" "}
        {m.held ? <span className="badge soon">HELD AT THIS RELEASE</span> : <span className="badge official">NEWEST APPROVED</span>}
      </p>
      {m.held && newest && newest.id !== m.serving?.id && (
        <p className="notice">
          v{newest.version} is approved but isn't going out, because you're holding players at v{m.serving?.version}.
        </p>
      )}
      {pending && (
        <p className="notice">
          Waiting for review: {pending.release_id ? `hold players at v${pending.release_version}` : "go back to the newest release"}.
        </p>
      )}

      <form className="form" onSubmit={onSubmit}>
        <div className="choice-list" role="radiogroup" aria-label="Release players get">
          <label className="check">
            <input type="radio" name="release" checked={choice === NEWEST} onChange={() => setChoice(NEWEST)} />
            <span>
              <b>Always the newest approved release</b>{" "}
              <span className="muted small">(recommended){newest ? ` · now v${newest.version}` : ""}</span>
              <span className="muted small" style={{ display: "block" }}>
                Each update goes out as soon as it's approved.
              </span>
            </span>
          </label>
          {m.approved.map((r) => (
            <label key={r.id} className="check">
              <input type="radio" name="release" checked={choice === r.id} onChange={() => setChoice(r.id)} />
              <span>
                Hold players at <b>v{r.version}</b>{" "}
                <span className="muted small">
                  · approved {r.reviewed_at ? formatDate(r.reviewed_at) : ""}
                  {r.automatic ? " automatically" : ""}
                  {r.id === m.serving?.id ? " · players get this now" : ""}
                </span>
              </span>
            </label>
          ))}
        </div>
        {choice !== current && (
          <>
            <label>
              Why (optional, for the reviewer)
              <input value={note} maxLength={500} onChange={(e) => setNote(e.target.value)} placeholder="e.g. 1.2 crashes on start" />
            </label>
            <p className="muted small">
              {!downloads
                ? "Players already get this release, so this changes straight away."
                : m.auto_approval_ready
                  ? "Players' apps will show an update notice. This release already passed every check, so it goes out straight away."
                  : `Players' apps would show an update notice, so a person checks this first: ${m.auto_approval_note}.`}
            </p>
          </>
        )}
        {error && <p className="form-error">{error}</p>}
        {result && <Outcome change={result} />}
        <div className="row-actions">
          <button className="btn slime" disabled={busy || choice === current}>
            {busy ? "Sending…" : !downloads || m.auto_approval_ready ? "Switch" : "Send for review"}
          </button>
        </div>
      </form>
    </section>
  );
}

/** Moving the app to another repository. */
function RepoMove({ slug, overview: m, onChanged }: { slug: string; overview: ManageOverview; onChanged: () => void }) {
  const [repo, setRepo] = useState("");
  const [note, setNote] = useState("");
  const { busy, error, result, submit } = useSubmit(() => {
    setRepo("");
    setNote("");
    onChanged();
  });
  const pending = m.changes.find((c) => c.kind === "repo" && c.status === "pending");

  return (
    <section className="panel panel-pad">
      <h2>Repository</h2>
      <p>
        <ExternalLink href={m.repo_url}>{m.repo_url.replace(/^https:\/\//, "")}</ExternalLink>
      </p>
      {!m.repo_locked ? (
        <p className="muted small">
          No one has this app yet, so you can change the repository on the{" "}
          <Link to={`/publish/${slug}`}>store page</Link> or here, straight away.
        </p>
      ) : (
        <p className="muted small">
          Players already have this app, so a move is always checked by a person, and the first release from the new
          repository is reviewed by a person too. Players who installed it from the old repository will be asked to
          reinstall before their Slop app takes updates from the new one.
        </p>
      )}
      {pending && <p className="notice">Waiting for review: move to {pending.repo_url}.</p>}
      <form
        className="form"
        onSubmit={(e) => {
          e.preventDefault();
          void submit(() => api.requestChange(slug, { kind: "repo", repo_url: repo, note: note || undefined }));
        }}
      >
        <label>
          Move to
          <input type="url" required value={repo} placeholder="https://github.com/you/new-repo" onChange={(e) => setRepo(e.target.value)} />
        </label>
        {m.repo_locked && (
          <label>
            Why (for the reviewer)
            <input value={note} maxLength={500} onChange={(e) => setNote(e.target.value)} placeholder="e.g. moved to our organisation's account" />
          </label>
        )}
        {error && <p className="form-error">{error}</p>}
        {result && <Outcome change={result} />}
        <div className="row-actions">
          <button className="btn" disabled={busy || !repo.trim()}>
            {busy ? "Sending…" : m.repo_locked ? "Request the move" : "Move"}
          </button>
        </div>
      </form>
    </section>
  );
}

const CHANGE_BADGE: Record<AppChange["status"], { label: string; badge: string }> = {
  pending: { label: "WAITING FOR REVIEW", badge: "soon" },
  approved: { label: "DONE", badge: "official" },
  rejected: { label: "TURNED DOWN", badge: "bad" },
  cancelled: { label: "CANCELLED", badge: "neutral" },
};

export function describeChange(c: Pick<AppChange, "kind" | "release_id" | "release_version" | "repo_url">): string {
  if (c.kind === "repo") return `Move to ${c.repo_url}`;
  return c.release_id ? `Hold players at v${c.release_version}` : "Follow the newest approved release";
}

function Requests({ slug, changes, onChanged }: { slug: string; changes: AppChange[]; onChanged: () => void }) {
  const { busy, error, submit } = useSubmit(onChanged);
  return (
    <>
      <h2 className="section-title">Requests</h2>
      {changes.length === 0 && <div className="empty-state">No changes requested yet.</div>}
      {error && <p className="form-error">{error}</p>}
      <ul className="release-list">
        {changes.map((c) => (
          <li key={c.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <b>{describeChange(c)}</b>
                <span className={`badge ${CHANGE_BADGE[c.status].badge}`}>
                  {c.status === "approved" && c.automatic ? "DONE AUTOMATICALLY" : CHANGE_BADGE[c.status].label}
                </span>
              </div>
              <span className="muted small">{formatDate(c.created_at)}</span>
            </div>
            {c.note && <p className="muted small">Your note: {c.note}</p>}
            {c.status_reason && <p className={c.status === "rejected" ? "notice" : "muted small"}>{c.status_reason}</p>}
            {c.status === "pending" && (
              <div className="row-actions">
                <button className="btn small" disabled={busy} onClick={() => void submit(() => api.cancelChange(slug, c.id))}>
                  Cancel request
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>
    </>
  );
}
