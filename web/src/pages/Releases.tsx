// Makers' releases: submit builds from your own release page, and follow them through review
// (docs/ARCHITECTURE.md §6.3-6.4). Slop never stores builds; it hashes them and signs a manifest.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useParams } from "react-router";

import { api, ApiError, type FileKind, type NewRelease, type Platform, type Release } from "../api";
import { PublishProgress } from "../components/PublishProgress";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";

const STATUS: Record<Release["status"], { label: string; badge: string; help: string }> = {
  fetching: { label: "CHECKING FILES", badge: "neutral", help: "Slop is downloading each file once to record its size and hash." },
  awaiting_review: { label: "IN REVIEW", badge: "soon", help: "A person at Slop is reviewing this release." },
  approved: { label: "APPROVED", badge: "official", help: "Signed by Slop. Players can install it." },
  rejected: { label: "REJECTED", badge: "bad", help: "Not approved. See the reason below." },
  failed: { label: "FAILED", badge: "bad", help: "Slop couldn't check the files. See the reason below." },
  revoked: { label: "WITHDRAWN", badge: "bad", help: "Slop withdrew this release; it no longer launches." },
};

const PLATFORMS: Array<{ value: Platform; label: string }> = [
  { value: "windows", label: "Windows" },
  { value: "mac", label: "macOS" },
  { value: "linux", label: "Linux" },
  { value: "android", label: "Android" },
];

interface FileRow {
  platform: Platform;
  kind: FileKind;
  url: string;
  exec: string;
  args: string;
}

const newRow = (platform: Platform = "windows"): FileRow => ({
  platform,
  kind: platform === "android" ? "apk" : "archive",
  url: "",
  exec: "",
  args: "",
});

