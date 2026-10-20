// Makers' releases: submit builds from your own release page, and follow them through review
// (docs/ARCHITECTURE.md §6.3-6.4). Slop never stores builds; it hashes them and signs a manifest.

import { FileChecks, SourceCheckList } from "../components/Checks";
import { useEffect, useState, type FormEvent } from "react";
import { Link, useParams } from "react-router";

import {
  api,
  ApiError,
  type AppPlatform,
  type FileKind,
  type GithubRelease,
  type NewRelease,
  type Platform,
  type Release,
  type UpdateSettings,
} from "../api";
import { PublishProgress } from "../components/PublishProgress";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";

const STATUS: Record<Release["status"], { label: string; badge: string; help: string }> = {
  fetching: { label: "CHECKING FILES", badge: "neutral", help: "Slop is downloading each file once to record its size and hash." },
  awaiting_review: { label: "IN REVIEW", badge: "soon", help: "Waiting for a person at Slop to review it." },
  approved: { label: "APPROVED", badge: "official", help: "Signed by Slop. Players can install it, and get it as an update." },
  rejected: { label: "REJECTED", badge: "bad", help: "Not approved. See the reason below." },
  failed: { label: "FAILED", badge: "bad", help: "Slop couldn't check the files. See the reason below." },
  revoked: { label: "WITHDRAWN", badge: "bad", help: "Slop withdrew this release; it no longer launches." },
};

const PLATFORMS: Array<{ value: Platform; label: string }> = [
  { value: "windows", label: "Windows" },
  { value: "linux", label: "Linux" },
  { value: "linux-deb", label: "Linux .deb" },
  { value: "linux-rpm", label: "Linux .rpm" },
  { value: "mac", label: "macOS" },
  { value: "android", label: "Android" },
];

/** What each platform's build is (the server checks the same, releases.rs). */
const BUILD: Record<AppPlatform, { kind: FileKind; extensions: string[]; describe: string }> = {
  windows: { kind: "archive", extensions: [".zip"], describe: "a .zip of the app's files" },
  linux: { kind: "archive", extensions: [".tar.gz", ".tgz"], describe: "a .tar.gz of the app's files" },
  "linux-deb": { kind: "installer", extensions: [".deb"], describe: "a .deb package" },
  "linux-rpm": { kind: "installer", extensions: [".rpm"], describe: "an .rpm package" },
};

export function fitsPlatform(platform: AppPlatform, fileName: string): boolean {
  const name = fileName.toLowerCase();
  return BUILD[platform].extensions.some((e) => name.endsWith(e));
}

