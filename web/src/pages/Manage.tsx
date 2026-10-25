// SPDX-License-Identifier: AGPL-3.0-or-later
// Manage an app: which release players get, the repository, update settings and the requests
// that change them (crates/slop-api/src/manage.rs). Anything that would have players download
// something different is checked before it happens; everything else the maker changes directly.

import { useState, type FormEvent } from "react";
import { Link, useParams } from "react-router";

import { api, ApiError, type AppChange, type ManageOverview, type RatingGroup, type WaitingRelease } from "../api";
import { OffersPanel } from "../components/Offers";
import { DemoPanel } from "../components/Demo";
import { BugReportsPanel, ReleaseFeedbackPanel } from "../components/BugReports";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { DonationsPanel } from "../components/DonationsManage";
import { RepoAccessPanel } from "../components/RepoToken";
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

      {m.frozen && <Frozen slug={slug} frozen={m.frozen} isPrivate={app.data.private} onChanged={changed} />}
      {m.pulse_required !== null && <PulseNeeded received={m.pulse_required} />}
      <WaitingReleases slug={slug} waiting={m.waiting} onChanged={changed} />
      <ReleaseChoice slug={slug} overview={m} onChanged={changed} />
      <UpdatesPanel key={settingsNonce} slug={slug} onChanged={overview.reload} />
      {!app.data.private && app.data.stage === "released" && <RatingBreakdown slug={slug} />}
      {app.data.private ? <GoPublic slug={slug} requested={m.public_requested} onChanged={changed} /> : <DonationsPanel slug={slug} />}
      <RepoMove slug={slug} overview={m} onChanged={changed} />
      {app.data.private && <RepoAccessPanel slug={slug} repoUrl={app.data.repo_url} />}
      <DemoPanel slug={slug} />
      <BugReportsPanel slug={slug} repoUrl={app.data.repo_url} accepting={app.data.bug_reports} />
      {!app.data.private && <ReleaseFeedbackPanel slug={slug} />}
      <OffersPanel slug={slug} originalSlug={app.data.branch_of} />
      <Requests slug={slug} changes={m.changes} onChanged={changed} />
    </div>
  );
}