export function Releases() {
  const { slug = "" } = useParams();
  const { me } = useSession();
  const app = useLoad(() => (me ? api.app(slug) : Promise.resolve(undefined)), [slug, me?.id]);
  const releases = useLoad(() => (me ? api.releases(slug) : Promise.resolve([])), [slug, me?.id]);
  const server = useLoad(() => api.server(), []);
  // While files are being checked, refresh every few seconds so progress moves on its own.
  const checking = releases.data?.some((r) => r.status === "fetching");
  useEffect(() => {
    if (!checking) return;
    const timer = setInterval(() => releases.reload(), 4000);
    return () => clearInterval(timer);
  }, [checking, releases.reload]);
  useDocumentTitle(app.data ? `${app.data.title} releases` : "Releases");

  if (me === undefined || app.loading) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <p>
          <Link to={`/signin?return_to=/publish/${slug}/releases`}>Sign in</Link> to manage releases.
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
  if (!app.data) return <Loading />;

  return (
    <div className="page narrow">
      <div className="crumbs">
        <Link to="/publish">Your apps</Link> › <Link to={`/app/${slug}`}>{app.data.title}</Link> › Releases
      </div>
      <h1 className="page-title">Releases</h1>
      <p className="muted">
        Builds stay on your own release page. Slop downloads each file once to record its size and SHA-256, a person reviews
        the release, and Slop signs it. Players' apps then download from your release page and refuse any file that doesn't
        match.
      </p>

      {releases.data && server.data && (
        <PublishProgress app={app.data} releases={releases.data} server={server.data} />
      )}

      <SlopBuildSetup slug={slug} />
      <SigningHelp />

      <ReleaseForm repoUrl={app.data.repo_url} slug={slug} onSubmitted={releases.reload} />

      <h2 className="section-title">History</h2>
      {releases.error && <ErrorState error={releases.error} onRetry={releases.reload} />}
      {releases.data?.length === 0 && <div className="empty-state">No releases yet.</div>}
      <ul className="release-list">
        {releases.data?.map((r) => (
          <li key={r.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <b>{r.version}</b>
                <span className={`badge ${STATUS[r.status].badge}`}>{STATUS[r.status].label}</span>
              </div>
              <span className="muted small">{formatDate(r.created_at)}</span>
            </div>
            <p className="muted small">{STATUS[r.status].help}</p>
            {r.status_reason && <p className="notice">{r.status_reason}</p>}
            <dl className="kv">
              <dt>Commit</dt>
              <dd>
                <code>{r.commit_sha.slice(0, 12)}</code>{" "}
                {r.commit_verified ? <span className="muted">found in your repo</span> : <span className="muted">checked by the reviewer</span>}
              </dd>
              {r.files.map((f) => (
                <FileDetail key={f.platform} file={f} />
              ))}
            </dl>
          </li>
        ))}
      </ul>
    </div>
  );
}

const SCAN_BADGE: Record<NonNullable<Release["files"][number]["scan_result"]>, { label: string; badge: string }> = {
  passed: { label: "SCAN PASSED", badge: "official" },
  note: { label: "SCAN PASSED WITH NOTES", badge: "soon" },
  on_hold: { label: "SCAN: ON HOLD", badge: "soon" },
  rejected: { label: "SCAN: REJECTED", badge: "bad" },
  blocked: { label: "SCAN: BLOCKED", badge: "bad" },
  skipped: { label: "NOT SCANNED", badge: "neutral" },
};

/** `owner/repo/.github/workflows/build.yml` from a workflow URI. */
export function workflowName(uri: string): string {
  return uri.replace(/^https:\/\/github\.com\//, "").replace(/@.*$/, "");
}

function FileDetail({ file }: { file: Release["files"][number] }) {
  const label = PLATFORMS.find((p) => p.value === file.platform)?.label ?? file.platform;
  return (
    <>
      <dt>{label}</dt>
      <dd>
        {file.file_name}
        {file.exec && (
          <>
            {" "}
            · starts <code>{file.exec}</code>
          </>
        )}
        {file.sha256 && (
          <div className="faint small">
            SHA-256 <code>{file.sha256.slice(0, 16)}…</code> · {file.size_bytes?.toLocaleString()} bytes
          </div>
        )}
        {file.scan_result && (
          <div className="small">
            <span className={`badge ${SCAN_BADGE[file.scan_result].badge}`}>{SCAN_BADGE[file.scan_result].label}</span>{" "}
            {file.scan_detail && <span className="muted">{file.scan_detail}</span>}
          </div>
        )}
        {file.provenance ? (
          <div className="small">
            <span className="badge official">{file.provenance.built_by_slop_build ? "BUILT BY SLOP BUILD" : "SIGNED BY YOUR CI"}</span>{" "}
            <span className="muted">
              {workflowName(file.provenance.signer_workflow)}
              {file.provenance.runner === "self-hosted" && " · self-hosted runner"}
            </span>
          </div>
        ) : (
          file.provenance_problem && (
            <div className="small">
              <span className="badge bad">NOT SIGNED</span> <span className="muted">{file.provenance_problem}</span>
            </div>
          )
        )}
      </dd>
    </>
  );
}

const WORKFLOW_SNIPPET = `permissions:
  contents: write        # upload to your release
  id-token: write        # get a signing certificate from Sigstore
  attestations: write    # store the attestation on GitHub

steps:
  # … build your app into dist/ …
  - uses: actions/attest-build-provenance@v2
    with:
      subject-path: dist/*`;

/** Slop Build: one file in the maker's repo, and releases arrive by themselves (ARCHITECTURE §6.2). */
function SlopBuildSetup({ slug }: { slug: string }) {
  const setup = useLoad(() => api.slopBuildSetup(slug), [slug]);
  if (setup.error?.status === 503) return null; // not set up on this server
  return (
    <section className="panel panel-pad help slop-build">
      <h2>Let Slop Build do it</h2>
      <p>
        Add one file to your repository and every push to your main branch, or every <code>v*</code> tag, builds your app
        for Windows, macOS and Linux on GitHub Actions, signs it, attaches it to a release, and sends it here for review. No
        keys or secrets to set up.
      </p>
      {setup.loading && <p className="muted">Looking at your repository…</p>}
      {setup.error && (
        <p className="form-error" role="alert">
          {setup.error.message}
        </p>
      )}
      {setup.data && (
        <>
          {setup.data.note && <p className="notice">{setup.data.note}</p>}
          {!setup.data.note && setup.data.supported && (
            <p className="muted small">
              Detected a <b>{setup.data.project_type === "rust" ? "Rust" : setup.data.project_type === "go" ? "Go" : "custom"}</b>{" "}
              project.
            </p>
          )}
          <p className="muted small">
            <code>{setup.data.file_path}</code>
          </p>
          <pre>
            <code>{setup.data.workflow}</code>
          </pre>
          {setup.data.create_url && (
            <p>
              <ExternalLink href={setup.data.create_url}>
                <span className="btn slime">Add to my repository on GitHub</span>
              </ExternalLink>{" "}
              <span className="muted small">Opens GitHub with the file filled in; you just commit it.</span>
            </p>
          )}
        </>
      )}
    </section>
  );
}

/** Every build needs its maker's signature (docs/SECURITY.md §5.3). */
function SigningHelp() {
  return (
    <details className="panel panel-pad help">
      <summary>
        <b>Or build and sign it yourself</b>{" "}
        <span className="muted">— every build needs your signature before it can be approved</span>
      </summary>
      <p>
        Build your release files in GitHub Actions and add GitHub's attestation step. It signs each file with a short-lived
        certificate tied to your repository, workflow and commit, and records it in Sigstore's public log. There's no key for
        you to manage, and nobody who only has your Slop password can fake it.
      </p>
      <pre>
        <code>{WORKFLOW_SNIPPET}</code>
      </pre>
      <p className="muted small">
        Slop finds the attestation by each file's hash. The commit you enter below must be the one the workflow built. Slop
        Build will set this up for you automatically. Only GitHub repositories are supported so far.
      </p>
    </details>
  );
}

function ReleaseForm({ repoUrl, slug, onSubmitted }: { repoUrl: string; slug: string; onSubmitted: () => void }) {
  const [version, setVersion] = useState("");
  const [commit, setCommit] = useState("");
  const [rows, setRows] = useState<FileRow[]>([newRow()]);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  const update = (i: number, change: Partial<FileRow>) =>
    setRows((rs) =>
      rs.map((r, j) => {
        if (j !== i) return r;
        const next = { ...r, ...change };
        if (change.platform === "android") next.kind = "apk";
        else if (change.platform && next.kind === "apk") next.kind = "archive";
        return next;
      }),
    );

  const used = new Set(rows.map((r) => r.platform));
  const nextPlatform = PLATFORMS.find((p) => !used.has(p.value))?.value;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    const release: NewRelease = {
      version: version.trim(),
      commit_sha: commit.trim().toLowerCase(),
      files: rows.map((r) => ({
        platform: r.platform,
        kind: r.kind,
        url: r.url.trim(),
        ...(r.kind === "archive" ? { exec: r.exec.trim() } : {}),
        args: r.args.trim() ? r.args.trim().split(/\s+/) : [],
      })),
    };
    try {
      await api.submitRelease(slug, release);
      setDone(true);
      setVersion("");
      setCommit("");
      setRows([newRow()]);
      onSubmitted();
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };

  const isGithub = repoUrl.startsWith("https://github.com/");

  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
      <h2>New release</h2>
      {done && (
        <p className="notice" role="status">
          Submitted. Slop is checking the files; you'll see the review status below.
        </p>
      )}
      <div className="row-2">
        <label>
          Version
          <input value={version} placeholder="1.0.0" maxLength={40} required onChange={(e) => setVersion(e.target.value)} />
          {error?.field === "version" && <span className="field-error">{error.message}</span>}
        </label>
        <label>
          Commit
          <input
            value={commit}
            placeholder="Full 40-character SHA"
            maxLength={40}
            minLength={40}
            required
            onChange={(e) => setCommit(e.target.value)}
          />
          <span className="hint">The commit these builds came from. Tags can move; commits can't.</span>
          {error?.field === "commit_sha" && <span className="field-error">{error.message}</span>}
        </label>
      </div>

      {rows.map((row, i) => (
        <fieldset key={i} className="file-row">
          <legend>Build {i + 1}</legend>
          <div className="row-2">
            <label>
              Platform
              <select value={row.platform} onChange={(e) => update(i, { platform: e.target.value as Platform })}>
                {PLATFORMS.map((p) => (
                  <option key={p.value} value={p.value} disabled={p.value !== row.platform && used.has(p.value)}>
                    {p.label}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Type
              <select
                value={row.kind}
                disabled={row.platform === "android"}
                onChange={(e) => update(i, { kind: e.target.value as FileKind })}
              >
                {row.platform === "android" ? (
                  <option value="apk">APK</option>
                ) : (
                  <>
                    <option value="archive">Zip of the app's files (preferred)</option>
                    <option value="installer">Installer</option>
                  </>
                )}
              </select>
            </label>
          </div>
          <label>
            File URL
            <input
              type="url"
              value={row.url}
              required
              placeholder={isGithub ? `${repoUrl}/releases/download/v1.0.0/app-${row.platform}.zip` : "https://…"}
              onChange={(e) => update(i, { url: e.target.value })}
            />
            <span className="hint">
              {isGithub ? "A file on this repository's GitHub releases page." : "On the same site as your repository."}
            </span>
          </label>
          {row.kind === "archive" && (
            <div className="row-2">
              <label>
                Program to start
                <input value={row.exec} required placeholder="bin/my-game" onChange={(e) => update(i, { exec: e.target.value })} />
                <span className="hint">Path inside the zip.</span>
              </label>
              <label>
                Arguments (optional)
                <input value={row.args} placeholder="--fullscreen" onChange={(e) => update(i, { args: e.target.value })} />
              </label>
            </div>
          )}
          {rows.length > 1 && (
            <button type="button" className="btn small ghost" onClick={() => setRows(rows.filter((_, j) => j !== i))}>
              Remove this build
            </button>
          )}
        </fieldset>
      ))}
      {error?.field === "files" && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {error && !error.field && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="actions spread">
        {nextPlatform ? (
          <button type="button" className="btn ghost" onClick={() => setRows([...rows, newRow(nextPlatform)])}>
            + Add another platform
          </button>
        ) : (
          <span />
        )}
        <button className="btn slime" disabled={busy}>
          {busy ? "Submitting…" : "Submit for review"}
        </button>
      </div>
    </form>
  );
}