interface FileRow {
  platform: AppPlatform;
  url: string;
  exec: string;
  args: string;
}

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
  const [settingsNonce, setSettingsNonce] = useState(0);

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

  const isGithub = app.data.repo_url.startsWith("https://github.com/");
  const onSubmitted = () => {
    releases.reload();
    setSettingsNonce((n) => n + 1);
  };

  return (
    <div className="page narrow">
      <div className="crumbs">
        <Link to="/publish">Your apps</Link> › <Link to={`/app/${slug}`}>{app.data.title}</Link> › Releases
      </div>
      <h1 className="page-title">Releases</h1>
      <p className="muted">
        A release is one version of your app: a GitHub release with your builds attached. Slop downloads each file once to
        record its size and SHA-256, checks it, and signs it. Players' apps then download from your release page and refuse any
        file that doesn't match.
      </p>

      {releases.data && server.data && (
        <PublishProgress app={app.data} releases={releases.data} server={server.data} />
      )}

      {app.data.platforms.length === 0 ? (
        <section className="panel panel-pad">
          <h2>Submit a release</h2>
          <p className="notice">
            First choose what your app runs on (Windows, Linux or both) on its store page.{" "}
            <Link to={`/publish/${slug}`}>Edit store page</Link>
          </p>
        </section>
      ) : isGithub ? (
        <ReleasePicker slug={slug} platforms={app.data.platforms} onSubmitted={onSubmitted} />
      ) : (
        <ReleaseForm repoUrl={app.data.repo_url} slug={slug} platforms={app.data.platforms} onSubmitted={onSubmitted} />
      )}

      <UpdatesPanel key={settingsNonce} slug={slug} />

      <SlopBuildSetup slug={slug} />
      <SigningHelp />

      {isGithub && app.data.platforms.length > 0 && (
        <details className="panel panel-pad help">
          <summary>
            <b>Enter a release by hand</b> <span className="muted">— if picking doesn't fit</span>
          </summary>
          <ReleaseForm repoUrl={app.data.repo_url} slug={slug} platforms={app.data.platforms} onSubmitted={onSubmitted} />
        </details>
      )}

      <h2 className="section-title">History</h2>
      {releases.error && <ErrorState error={releases.error} onRetry={releases.reload} />}
      {releases.data?.length === 0 && <div className="empty-state">No releases yet.</div>}
      <ul className="release-list">
        {releases.data?.map((r) => (
          <li key={r.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <b>{r.version}</b>
                <span className={`badge ${STATUS[r.status].badge}`}>
                  {r.auto_approved && r.status === "approved" ? "APPROVED AUTOMATICALLY" : STATUS[r.status].label}
                </span>
                {r.source !== "maker" && (
                  <span className="muted small">{r.source === "slop_build" ? "from Slop Build" : "picked up from GitHub"}</span>
                )}
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
            <SourceCheckList checks={r.source_checks} audience="maker" />
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
        <FileChecks file={file} audience="maker" />
        {file.provenance ? (
          <div className="small">
            <span className="badge official">{file.provenance.built_by_slop_build ? "BUILT BY SLOP BUILD" : "SIGNED BY YOUR CI"}</span>{" "}
            <span className="muted">
              {workflowName(file.provenance.signer_workflow)}
              {file.provenance.runner === "self-hosted" && " · self-hosted runner"}
            </span>
          </div>
        ) : file.maker_key ? (
          <div className="small">
            <span className="badge official">SIGNED BY YOUR KEY</span>{" "}
            <span className="muted">
              {file.maker_key.key_name} · <code>{file.maker_key.fingerprint}</code>
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

const PROJECT_LABEL: Record<string, string> = {
  rust: "Rust",
  go: "Go",
  godot: "Godot",
  "godot-dotnet": "Godot (C#)",
  dotnet: ".NET",
  unity: "Unity",
  web: "web (HTML5)",
  electron: "Electron",
  tauri: "Tauri",
  python: "Python",
  custom: "custom",
};

/** Slop Build: one file in the maker's repo, and releases arrive by themselves (ARCHITECTURE §6.2). */
function SlopBuildSetup({ slug }: { slug: string }) {
  const setup = useLoad(() => api.slopBuildSetup(slug), [slug]);
  if (setup.error?.status === 503) return null; // not set up on this server
  return (
    <section className="panel panel-pad help slop-build">
      <h2>Let Slop Build do it</h2>
      <p>
        Add one file to your repository and every push to your main branch, or every <code>v*</code> tag, builds your app
        (Rust, Go, .NET, Godot, Unity, Python, Electron, Tauri, a website, or your own build command) for Windows and Linux on GitHub
        Actions (a .zip, a .tar.gz and .deb/.rpm packages), signs it, attaches it to a
        release, and sends it here. No
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
              Detected a <b>{PROJECT_LABEL[setup.data.project_type]}</b> project.
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
          {setup.data.build_now_url && (
            <p>
              <ExternalLink href={setup.data.build_now_url}>
                <span className="btn">Build now</span>
              </ExternalLink>{" "}
              <span className="muted small">
                Once the file is in your repository: opens it on GitHub, where Run workflow builds your main branch straight
                away.
              </span>
            </p>
          )}
          <details>
            <summary>
              <b>Building somewhere else?</b> <span className="muted">Add Slop Check so Windows builds get Microsoft Defender's check</span>
            </summary>
            <p className="muted small">
              Windows builds need a clean Microsoft Defender check before they're published (Slop Build does it for you). Slop Check runs it
              on GitHub's Windows runner whenever you publish a release. <Link to="/publish/guide#slop-check">More about it</Link>.
            </p>
            <p className="muted small">
              <code>{setup.data.check_file_path}</code>
            </p>
            <pre>
              <code>{setup.data.check_workflow}</code>
            </pre>
            {setup.data.check_create_url && (
              <p>
                <ExternalLink href={setup.data.check_create_url}>
                  <span className="btn">Add Slop Check to my repository</span>
                </ExternalLink>
              </p>
            )}
          </details>
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

function ReleaseForm({
  repoUrl,
  slug,
  platforms,
  onSubmitted,
}: {
  repoUrl: string;
  slug: string;
  platforms: AppPlatform[];
  onSubmitted: () => void;
}) {
  const blank = () => platforms.map((platform): FileRow => ({ platform, url: "", exec: "", args: "" }));
  const [version, setVersion] = useState("");
  const [commit, setCommit] = useState("");
  const [rows, setRows] = useState<FileRow[]>(blank);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  const update = (i: number, change: Partial<FileRow>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...change } : r)));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    const release: NewRelease = {
      version: version.trim(),
      commit_sha: commit.trim().toLowerCase(),
      files: rows.map((r) => ({
        platform: r.platform,
        kind: BUILD[r.platform].kind,
        url: r.url.trim(),
        ...(BUILD[r.platform].kind === "archive" ? { exec: r.exec.trim() } : {}),
        args: r.args.trim() ? r.args.trim().split(/\s+/) : [],
      })),
    };
    try {
      await api.submitRelease(slug, release);
      setDone(true);
      setVersion("");
      setCommit("");
      setRows(blank());
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
        <fieldset key={row.platform} className="file-row">
          <legend>
            {PLATFORM_LABEL[row.platform]} <span className="muted small">— {BUILD[row.platform].describe}</span>
          </legend>
          <label>
            File URL
            <input
              type="url"
              value={row.url}
              required
              placeholder={
                isGithub
                  ? `${repoUrl}/releases/download/v1.0.0/app${BUILD[row.platform].extensions[0]}`
                  : "https://…"
              }
              onChange={(e) => update(i, { url: e.target.value })}
            />
            <span className="hint">
              {isGithub ? "A file on this repository's GitHub releases page." : "On the same site as your repository."}
            </span>
          </label>
          {BUILD[row.platform].kind === "archive" && (
            <div className="row-2">
              <label>
                Program to start
                <input
                  value={row.exec}
                  required
                  placeholder={row.platform === "windows" ? "my-game.exe" : "bin/my-game"}
                  onChange={(e) => update(i, { exec: e.target.value })}
                />
                <span className="hint">Path inside the archive.</span>
              </label>
              <label>
                Arguments (optional)
                <input value={row.args} placeholder="--fullscreen" onChange={(e) => update(i, { args: e.target.value })} />
              </label>
            </div>
          )}
        </fieldset>
      ))}
      {error && (error.field === "files" || !error.field) && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="actions">
        <button className="btn slime" disabled={busy}>
          {busy ? "Submitting…" : "Submit"}
        </button>
      </div>
    </form>
  );
}

const PLATFORM_LABEL = Object.fromEntries(PLATFORMS.map((p) => [p.value, p.label])) as Record<Platform, string>;

interface PickRow {
  include: boolean;
  name: string;
  url: string;
  platform: AppPlatform;
  exec: string;
}

/** Starts from Slop's guesses: the first file that fits each of the app's platforms is ticked. */
export function pickRows(release: GithubRelease, platforms: AppPlatform[]): PickRow[] {
  const taken = new Set<AppPlatform>();
  return release.assets.map((a) => {
    const guess = platforms.find((p) => p === a.platform) ?? platforms.find((p) => fitsPlatform(p, a.name));
    const include = guess !== undefined && !taken.has(guess);
    if (guess) taken.add(guess);
    return { include, name: a.name, url: a.url, platform: guess ?? platforms[0], exec: "" };
  });
}

/** "Pick a release": the repository's GitHub releases, so makers don't type versions, commits or links. */
function ReleasePicker({
  slug,
  platforms,
  onSubmitted,
}: {
  slug: string;
  platforms: AppPlatform[];
  onSubmitted: () => void;
}) {
  const list = useLoad(() => api.githubReleases(slug), [slug]);
  const [tag, setTag] = useState("");
  const [rows, setRows] = useState<PickRow[]>([]);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string>();

  const releases = list.data ?? [];
  const chosen = releases.find((r) => r.tag === tag);
  const choose = (t: string) => {
    setTag(t);
    setError(undefined);
    setDone(undefined);
    const r = releases.find((x) => x.tag === t);
    setRows(r ? pickRows(r, platforms) : []);
  };
  // Start on the newest release that hasn't been sent yet.
  useEffect(() => {
    if (tag || !list.data) return;
    const first = list.data.find((r) => !r.submitted && !r.prerelease) ?? list.data.find((r) => !r.submitted);
    if (first) choose(first.tag);
  }, [list.data]);

  const update = (i: number, change: Partial<PickRow>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...change } : r)));
  const included = rows.filter((r) => r.include);
  const chosenPlatforms = included.map((r) => r.platform);
  const duplicate = chosenPlatforms.find((p, i) => chosenPlatforms.indexOf(p) !== i);
  const missing = platforms.filter((p) => !chosenPlatforms.includes(p));
  const wrongType = included.find((r) => !fitsPlatform(r.platform, r.name));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!chosen) return;
    setBusy(true);
    setError(undefined);
    try {
      await api.submitGithubRelease(
        slug,
        chosen.tag,
        included.map((r) => ({
          platform: r.platform,
          kind: BUILD[r.platform].kind,
          url: r.url,
          ...(BUILD[r.platform].kind === "archive" ? { exec: r.exec.trim() } : {}),
        })),
      );
      setDone(chosen.version);
      list.reload();
      setTag("");
      setRows([]);
      onSubmitted();
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
      <h2>Submit a release</h2>
      <p className="muted small">
        Make a release on GitHub with your builds attached, then pick it here. Slop reads the version from its tag and the
        commit from GitHub. Each release needs: {platforms.map((p) => `${PLATFORM_LABEL[p]} (${BUILD[p].describe})`).join(", ")}.
      </p>
      {done && (
        <p className="notice" role="status">
          Version {done} sent. Slop is checking the files; follow it in the progress above.
        </p>
      )}
      {list.loading && !list.data && <p className="muted">Reading your GitHub releases…</p>}
      {list.error && (
        <p className="form-error" role="alert">
          {list.error.message}
        </p>
      )}
      {list.data && releases.length === 0 && (
        <p className="notice">
          Your repository has no releases yet. On GitHub, open <b>Releases → Draft a new release</b>, give it a tag like{" "}
          <code>v1.0.0</code>, attach your builds and publish it. Then come back and{" "}
          <button type="button" className="link-button" onClick={list.reload}>
            look again
          </button>
          .
        </p>
      )}
      {releases.length > 0 && (
        <label>
          GitHub release
          <select value={tag} onChange={(e) => choose(e.target.value)}>
            <option value="">Choose a release…</option>
            {releases.map((r) => (
              <option key={r.tag} value={r.tag} disabled={r.submitted}>
                {r.tag}
                {r.name && r.name !== r.tag ? ` — ${r.name}` : ""}
                {r.prerelease ? " (pre-release)" : ""}
                {r.submitted ? " (already sent)" : ""}
              </option>
            ))}
          </select>
        </label>
      )}
      {chosen && (
        <>
          <p className="muted small">
            Version <b>{chosen.version}</b> ·{" "}
            <ExternalLink href={chosen.html_url}>see it on GitHub</ExternalLink>
          </p>
          {rows.length === 0 && <p className="notice">This release has no files attached. Add your builds to it on GitHub.</p>}
          {rows.length > 0 && (
            <p className="small">
              <b>Which files are builds?</b> Tick one for each platform.
            </p>
          )}
          {rows.map((row, i) => (
            <fieldset key={row.name} className={`file-row pick-row ${row.include ? "" : "off"}`}>
              <label className="check">
                <input type="checkbox" checked={row.include} onChange={(e) => update(i, { include: e.target.checked })} />
                <code>{row.name}</code>
              </label>
              {row.include && (
                <label>
                  Build for
                  <select value={row.platform} onChange={(e) => update(i, { platform: e.target.value as AppPlatform })}>
                    {platforms.map((p) => (
                      <option key={p} value={p}>
                        {PLATFORM_LABEL[p]}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {row.include && !fitsPlatform(row.platform, row.name) && (
                <p className="form-error">
                  The {PLATFORM_LABEL[row.platform]} build must be {BUILD[row.platform].describe}.
                </p>
              )}
              {row.include && BUILD[row.platform].kind === "archive" && (
                <label>
                  Program to start
                  <input value={row.exec} required placeholder="my-game.exe or bin/my-game" onChange={(e) => update(i, { exec: e.target.value })} />
                  <span className="hint">Its path inside the zip. Slop remembers it for your next releases.</span>
                </label>
              )}
            </fieldset>
          ))}
          {duplicate && <p className="form-error">Pick one file for {PLATFORM_LABEL[duplicate]}.</p>}
          {rows.length > 0 && missing.length > 0 && (
            <p className="notice">
              Still needed: {missing.map((p) => `${PLATFORM_LABEL[p]} (${BUILD[p].describe})`).join(", ")}. If the release
              doesn't have it, attach it on GitHub and pick the release again.
            </p>
          )}
          {error && (
            <p className="form-error" role="alert">
              {error.message}
            </p>
          )}
          <div className="actions">
            <button
              className="btn slime"
              disabled={busy || included.length === 0 || Boolean(duplicate) || missing.length > 0 || Boolean(wrongType)}
            >
              {busy ? "Sending…" : `Send version ${chosen.version}`}
            </button>
          </div>
        </>
      )}
    </form>
  );
}

/** Automatic pickup of new GitHub releases, and automatic approval of updates. */
export function UpdatesPanel({ slug }: { slug: string }) {
  const loaded = useLoad(() => api.updateSettings(slug), [slug]);
  const [settings, setSettings] = useState<UpdateSettings>();
  const [error, setError] = useState<string>();
  const s = settings ?? loaded.data;
  if (!s) return null;

  const change = async (c: Partial<Pick<UpdateSettings, "watch_releases" | "auto_update">>) => {
    setError(undefined);
    try {
      setSettings(await api.changeUpdateSettings(slug, c));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };

  return (
    <section className="panel panel-pad form updates-panel">
      <h2>Updates</h2>
      <label className="check">
        <input type="checkbox" checked={s.watch_releases} onChange={(e) => void change({ watch_releases: e.target.checked })} />
        <span>
          <b>Pick up new GitHub releases</b>
          <span className="muted small" style={{ display: "block" }}>
            When you publish a new release on GitHub (not a draft or pre-release), Slop sends it for you, usually within
            10 minutes (longer for apps that haven't updated in a while). It expects the same files as last time, with the new version in their names.
          </span>
        </span>
      </label>
      {s.watch_releases && (
        <p className="muted small updates-status">
          {s.watch_note ??
            (s.watch_checked_at
              ? `Last checked ${formatDate(s.watch_checked_at)}${s.watch_last_tag ? ` · newest seen: ${s.watch_last_tag}` : ""}.`
              : "Not checked yet.")}
        </p>
      )}
      {s.watch_problem && <p className="notice">{s.watch_problem}</p>}
      {s.watch_releases && s.github_app && (
        <div className="muted small updates-status">
          {s.github_app.installed ? (
            <>
              <span className="badge official">GITHUB APP INSTALLED</span> GitHub tells Slop the moment you publish a
              release, so it's picked up within a minute.
            </>
          ) : (
            <>
              <b>Faster pickup:</b> install Slop's GitHub App on this repository and GitHub tells Slop the moment you
              publish a release, instead of Slop checking every so often. It only reads your releases and code.{" "}
              <ExternalLink href={s.github_app.install_url}>Install on GitHub</ExternalLink>
            </>
          )}
        </div>
      )}
      <label className="check">
        <input type="checkbox" checked={s.auto_update} onChange={(e) => void change({ auto_update: e.target.checked })} />
        <span>
          <b>Approve updates automatically</b>
          <span className="muted small" style={{ display: "block" }}>
            After a person at Slop approves your first release, new versions that pass every check go out to players
            straight away. Players get a notice and update when they want.
          </span>
        </span>
      </label>
      {s.auto_update && (
        <p className="muted small updates-status">
          {s.auto_approval_ready
            ? "Your next release will be approved automatically if it passes every check."
            : `Your next release waits for a person: ${s.auto_approval_note}.`}
        </p>
      )}
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}