/** Slop can't read the repository (SECURITY §12.2): no new releases, and out after 14 days. */
function Frozen({
  slug,
  frozen,
  isPrivate,
  onChanged,
}: {
  slug: string;
  frozen: NonNullable<ManageOverview["frozen"]>;
  isPrivate: boolean;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [still, setStill] = useState(false);
  const [error, setError] = useState<string>();
  const check = async () => {
    setBusy(true);
    setError(undefined);
    setStill(false);
    try {
      const after = await api.repoCheck(slug);
      if (after.frozen) setStill(true);
      else onChanged();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  if (frozen.by_staff)
    return (
      <section className="panel panel-pad notice-panel" role="status">
        <h2>Frozen by Slop</h2>
        <p>
          Since {formatDate(frozen.since)}, this app takes no new releases: {frozen.reason}
        </p>
        <p>It stays in the store and copies people installed keep working. Slop lifts the freeze once it's dealt with.</p>
      </section>
    );
  const past = new Date(frozen.until).getTime() < Date.now();
  return (
    <section className="panel panel-pad notice-panel" role="status">
      <h2>Frozen: Slop can't check the repository</h2>
      <p>
        Since {formatDate(frozen.since)}, {frozen.reason}. Until Slop can read it again, this app takes no new releases.
        Copies people installed keep working.
      </p>
      <p>
        {past
          ? isPrivate
            ? "It's no longer shared, since it stayed frozen for 14 days."
            : "It left the store, since it stayed frozen for 14 days. It comes back once Slop can read the repository."
          : isPrivate
            ? `If it's still frozen on ${formatDate(frozen.until)}, it stops being shared.`
            : `If it's still frozen on ${formatDate(frozen.until)}, it leaves the store.`}
      </p>
      <div className="row-actions">
        <button type="button" className="btn" disabled={busy} onClick={() => void check()}>
          {busy ? "Checking…" : "Check now"}
        </button>
        {still && <span className="muted small">Still can't read it. Check the repository's address and Slop's access.</span>}
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

const PLATFORM_NAME: Record<string, string> = { windows: "Windows", linux: "Linux" };

function GroupTable({ caption, groups, name }: { caption: string; groups: RatingGroup[]; name: (key: string | null) => string }) {
  return (
    <table className="kv-table">
      <caption className="sr-only">{caption}</caption>
      <thead>
        <tr>
          <th scope="col">{caption}</th>
          <th scope="col">Weighted</th>
          <th scope="col">Unweighted</th>
          <th scope="col">👍 / 👎</th>
        </tr>
      </thead>
      <tbody>
        {groups.map((g) => (
          <tr key={g.key ?? "none"}>
            <td>{name(g.key)}</td>
            <td>{g.percent_weighted !== null ? `${g.percent_weighted}%` : "—"}</td>
            <td>{g.percent !== null ? `${g.percent}%` : "—"}</td>
            <td>
              {g.not_slop.toLocaleString()} / {g.slop.toLocaleString()}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The maker's ratings by version and platform, and what each version did to votes (METRICS §3, §5). */
function RatingBreakdown({ slug }: { slug: string }) {
  const b = useLoad(() => api.ratingsBreakdown(slug), [slug]);
  return (
    <section className="panel panel-pad">
      <h2>Ratings</h2>
      <p className="muted small">
        Each player's current vote, grouped by the version and platform they voted on. Percentages show from 5 votes.{" "}
        <Link to="/ratings">How ratings work</Link>.
      </p>
      {b.error && <ErrorState error={b.error} onRetry={b.reload} />}
      {b.data && b.data.by_version.length === 0 && <p className="muted">No votes yet.</p>}
      {b.data && b.data.by_version.length > 0 && (
        <>
          <GroupTable caption="Version" groups={b.data.by_version} name={(k) => (k ? `v${k}` : "Before versions were recorded")} />
          <GroupTable caption="Platform" groups={b.data.by_platform} name={(k) => (k ? (PLATFORM_NAME[k] ?? k) : "Not recorded")} />
          {b.data.impact.some((i) => i.to_not_slop + i.to_slop > 0) && (
            <>
              <h3>Update impact</h3>
              <ul>
                {b.data.impact
                  .filter((i) => i.to_not_slop + i.to_slop + i.new_votes > 0)
                  .map((i) => (
                    <li key={i.version}>
                      On v{i.version}: {i.to_not_slop.toLocaleString()} changed 👎 → 👍, {i.to_slop.toLocaleString()} changed 👍 → 👎,{" "}
                      {i.new_votes.toLocaleString()} new votes.
                    </li>
                  ))}
              </ul>
            </>
          )}
        </>
      )}
    </section>
  );
}

/** Community builds of games whose original counts players with Slop Pulse need it too (DESIGN §9). */
function PulseNeeded({ received }: { received: boolean }) {
  return (
    <section className="panel panel-pad">
      <h2>Slop Pulse</h2>
      {received ? (
        <p>✓ Slop has had a Pulse handshake from this build, so it can be listed once a reviewer looks at it.</p>
      ) : (
        <p>
          The game this branches from counts players with Slop Pulse, so community builds of it must include it too, and a
          reviewer can't list this one before its first Pulse handshake. Keep the Pulse code from the original, then
          install your approved build through Slop and launch it once. See the{" "}
          <Link to="/publish/guide#slop-pulse-player-counts">publishing guide</Link>.
        </p>
      )}
    </section>
  );
}

/** Private Slop going public (SECURITY §12.3): a public repository, then a fresh review. */
function GoPublic({ slug, requested, onChanged }: { slug: string; requested: boolean; onChanged: () => void }) {
  const { busy, error, submit } = useSubmit(onChanged);
  return (
    <section className="panel panel-pad">
      <h2>Make it public</h2>
      {requested ? (
        <>
          <p>
            Waiting for a reviewer. They check the repository again from scratch; once they list it, it's in the store for
            everyone, and the people you shared it with keep it like anyone else. It stays private until then.
          </p>
          <button type="button" className="btn" disabled={busy} onClick={() => void submit(() => api.cancelMakePublic(slug))}>
            Keep it private
          </button>
        </>
      ) : (
        <>
          <p>
            Apps in the store need public source. Make the repository public under its open-source licence first, then ask
            here. A reviewer checks it again before it goes in the store.
          </p>
          <button type="button" className="btn" disabled={busy} onClick={() => void submit(() => api.makePublic(slug))}>
            Ask to make it public
          </button>
        </>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
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

/** Where a release not reviewed yet is, and where it came from. Exported for tests. */
export function describeWaiting(r: Pick<WaitingRelease, "status" | "source">): string {
  const where = r.status === "fetching" ? "Checking files" : "Waiting for review";
  const from = { maker: "submitted from your account", slop_build: "from Slop Build", github_release: "picked up from GitHub" }[
    r.source
  ];
  return `${where}, ${from}`;
}

/** Releases not reviewed yet: one the maker didn't send, or sent by mistake, can be withdrawn. */
function WaitingReleases({ slug, waiting, onChanged }: { slug: string; waiting: WaitingRelease[]; onChanged: () => void }) {
  const { busy, error, submit } = useSubmit(onChanged);
  if (waiting.length === 0) return null;
  const withdraw = (r: WaitingRelease) => {
    if (!window.confirm(`Withdraw v${r.version}? It won't be reviewed or reach players.`)) return;
    void submit(async () => {
      await api.withdrawRelease(slug, r.id);
    });
  };
  return (
    <section className="panel panel-pad">
      <h2>Not reviewed yet</h2>
      <p className="muted small">
        If you didn't send one of these, withdraw it and secure your accounts and repository. Your next release is then
        reviewed by a person.
      </p>
      {error && <p className="form-error">{error}</p>}
      <ul className="release-list">
        {waiting.map((r) => (
          <li key={r.id} className="title-row spread">
            <span>
              <b>v{r.version}</b> <span className="muted small">{describeWaiting(r)}, {formatDate(r.created_at)}</span>
            </span>
            <button type="button" className="btn small" disabled={busy} onClick={() => withdraw(r)}>
              Withdraw
            </button>
          </li>
        ))}
      </ul>
    </section>
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
