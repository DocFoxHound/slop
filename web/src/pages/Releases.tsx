// SPDX-License-Identifier: AGPL-3.0-or-later
// Makers' releases: submit builds from your own release page, follow them through SLAPorSLOP's
// checks, and publish them (docs/ARCHITECTURE.md §6.3-6.4). SLAPorSLOP never stores builds; it
// hashes them, and signs a manifest when its maker presses Publish (crates/slop-api/src/publish.rs).
// A release the checks hold shows what holds it and the conversation with staff (ReleaseHold.tsx).
// A browser game's release is a play address instead (§6.8): SLAPorSLOP checks the address, and
// Play opens it once its maker publishes it. Nothing is built, signed or scanned.
// What the checks found to fix is on the To fix tab (Fixes.tsx), and Slop Build, Slop Check and
// signing on the Builds tab (Builds.tsx).

import {
  AddressCheckList,
  FileChecks,
  RepoScanList,
  SourceCheckList,
} from "../components/Checks";
import { useEffect, useId, useState, type FormEvent } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router";

import {
  api,
  ApiError,
  type AppPlatform,
  type BuildPlatform,
  type FileKind,
  type GithubRelease,
  type NewRelease,
  type Platform,
  publishBlocked,
  type Release,
  type ReleaseRating,
  type UpdateSettings,
} from "../api";
import { MakerFrame } from "../components/MakerFrame";
import { PublishProgress, refreshEvery } from "../components/PublishProgress";
import { AskAboutFailed, PastHold, ReleaseHold } from "../components/ReleaseHold";
import { useInviteGate } from "../components/Invite";
import { useTermsGate } from "../components/Terms";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { CONTENT_LABEL, formatDate, isDedicatedServer, playsInBrowser } from "../format";
import { useConfirmStep, useDocumentTitle, useInterval, useLoad } from "../hooks";
import { renderMarkdown } from "../markdown";
import { useSession } from "../session";
import { AgeSelect, ContentLabelPicker } from "./Publish";

const STATUS: Record<
  Release["status"],
  { label: string; badge: string; help: string }
> = {
  fetching: {
    label: "CHECKING FILES",
    badge: "neutral",
    help: "SLAPorSLOP is downloading, hashing and scanning each file, one at a time: Publishing progress above shows which.",
  },
  awaiting_review: {
    label: "ON HOLD",
    badge: "warn",
    help: "SLAPorSLOP's checks held it. Below: what was found, and what you can do.",
  },
  ready: {
    label: "PASSED, NOT PUBLISHED",
    badge: "soon",
    help: "It passed every check. Players get it once you publish it.",
  },
  approved: {
    label: "PUBLISHED",
    badge: "official",
    help: "Signed by SLAPorSLOP when you published it. Players can install it, and get it as an update.",
  },
  rejected: {
    label: "REJECTED",
    badge: "bad",
    help: "Turned down by SLAPorSLOP staff. See the reason below.",
  },
  failed: {
    label: "FAILED",
    badge: "bad",
    help: "It didn't pass SLAPorSLOP's checks. See the reason below.",
  },
  revoked: {
    label: "WITHDRAWN",
    badge: "bad",
    help: "SLAPorSLOP withdrew this release; it no longer launches.",
  },
  withdrawn: {
    label: "WITHDRAWN BEFORE PUBLISHING",
    badge: "neutral",
    help: "Withdrawn before it was published. It never reached players.",
  },
};

/** A browser game's release is a play address: the same statuses, in its own words (never
 *  "signed", "scanned" or "files"). */
const ADDRESS_STATUS: typeof STATUS = {
  fetching: {
    label: "CHECKING THE ADDRESS",
    badge: "neutral",
    help: "SLAPorSLOP is checking the address: https, a public site, the bad-link lists, and that the page loads.",
  },
  awaiting_review: {
    label: "ON HOLD",
    badge: "warn",
    help: "On hold for SLAPorSLOP staff. Below: why, and what you can do.",
  },
  ready: {
    label: "PASSED, NOT PUBLISHED",
    badge: "soon",
    help: "It passed the checks. Play opens this address once you publish it.",
  },
  approved: {
    label: "PUBLISHED",
    badge: "official",
    help: "Published: Play opens this address.",
  },
  rejected: STATUS.rejected,
  failed: {
    label: "FAILED",
    badge: "bad",
    help: "SLAPorSLOP couldn't check the address. See the reason below.",
  },
  revoked: {
    label: "WITHDRAWN",
    badge: "bad",
    help: "Withdrawn by SLAPorSLOP: Play no longer opens it.",
  },
  withdrawn: STATUS.withdrawn,
};

/** A release's status in words: a download's, or a browser game's play address (which, Coming
 *  soon, Play opens only once the game is in Beta). Exported for tests. */
export function statusText(
  status: Release["status"],
  browser: boolean,
  comingSoon = false,
  blocked = false,
): { label: string; badge: string; help: string } {
  // Something must be fixed before it can be published (its Publish panel lists what).
  if (blocked && status === "ready") {
    return { ...STATUS.ready, help: "It passed SLAPorSLOP's checks, but something must be fixed before you can publish it. Below: what, and what you can do." };
  }
  // Nobody can download a Coming soon page's builds: players get them once it's in Beta.
  if (!browser && comingSoon && status === "ready") {
    return { ...STATUS.ready, help: "It passed every check. Once you publish it, players get it when the page is in Beta." };
  }
  if (!browser && comingSoon && status === "approved") {
    return { ...STATUS.approved, help: "Signed by SLAPorSLOP when you published it. Players can install it once the page is in Beta." };
  }
  if (browser && comingSoon && status === "approved") {
    return { ...ADDRESS_STATUS.approved, help: "Published: Play opens this address once the game is in Beta." };
  }
  if (browser && comingSoon && status === "ready") {
    return { ...ADDRESS_STATUS.ready, help: "It passed the checks. Once you publish it, Play opens this address when the game is in Beta." };
  }
  return (browser ? ADDRESS_STATUS : STATUS)[status];
}

/** "The source check found …" as a sentence. */
const sentenceOf = (text: string) => `${text.charAt(0).toUpperCase()}${text.slice(1).replace(/\.$/, "")}.`;

/** What publishing a release that passed waits for, if anything: in words, or undefined when it
 *  can be published now (the server checks again). Exported for tests. */
export function publishWait(r: Pick<Release, "repo_scan">, picturesPending: boolean): string | undefined {
  if (picturesPending) return "Its new pictures are waiting for review. You can publish once they're reviewed.";
  if (!r.repo_scan || r.repo_scan.status === "failed") {
    return "SLAPorSLOP hasn't read this release's source yet. It tries again every hour, and you can publish once it has a result.";
  }
  return undefined;
}

/**
 * A published (or since withdrawn) release sent after `r`: then `r` can never be published
 * (Publish refuses a release older than one already out, publish.rs), so it gets no Publish panel.
 * Exported for tests.
 */
export function newerOut(r: Pick<Release, "id" | "created_at">, releases: Array<Pick<Release, "id" | "status" | "created_at" | "version">>) {
  return releases.find(
    (x) => x.id !== r.id && (x.status === "approved" || x.status === "revoked") && Date.parse(x.created_at) > Date.parse(r.created_at),
  );
}

/** What to know before publishing, as information (none of it stops it). Exported for tests. */
export function publishNotes(
  r: Pick<Release, "version" | "new_concerns" | "new_play_address" | "play_address">,
  ctx: { heldAt?: string; playsNow?: string | null },
): string[] {
  const notes: string[] = [];
  if (ctx.heldAt) notes.push(`Players are held at ${ctx.heldAt}. Publishing ${r.version} ends the hold.`);
  if (r.new_play_address && r.play_address) {
    notes.push(
      ctx.playsNow
        ? `A new site: Play will open ${r.play_address} instead of ${ctx.playsNow}.`
        : `A new address: Play will open ${r.play_address}.`,
    );
  }
  if (r.new_concerns) notes.push(`${sentenceOf(r.new_concerns)} See the source check on this release.`);
  return notes;
}

/**
 * A version to suggest for the next release: the newest one's last number plus one ("1.0.0" →
 * "1.0.1", "v2" → "v3"), "1.0.0" for the first, and never one already used (versions are unique,
 * whatever became of the release). `taken`: every release's version, newest first. Exported for tests.
 */
export function nextVersion(taken: string[]): string {
  const used = new Set(taken);
  let version = taken[0];
  if (!version) return "1.0.0";
  for (let i = 0; i < 100; i++) {
    const m = /^(.*?)(\d+)$/.exec(version);
    version = m ? `${m[1]}${Number(m[2]) + 1}` : `${version}.1`;
    if (version.length > 40) return "";
    if (!used.has(version)) return version;
  }
  return "";
}

const PLATFORMS: Array<{ value: Platform; label: string }> = [
  { value: "windows", label: "Windows" },
  { value: "linux", label: "Linux" },
  { value: "linux-deb", label: "Linux .deb" },
  { value: "linux-rpm", label: "Linux .rpm" },
  { value: "any", label: "Mod (every platform)" },
  { value: "mac", label: "macOS" },
  { value: "android", label: "Android" },
];

/** What each platform's build is (the server checks the same, releases.rs). */
const BUILD: Record<
  BuildPlatform,
  { kind: FileKind; extensions: string[]; describe: string }
> = {
  windows: {
    kind: "archive",
    extensions: [".zip"],
    describe: "a .zip of the app's files",
  },
  linux: {
    kind: "archive",
    extensions: [".tar.gz", ".tgz"],
    describe: "a .tar.gz of the app's files",
  },
  "linux-deb": {
    kind: "installer",
    extensions: [".deb"],
    describe: "a .deb package",
  },
  "linux-rpm": {
    kind: "installer",
    extensions: [".rpm"],
    describe: "an .rpm package",
  },
  any: {
    kind: "archive",
    extensions: [".zip"],
    describe: "a .zip of the mod's files",
  },
};

/** Archives name a program to start; a mod has none (the game loads it). */
const startsProgram = (p: BuildPlatform) => BUILD[p].kind === "archive" && p !== "any";

/** The store page's platforms that take a file: all but "browser" (a browser game's release is a play address). */
const fileBuilds = (platforms: AppPlatform[]): BuildPlatform[] =>
  platforms.filter((p): p is BuildPlatform => p !== "browser");

export function fitsPlatform(platform: BuildPlatform, fileName: string): boolean {
  const name = fileName.toLowerCase();
  return BUILD[platform].extensions.some((e) => name.endsWith(e));
}

interface FileRow {
  platform: BuildPlatform;
  url: string;
  exec: string;
  args: string;
}

/** The first play address sent with a new store page (Publish.tsx), when it didn't go through. */
export interface FirstAddress {
  address: string;
  message: string;
  field?: string;
}

export function Releases() {
  const { slug = "" } = useParams();
  const { me } = useSession();
  const location = useLocation();
  const navigate = useNavigate();
  const first = (location.state as { firstAddress?: FirstAddress } | null)?.firstAddress;
  const app = useLoad(
    () => (me ? api.app(slug) : Promise.resolve(undefined)),
    [slug, me?.id],
  );
  const releases = useLoad(
    () => (me ? api.releases(slug) : Promise.resolve([])),
    [slug, me?.id],
  );
  const server = useLoad(() => api.server(), []);
  // A release that passed waits for its maker: what publishing it waits for (pictures), and the
  // hold at an older release it would end.
  const anyReady = Boolean(releases.data?.some((r) => r.status === "ready"));
  const media = useLoad(() => (me && anyReady ? api.media(slug) : Promise.resolve(undefined)), [slug, me?.id, anyReady]);
  const overview = useLoad(() => (me && anyReady ? api.manage(slug) : Promise.resolve(undefined)), [slug, me?.id, anyReady]);
  const [published, setPublished] = useState<string>();
  // Older links to what moved to its own tab (2026-11-20): "#fixes" and "#slop-check".
  useEffect(() => {
    if (location.hash === "#fixes") navigate(`/publish/${slug}/fixes`, { replace: true });
    else if (location.hash === "#slop-check") navigate(`/publish/${slug}/builds#slop-check`, { replace: true });
  }, [location.hash, slug, navigate]);
  // "#release-…" (from the progress panel or Manage): scroll to that release once it's shown.
  const loaded = Boolean(releases.data);
  useEffect(() => {
    if (!loaded || !location.hash.startsWith("#release-")) return;
    document.getElementById(location.hash.slice(1))?.scrollIntoView({ block: "start" });
  }, [loaded, location.hash]);
  // New releases start from what the last published one needed.
  const lastRuntimes = releases.data?.find((r) => r.status === "approved")?.runtimes ?? [];
  // While files are being checked (or a release is on hold), look again now and then so progress
  // moves on its own; stop once nothing is under way.
  useInterval(releases.reload, refreshEvery(releases.data));
  useDocumentTitle(app.data ? `${app.data.title} releases` : "Releases");
  const [settingsNonce, setSettingsNonce] = useState(0);

  if (me === undefined || app.loading) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <p>
          <Link to={`/signin?return_to=/publish/${slug}/releases`}>
            Sign in
          </Link>{" "}
          to manage releases.
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
  const browser = playsInBrowser(app.data);
  const onSubmitted = () => {
    releases.reload();
    setSettingsNonce((n) => n + 1);
    // Sent now: a reload doesn't bring back the first address's problem.
    if (first) navigate(location.pathname, { replace: true, state: null });
  };
  const status = (r: Release) => statusText(r.status, browser, app.data?.stage === "coming_soon", publishBlocked(r));
  const changed = () => {
    releases.reload();
    media.reload();
    overview.reload();
  };
  const onPublished = (r: Release) => {
    setPublished(r.version);
    changed();
    // The store page now shows its rating; the stage bar may now move on.
    app.reload();
  };
  const heldAt = overview.data?.held ? overview.data.serving?.version : undefined;
  const comingSoon = app.data.stage === "coming_soon";
  const playsNow = app.data.play_address;

  // The tab's To fix count follows the newest release's checks.
  const newest = releases.data?.[0];
  const fixesKey = newest && `${newest.id}:${newest.status}:${newest.repo_scan?.status ?? ""}`;

  return (
    <MakerFrame app={app.data} tab="releases" fixesKey={fixesKey} onStageMoved={app.reload}>
      <div className="maker-cols">
        <div className="maker-main">
          {browser ? (
            <p className="muted">
              Your game runs on your own website. Here a release is its play address:
              where Play sends players. SLAPorSLOP checks it (https, a public site, the
              bad-link lists, and that the page loads), and checks it again whenever it
              changes. Once it passes, you publish it and Play opens it. SLAPorSLOP doesn't
              host, build or scan your site.
            </p>
          ) : (
            <p className="muted">
              A release is one version of your app: a GitHub release with your builds
              attached. SLAPorSLOP downloads each file once to record its size and SHA-256,
              and checks it. Once it passes, you publish it: SLAPorSLOP signs it, and
              players' apps download from your release page and refuse any file that
              doesn't match.
            </p>
          )}
          <div id="new-release" tabIndex={-1}>

          {app.data.platforms.length === 0 ? (
            <section className="panel panel-pad">
              <h2>Submit a release</h2>
              <p className="notice">
                First choose what your app runs on (Windows, Linux or both, or In the
                browser) on its store page. <Link to={`/publish/${slug}`}>Edit store page</Link>
              </p>
            </section>
          ) : browser ? (
            releases.data ? (
              <PlayAddressForm
                slug={slug}
                current={app.data.play_address}
                // Starts from where Play goes, else the newest address sent.
                start={first?.address ?? app.data.play_address ?? releases.data.find((r) => r.play_address)?.play_address ?? ""}
                versions={releases.data.map((r) => r.version)}
                rating={{ min_age: app.data.min_age, content_labels: app.data.content_labels }}
                isGithub={isGithub}
                first={first}
                onSubmitted={onSubmitted}
              />
            ) : (
              <Loading />
            )
          ) : isGithub ? (
            <ReleasePicker
              slug={slug}
              platforms={fileBuilds(app.data.platforms)}
              rating={{ min_age: app.data.min_age, content_labels: app.data.content_labels }}
              lastRuntimes={lastRuntimes}
              onSubmitted={onSubmitted}
            />
          ) : (
            <ReleaseForm
              repoUrl={app.data.repo_url}
              slug={slug}
              platforms={fileBuilds(app.data.platforms)}
              rating={{ min_age: app.data.min_age, content_labels: app.data.content_labels }}
              lastRuntimes={lastRuntimes}
              onSubmitted={onSubmitted}
            />
          )}
          </div>

          {isGithub && !browser && app.data.platforms.length > 0 && (
            <details className="panel panel-pad help">
              <summary>
                <b>Enter a release by hand</b>{" "}
                <span className="muted">— if picking doesn't fit</span>
              </summary>
              <ReleaseForm
                repoUrl={app.data.repo_url}
                slug={slug}
                platforms={fileBuilds(app.data.platforms)}
                rating={{ min_age: app.data.min_age, content_labels: app.data.content_labels }}
                lastRuntimes={lastRuntimes}
                onSubmitted={onSubmitted}
              />
            </details>
          )}

          {Boolean(releases.data?.length) && (
            <p className="muted small">
              What SLAPorSLOP's checks found to fix, ready to copy for your AI: <Link to={`/publish/${slug}/fixes`}>To fix</Link>.
            </p>
          )}

          <h2 className="section-title">History</h2>
          {published && (
            <p className="notice" role="status">
              Version {published} is published.{" "}
              {app.data.status === "draft"
                ? browser
                  ? "Players can play it once you put the game in the store."
                  : "Players can get it once you put the app in the store."
                : browser
                  ? "Play opens its address."
                  : "Players get it as an update."}
            </p>
          )}
          {releases.error && (
            <ErrorState error={releases.error} onRetry={releases.reload} />
          )}
          {releases.data?.length === 0 && (
            <div className="empty-state">No releases yet.</div>
          )}
          <ul className="release-list">
            {releases.data?.map((r) => (
              <li key={r.id} id={`release-${r.id}`} className="panel panel-pad">
                <div className="title-row spread">
                  <div className="title-row">
                    <b>{r.version}</b>
                    {r.rating && (
                      <span className="muted small">
                        {r.rating.min_age === 0 ? "Everyone" : `${r.rating.min_age}+`}
                        {r.rating.content_labels.length > 0 &&
                          ` · ${r.rating.content_labels.map((l) => CONTENT_LABEL[l]).join(", ")}`}
                      </span>
                    )}
                    <span className={`badge ${status(r).badge}`}>{status(r).label}</span>
                    {r.source !== "maker" && (
                      <span className="muted small">
                        {r.source === "slop_build"
                          ? "from Slop Build"
                          : "picked up from GitHub"}
                      </span>
                    )}
                  </div>
                  <span className="muted small">{formatDate(r.created_at)}</span>
                </div>
                <p className="muted small">
                  {/* An approved address that isn't the one Play opens any more says so (the server knows). */}
                  {browser && r.status === "approved" && r.plays_now === false && r.progress
                    ? r.progress.summary
                    : status(r).help}
                </p>
                {/* On hold, what holds it says the reason (ReleaseHold): not twice. */}
                {r.status_reason && r.status !== "awaiting_review" && <p className="notice">{r.status_reason}</p>}
                {r.status === "ready" &&
                  (() => {
                    const newer = newerOut(r, releases.data ?? []);
                    return newer ? (
                      <p className="notice small">Version {newer.version} is newer and already published, so this one can't be published.</p>
                    ) : (
                      <PublishPanel
                        slug={slug}
                        release={r}
                        browser={browser}
                        comingSoon={comingSoon}
                        draft={app.data?.status === "draft"}
                        playsNow={playsNow}
                        heldAt={heldAt}
                        picturesPending={Boolean(media.data?.pending)}
                        scanning={server.data?.scanning}
                        onPublished={onPublished}
                        onRefused={changed}
                      />
                    );
                  })()}
                {r.status === "awaiting_review" && (
                  <>
                    <ReleaseHold slug={slug} release={r} scanning={server.data?.scanning} onChanged={changed} />
                    <HeldPublish version={r.version} />
                  </>
                )}
                {(r.status === "failed" || r.status === "rejected") && (
                  <AskAboutFailed slug={slug} release={r} scanning={server.data?.scanning} onChanged={changed} />
                )}
                {/* A failed release's only hold is a question; a turned-down one's past hold folds away until its maker asks. */}
                {r.hold && r.hold.status !== "open" && r.status !== "failed" && !(r.status === "rejected" && r.hold.kinds.includes("question")) && (
                  <PastHold slug={slug} release={r} summary="What held it, and the conversation with SLAPorSLOP staff" />
                )}
                {r.notes && (
                  <div className="release-notes doc" dangerouslySetInnerHTML={{ __html: renderMarkdown(r.notes) }} />
                )}
                <dl className="kv">
                  {r.play_address && (
                    <>
                      <dt>Play address</dt>
                      <dd>
                        <ExternalLink href={r.play_address}>{r.play_address}</ExternalLink>
                      </dd>
                    </>
                  )}
                  <dt>Commit</dt>
                  <dd>
                    <code>{r.commit_sha.slice(0, 12)}</code>{" "}
                    {r.commit_verified ? (
                      <span className="muted">found in your repo</span>
                    ) : (
                      <span className="muted">not found in your repository</span>
                    )}
                  </dd>
                  {r.files.map((f) => (
                    <FileDetail key={f.platform} file={f} />
                  ))}
                </dl>
                {r.play_address && <AddressCheckList checks={r.address_checks} status={r.status} audience="maker" />}
                <SourceCheckList checks={r.source_checks} audience="maker" />
                <RepoScanList
                  scan={r.repo_scan}
                  advisories={r.advisories}
                  audience="maker"
                  browser={browser}
                  server={isDedicatedServer(app.data)}
                />
              </li>
            ))}
          </ul>
        </div>
        <div className="maker-side">
          {releases.data && server.data && (
            <PublishProgress
              app={app.data}
              releases={releases.data}
              server={server.data}
              onListed={app.reload}
            />
          )}
          <UpdatesPanel key={settingsNonce} slug={slug} browser={browser} />
          {/* Slop Build makes downloads, and nothing of a browser game is signed. */}
          {!browser && (
            <p className="panel panel-pad small">
              Build automatically with Slop Build, add Slop Check, or sign builds yourself: <Link to={`/publish/${slug}/builds`}>Builds</Link>.
            </p>
          )}
        </div>
      </div>
    </MakerFrame>
  );
}

const SCAN_BADGE: Record<
  NonNullable<Release["files"][number]["scan_result"]>,
  { label: string; badge: string }
> = {
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
  const label =
    PLATFORMS.find((p) => p.value === file.platform)?.label ?? file.platform;
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
            SHA-256 <code>{file.sha256.slice(0, 16)}…</code> ·{" "}
            {file.size_bytes?.toLocaleString()} bytes
          </div>
        )}
        {file.scan_result && (
          <div className="small">
            <span className={`badge ${SCAN_BADGE[file.scan_result].badge}`}>
              {SCAN_BADGE[file.scan_result].label}
            </span>{" "}
            {file.scan_detail && (
              <span className="muted">{file.scan_detail}</span>
            )}
          </div>
        )}
        <FileChecks file={file} audience="maker" />
        {file.provenance ? (
          <div className="small">
            <span className="badge official">
              {file.provenance.built_by_slop_build
                ? "BUILT BY SLOP BUILD"
                : "SIGNED BY YOUR CI"}
            </span>{" "}
            <span className="muted">
              {workflowName(file.provenance.signer_workflow)}
              {file.provenance.runner === "self-hosted" &&
                " · self-hosted runner"}
            </span>
          </div>
        ) : file.maker_key ? (
          <div className="small">
            <span className="badge official">SIGNED BY YOUR KEY</span>{" "}
            <span className="muted">
              {file.maker_key.key_name} ·{" "}
              <code>{file.maker_key.fingerprint}</code>
            </span>
          </div>
        ) : (
          file.provenance_problem && (
            <div className="small">
              <span className="badge bad">NOT SIGNED</span>{" "}
              <span className="muted">{file.provenance_problem}</span>
            </div>
          )
        )}
      </dd>
    </>
  );
}

/**
 * A browser game's release: its play address, a version (one is suggested: SLAPorSLOP can't see
 * what a site serves, so it's the maker's word), and the commit the site is built from (left out
 * on GitHub: SLAPorSLOP takes the newest on the default branch). No files, runtimes or signature.
 */
function PlayAddressForm({
  slug,
  current,
  start,
  versions,
  rating: storeRating,
  isGithub,
  first,
  onSubmitted,
}: {
  slug: string;
  /** The address Play opens now, if any. */
  current: string | null;
  /** The address the field starts with. */
  start: string;
  /** Every release's version, newest first: a suggestion is never one of them. */
  versions: string[];
  rating: ReleaseRating;
  isGithub: boolean;
  /** The first address, sent with the new store page, if it didn't go through: it waits here. */
  first?: FirstAddress;
  onSubmitted: () => void;
}) {
  const [address, setAddress] = useState(start);
  // Suggested; the maker can type another.
  const [version, setVersion] = useState(() => nextVersion(versions));
  const [commit, setCommit] = useState("");
  const [rating, setRating] = useState(storeRating);
  const [notes, setNotes] = useState("");
  const [error, setError] = useState<ApiError | undefined>(
    first ? new ApiError(422, "invalid", first.message, first.field) : undefined,
  );
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string>();
  const gate = useTermsGate();

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    setDone(undefined);
    const release: NewRelease = {
      version: version.trim(),
      play_address: address.trim(),
      ...(commit.trim() ? { commit_sha: commit.trim().toLowerCase() } : {}),
      rating,
      notes: notes.trim() || undefined,
    };
    try {
      await gate.guard(() => api.submitRelease(slug, release));
      setDone(release.version);
      setVersion(nextVersion([release.version, ...versions]));
      setCommit("");
      setNotes("");
      onSubmitted();
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };
  const fieldError = (name: string) =>
    error?.field === name && (
      <span className="field-error" role="alert">
        {error.message}
      </span>
    );

  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)} aria-labelledby="play-address-title">
      {gate.prompt}
      <h2 id="play-address-title">{current ? "A new play address or version" : "Your play address"}</h2>
      {first && !done && (
        <p className="notice" role="alert">
          Your store page is made, but its play address didn't go through. Fix it below and send it again.
        </p>
      )}
      {done && (
        <p className="notice" role="status">
          Version {done} sent. SLAPorSLOP is checking the address; once it passes, publish it below and Play opens it.
        </p>
      )}
      <label>
        Play address
        <input
          type="url"
          value={address}
          maxLength={2000}
          required
          placeholder="https://yourgame.example/"
          onChange={(e) => setAddress(e.target.value)}
        />
        <span className="hint">
          Where Play sends players: https, on a site of yours. SLAPorSLOP checks every new address before you can publish it.
        </span>
        {fieldError("play_address")}
      </label>
      <div className="row-2">
        <label>
          Version
          <input value={version} placeholder="1.0.0" maxLength={40} required onChange={(e) => setVersion(e.target.value)} />
          <span className="hint">Yours to choose: players see it beside their votes and bug reports.</span>
          {fieldError("version")}
        </label>
        <label>
          Commit{isGithub && <span className="muted"> (optional)</span>}
          <input
            value={commit}
            placeholder="Full 40-character SHA"
            maxLength={40}
            minLength={40}
            required={!isGithub}
            onChange={(e) => setCommit(e.target.value)}
          />
          <span className="hint">
            {isGithub
              ? "The commit your site is built from. Leave it empty and SLAPorSLOP takes the newest on your default branch."
              : "The commit your site is built from: SLAPorSLOP reads your source there."}
          </span>
          {fieldError("commit_sha")}
        </label>
      </div>
      <RatingFields value={rating} storeRating={storeRating} onChange={setRating} />
      <NotesField value={notes} onChange={setNotes} />
      {error && !["play_address", "version", "commit_sha"].includes(error.field ?? "") && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="actions">
        <button className="btn slime" disabled={busy}>
          {busy ? "Sending…" : "Send the address"}
        </button>
      </div>
    </form>
  );
}

function ReleaseForm({
  repoUrl,
  slug,
  platforms,
  rating: storeRating,
  lastRuntimes,
  onSubmitted,
}: {
  repoUrl: string;
  slug: string;
  platforms: BuildPlatform[];
  rating: ReleaseRating;
  lastRuntimes: string[];
  onSubmitted: () => void;
}) {
  const [rating, setRating] = useState(storeRating);
  const [runtimes, setRuntimes] = useState(lastRuntimes);
  const [notes, setNotes] = useState("");
  const blank = () =>
    platforms.map((platform): FileRow => ({
      platform,
      url: "",
      exec: "",
      args: "",
    }));
  const [version, setVersion] = useState("");
  const [commit, setCommit] = useState("");
  const [rows, setRows] = useState<FileRow[]>(blank);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const gate = useTermsGate();

  const update = (i: number, change: Partial<FileRow>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...change } : r)));

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
        ...(startsProgram(r.platform)
          ? { exec: r.exec.trim() }
          : {}),
        args: r.args.trim() ? r.args.trim().split(/\s+/) : [],
      })),
      rating,
      runtimes: runtimes.length > 0 || lastRuntimes.length > 0 ? runtimes : undefined,
      notes: notes.trim() || undefined,
    };
    try {
      await gate.guard(() => api.submitRelease(slug, release));
      setDone(true);
      setVersion("");
      setCommit("");
      setNotes("");
      setRows(blank());
      onSubmitted();
    } catch (err) {
      setError(
        err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)),
      );
    } finally {
      setBusy(false);
    }
  };

  const isGithub = repoUrl.startsWith("https://github.com/");

  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
      {gate.prompt}
      <h2>New release</h2>
      {done && (
        <p className="notice" role="status">
          Submitted. SLAPorSLOP is checking the files; you'll see how it goes
          below, and publish it there once it passes.
        </p>
      )}
      <div className="row-2">
        <label>
          Version
          <input
            value={version}
            placeholder="1.0.0"
            maxLength={40}
            required
            onChange={(e) => setVersion(e.target.value)}
          />
          {error?.field === "version" && (
            <span className="field-error">{error.message}</span>
          )}
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
          <span className="hint">
            The commit these builds came from. Tags can move; commits can't.
          </span>
          {error?.field === "commit_sha" && (
            <span className="field-error">{error.message}</span>
          )}
        </label>
      </div>

      {rows.map((row, i) => (
        <fieldset key={row.platform} className="file-row">
          <legend>
            {PLATFORM_LABEL[row.platform]}{" "}
            <span className="muted small">
              — {BUILD[row.platform].describe}
            </span>
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
              {isGithub
                ? "A file on this repository's GitHub releases page."
                : "On the same site as your repository."}
            </span>
          </label>
          {startsProgram(row.platform) && (
            <div className="row-2">
              <label>
                Program to start
                <input
                  value={row.exec}
                  required
                  placeholder={
                    row.platform === "windows" ? "my-game.exe" : "bin/my-game"
                  }
                  onChange={(e) => update(i, { exec: e.target.value })}
                />
                <span className="hint">Path inside the archive.</span>
              </label>
              <label>
                Arguments (optional)
                <input
                  value={row.args}
                  placeholder="--fullscreen"
                  onChange={(e) => update(i, { args: e.target.value })}
                />
              </label>
            </div>
          )}
        </fieldset>
      ))}
      <RatingFields value={rating} storeRating={storeRating} onChange={setRating} />
      <RuntimeFields value={runtimes} onChange={setRuntimes} />
      <NotesField value={notes} onChange={setNotes} />
      {error && error.field !== "version" && error.field !== "commit_sha" && (
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

const PLATFORM_LABEL = Object.fromEntries(
  PLATFORMS.map((p) => [p.value, p.label]),
) as Record<Platform, string>;

interface PickRow {
  include: boolean;
  name: string;
  url: string;
  platform: BuildPlatform;
  exec: string;
}

/** Starts from SLAPorSLOP's guesses: the first file that fits each of the app's platforms is ticked. */
export function pickRows(
  release: GithubRelease,
  platforms: BuildPlatform[],
): PickRow[] {
  const taken = new Set<BuildPlatform>();
  return release.assets.map((a) => {
    const guess =
      platforms.find((p) => p === a.platform) ??
      platforms.find((p) => fitsPlatform(p, a.name));
    const include = guess !== undefined && !taken.has(guess);
    if (guess) taken.add(guess);
    return {
      include,
      name: a.name,
      url: a.url,
      platform: guess ?? platforms[0],
      exec: "",
    };
  });
}

/** "Pick a release": the repository's GitHub releases, so makers don't type versions, commits or links. */
function ReleasePicker({
  slug,
  platforms,
  rating: storeRating,
  lastRuntimes,
  onSubmitted,
}: {
  slug: string;
  platforms: BuildPlatform[];
  rating: ReleaseRating;
  lastRuntimes: string[];
  onSubmitted: () => void;
}) {
  const [rating, setRating] = useState(storeRating);
  const [runtimes, setRuntimes] = useState(lastRuntimes);
  const [notes, setNotes] = useState("");
  const list = useLoad(() => api.githubReleases(slug), [slug]);
  const picker = useTermsGate();
  const [tag, setTag] = useState("");
  const [rows, setRows] = useState<PickRow[]>([]);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string>();

  const releases = list.data ?? [];
  const chosen = releases.find((r) => r.tag === tag);
  /** `keepNotice`: picked by the page itself (after sending one), so "sent" stays up. */
  const choose = (t: string, keepNotice = false) => {
    setTag(t);
    setError(undefined);
    if (!keepNotice) setDone(undefined);
    const r = releases.find((x) => x.tag === t);
    setRows(r ? pickRows(r, platforms) : []);
  };
  // Start on the newest release that hasn't been sent yet.
  useEffect(() => {
    if (tag || !list.data) return;
    const first =
      list.data.find((r) => !r.submitted && !r.prerelease) ??
      list.data.find((r) => !r.submitted);
    if (first) choose(first.tag, true);
  }, [list.data]);

  const update = (i: number, change: Partial<PickRow>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...change } : r)));
  const included = rows.filter((r) => r.include);
  const chosenPlatforms = included.map((r) => r.platform);
  const duplicate = chosenPlatforms.find(
    (p, i) => chosenPlatforms.indexOf(p) !== i,
  );
  const missing = platforms.filter((p) => !chosenPlatforms.includes(p));
  const wrongType = included.find((r) => !fitsPlatform(r.platform, r.name));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!chosen) return;
    setBusy(true);
    setError(undefined);
    try {
      await picker.guard(() => api.submitGithubRelease(
        slug,
        chosen.tag,
        included.map((r) => ({
          platform: r.platform,
          kind: BUILD[r.platform].kind,
          url: r.url,
          ...(startsProgram(r.platform)
            ? { exec: r.exec.trim() }
            : {}),
        })),
        rating,
        runtimes,
        notes.trim() || undefined,
      ));
      setDone(chosen.version);
      setNotes("");
      list.reload();
      setTag("");
      setRows([]);
      onSubmitted();
    } catch (err) {
      setError(
        err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
      {picker.prompt}
      <h2>Submit a release</h2>
      <p className="muted small">
        Make a release on GitHub with your builds attached, then pick it here.
        SLAPorSLOP reads the version from its tag and the commit from GitHub. Each
        release needs:{" "}
        {platforms
          .map((p) => `${PLATFORM_LABEL[p]} (${BUILD[p].describe})`)
          .join(", ")}
        .
      </p>
      {done && (
        <p className="notice" role="status">
          Version {done} sent. SLAPorSLOP is checking the files; follow it in the
          progress above.
        </p>
      )}
      {list.loading && !list.data && (
        <p className="muted">Reading your GitHub releases…</p>
      )}
      {list.error && (
        <p className="form-error" role="alert">
          {list.error.message}
        </p>
      )}
      {list.data && releases.length === 0 && (
        <p className="notice">
          Your repository has no releases yet. On GitHub, open{" "}
          <b>Releases → Draft a new release</b>, give it a tag like{" "}
          <code>v1.0.0</code>, attach your builds and publish it. Then come back
          and{" "}
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
          {rows.length === 0 && (
            <p className="notice">
              This release has no files attached. Add your builds to it on
              GitHub.
            </p>
          )}
          {rows.length > 0 && (
            <p className="small">
              <b>Which files are builds?</b> Tick one for each platform.
            </p>
          )}
          {rows.map((row, i) => (
            <fieldset
              key={row.name}
              className={`file-row pick-row ${row.include ? "" : "off"}`}
            >
              <label className="check">
                <input
                  type="checkbox"
                  checked={row.include}
                  onChange={(e) => update(i, { include: e.target.checked })}
                />
                <code>{row.name}</code>
              </label>
              {row.include && (
                <label>
                  Build for
                  <select
                    value={row.platform}
                    onChange={(e) =>
                      update(i, { platform: e.target.value as BuildPlatform })
                    }
                  >
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
                  The {PLATFORM_LABEL[row.platform]} build must be{" "}
                  {BUILD[row.platform].describe}.
                </p>
              )}
              {row.include && startsProgram(row.platform) && (
                <label>
                  Program to start
                  <input
                    value={row.exec}
                    required
                    placeholder="my-game.exe or bin/my-game"
                    onChange={(e) => update(i, { exec: e.target.value })}
                  />
                  <span className="hint">
                    Its path inside the zip. SLAPorSLOP remembers it for your next
                    releases.
                  </span>
                </label>
              )}
            </fieldset>
          ))}
          {duplicate && (
            <p className="form-error">
              Pick one file for {PLATFORM_LABEL[duplicate]}.
            </p>
          )}
          {rows.length > 0 && missing.length > 0 && (
            <p className="notice">
              Still needed:{" "}
              {missing
                .map((p) => `${PLATFORM_LABEL[p]} (${BUILD[p].describe})`)
                .join(", ")}
              . If the release doesn't have it, attach it on GitHub and pick the
              release again.
            </p>
          )}
          {error && (
            <p className="form-error" role="alert">
              {error.message}
            </p>
          )}
          <RatingFields value={rating} storeRating={storeRating} onChange={setRating} />
          <RuntimeFields value={runtimes} onChange={setRuntimes} />
          <NotesField value={notes} onChange={setNotes} fromGithub />
          <div className="actions">
            <button
              className="btn slime"
              disabled={
                busy ||
                included.length === 0 ||
                Boolean(duplicate) ||
                missing.length > 0 ||
                Boolean(wrongType)
              }
            >
              {busy ? "Sending…" : `Send version ${chosen.version}`}
            </button>
          </div>
        </>
      )}
    </form>
  );
}

/**
 * Automatic pickup of new GitHub releases. A browser game has none to pick up (its releases are
 * play addresses), so it shows nothing. Every release is published by its maker: nothing goes to
 * players automatically. `onChanged` tells the page after a change.
 */
export function UpdatesPanel({ slug, browser = false, onChanged }: { slug: string; browser?: boolean; onChanged?: () => void }) {
  const loaded = useLoad(() => (browser ? Promise.resolve(undefined) : api.updateSettings(slug)), [slug, browser]);
  const [settings, setSettings] = useState<UpdateSettings>();
  const [error, setError] = useState<string>();
  const s = settings ?? loaded.data;
  if (browser || !s) return null;

  const change = async (c: Partial<Pick<UpdateSettings, "watch_releases">>) => {
    setError(undefined);
    try {
      setSettings(await api.changeUpdateSettings(slug, c));
      onChanged?.();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };

  return (
    <section className="panel panel-pad form updates-panel">
      <h2>Updates</h2>
      <label className="check">
        <input
          type="checkbox"
          checked={s.watch_releases}
          onChange={(e) => void change({ watch_releases: e.target.checked })}
        />
        <span>
          <b>Pick up new GitHub releases</b>
          <span className="muted small" style={{ display: "block" }}>
            When you publish a new release on GitHub (not a draft or
            pre-release), SLAPorSLOP sends it for you, usually within 10 minutes
            (longer for apps that haven't updated in a while). It expects the
            same files as last time, with the new version in their names. Once
            it passes the checks, you publish it here.
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
              <span className="badge official">GITHUB APP INSTALLED</span>{" "}
              GitHub tells SLAPorSLOP the moment you publish a release, so it's picked
              up within a minute.
            </>
          ) : (
            <>
              <b>Faster pickup:</b> install SLAPorSLOP's GitHub App on this repository
              and GitHub tells SLAPorSLOP the moment you publish a release, instead of
              SLAPorSLOP checking every so often. It reads your releases and code,
              and never changes them.{" "}
              <ExternalLink href={s.github_app.install_url}>
                Install on GitHub
              </ExternalLink>
            </>
          )}
        </div>
      )}
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}

/** "1 thing" or "3 things". */
const things = (n: number) => (n === 1 ? "1 thing" : `${n} things`);

/**
 * What publishing does, in words that can't be mistaken (owner request 2026-11-20): `what` beside
 * the button, `effect` in the confirm step, for every case. Publishing makes it the version players
 * get, and nothing else: it doesn't put a draft in the store (Put it in the store on Manage does),
 * and a Coming soon page's builds can't be downloaded at all, so Move to Beta is what lets players
 * get it (a browser game: what makes Play open it). `effect` is undefined for a download in the
 * store and playable, whose confirm step says how to roll back. Exported for tests.
 */
export function publishWords({
  browser,
  comingSoon,
  draft,
  heldAt,
}: {
  browser: boolean;
  comingSoon: boolean;
  draft: boolean;
  heldAt?: string;
}): { what: string; effect?: string } {
  const notInStore = `Publishing doesn't put the ${browser ? "game" : "app"} in the store: Put it in the store, on its Manage page, does.`;
  const toBeta = browser
    ? "Play opens nothing on a Coming soon page: moving the game to Beta (Move to Beta, at the top of this page) is what lets players play it."
    : "Nobody can download a Coming soon page's builds: moving the page to Beta (Move to Beta, at the top of this page) is what lets players get it.";
  const ends = heldAt ? ` The hold at ${heldAt} ends.` : "";
  if (browser) {
    if (draft) {
      return {
        what: `Once you publish it, it's the address Play will open. ${notInStore}${comingSoon ? ` ${toBeta}` : ""}`,
        effect: comingSoon
          ? "Play opens it once the game is in the store and in Beta."
          : "Players can play it once you put the game in the store.",
      };
    }
    if (comingSoon) {
      return {
        what: `Once you publish it, it's the address Play will open. ${toBeta}`,
        effect: `Play opens its address once the game is in Beta.${ends} To change it later, publish a newer one.`,
      };
    }
    return {
      what: "Once you publish it, Play opens its address.",
      effect: `Play opens its address straight away.${ends} To change it later, publish a newer one.`,
    };
  }
  if (draft) {
    return {
      what: `Once you publish it, SLAPorSLOP signs it and it's the version players will get. ${notInStore}${comingSoon ? ` ${toBeta}` : ""}`,
      effect: comingSoon
        ? "Players can get it once the app is in the store and in Beta."
        : "Players can get it once you put the app in the store.",
    };
  }
  if (comingSoon) {
    return {
      what: `Once you publish it, SLAPorSLOP signs it and it's the version players will get. ${toBeta}`,
      effect: `Players can get it once the page is in Beta.${ends} Publishing doesn't change the page's stage.`,
    };
  }
  return { what: "Once you publish it, SLAPorSLOP signs it and players get it." };
}

/**
 * Publish, on a release that passed every check (publish.rs): what to know first, as information
 * (a hold it ends, a new site, what the source check found), what it waits for, if anything, and
 * the button, with a confirm step. No checkbox and no signing in again: the maker is signed in.
 * While something must be fixed in it (and staff haven't allowed it anyway), the button is
 * unavailable and the panel lists what, links to the fixes and offers to ask SLAPorSLOP staff
 * (owner decision 2026-11-20); what should be fixed is only a note.
 */
export function PublishPanel({
  slug,
  release: r,
  browser,
  comingSoon,
  draft = false,
  playsNow,
  heldAt,
  picturesPending,
  scanning,
  onPublished,
  onRefused,
}: {
  slug: string;
  release: Release;
  browser: boolean;
  comingSoon: boolean;
  /** This server's scanner is on (for the conversation with staff). */
  scanning?: boolean;
  /** The app isn't in the store yet: players get nothing until its maker puts it there. */
  draft?: boolean;
  /** The address Play opens now (a browser game). */
  playsNow: string | null;
  /** The version players are held at, if they are. */
  heldAt?: string;
  picturesPending: boolean;
  onPublished: (r: Release) => void;
  /** It couldn't be published (it went back on hold, say): the page looks again. */
  onRefused: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const titleId = useId();
  const waitId = useId();
  const blockId = useId();
  const { opener, close } = useConfirmStep(confirming, setConfirming);
  const blocked = publishBlocked(r);
  const mustFix = r.must_fix ?? [];
  const shouldFix = r.should_fix ?? [];
  const waits = blocked ? undefined : publishWait(r, picturesPending);
  const notes = publishNotes(r, { heldAt, playsNow });
  const fixesLink = `/publish/${slug}/fixes`;
  // While SLAPorSLOP is in beta, going public may need the invite code first (invites.rs).
  const invite = useInviteGate();
  const publish = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const done = await invite.guard(() => api.publishRelease(slug, r.id));
      setConfirming(false);
      onPublished(done);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      onRefused();
    } finally {
      setBusy(false);
    }
  };
  // What publishing does, and (in the confirm step) what happens at once and how to go back on it.
  const { what, effect } = publishWords({ browser, comingSoon, draft, heldAt });
  if (blocked) {
    return (
      <section className="publish-panel" aria-labelledby={titleId}>
        <h3 id={titleId}>Publish version {r.version}</h3>
        <p className="small">
          It passed SLAPorSLOP's checks, but {things(mustFix.length)} must be fixed before you can publish it:
        </p>
        <ul className="fix-list small" aria-label="Must be fixed before publishing">
          {mustFix.map((f, i) => (
            <li key={`${f.kind}-${i}`}>
              <span className="badge bad">MUST FIX</span> {f.problem}
            </li>
          ))}
        </ul>
        <p className="small">
          <Link to={fixesLink}>See how to fix {mustFix.length === 1 ? "it" : "them"}</Link> (with "Copy for your AI"). What
          your store page says, like going online, counts as soon as you change the page; the rest needs a new version.
        </p>
        <div className="row-actions held-publish">
          <button type="button" className="btn slime small" disabled aria-describedby={blockId}>
            Publish version {r.version}…
          </button>
          <span id={blockId} className="small muted">
            You can publish it once {mustFix.length === 1 ? "that's" : "they're"} fixed.
          </span>
        </div>
        <AskAboutFailed slug={slug} release={r} scanning={scanning} onChanged={onRefused} />
      </section>
    );
  }
  return (
    <section className="publish-panel" aria-labelledby={titleId}>
      <h3 id={titleId}>Publish version {r.version}</h3>
      <p className="small">It passed SLAPorSLOP's checks. {what}</p>
      {r.must_fix_waived_at && mustFix.length > 0 && (
        <p className="small">
          SLAPorSLOP staff allowed publishing it on {formatDate(r.must_fix_waived_at)}, with what must be fixed in it:{" "}
          {mustFix.map((f) => f.problem).join(" ")} Fix it in your next version.
        </p>
      )}
      {shouldFix.length > 0 && (
        <p className="small">
          {things(shouldFix.length)} should be fixed: <Link to={fixesLink}>see what and how</Link>. You can publish it anyway.
        </p>
      )}
      {notes.length > 0 && (
        <ul className="small publish-notes" aria-label="Before you publish">
          {notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
      {waits && (
        <p id={waitId} className="notice small" role="status">
          {waits}
        </p>
      )}
      {!confirming ? (
        <div className="row-actions">
          <button
            ref={opener}
            type="button"
            className="btn slime small"
            disabled={Boolean(waits) || busy}
            aria-describedby={waits ? waitId : undefined}
            onClick={() => setConfirming(true)}
          >
            Publish version {r.version}…
          </button>
        </div>
      ) : (
        <div className="stage-confirm" role="group" aria-label={`Publish version ${r.version}`}>
          <b>Publish version {r.version} now?</b>
          <p className="small">
            {effect ?? (
              <>
                Players get it straight away{heldAt ? `, and the hold at ${heldAt} ends` : ""}. To roll back later, hold players at
                an earlier version on <Link to={`/publish/${slug}/manage`}>Manage</Link>, or publish a newer one.
              </>
            )}
          </p>
          <div className="row-actions">
            <button type="button" className="btn small slime" autoFocus disabled={busy} onClick={() => void publish()}>
              {busy ? "Publishing…" : "Yes, publish it"}
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
      {invite.prompt}
    </section>
  );
}

/**
 * A release on hold can't be published yet: the button, unavailable, with why (so Publish is never
 * a surprise once the hold ends).
 */
function HeldPublish({ version }: { version: string }) {
  const id = useId();
  return (
    <div className="row-actions held-publish">
      <button type="button" className="btn slime small" disabled aria-describedby={id}>
        Publish version {version}
      </button>
      <span id={id} className="small muted">
        You can publish once the hold ends.
      </span>
    </div>
  );
}

/** This release's rating (AGE-7): the store page's to start with. */
function RatingFields({
  value,
  storeRating,
  onChange,
}: {
  value: ReleaseRating;
  storeRating: ReleaseRating;
  onChange: (r: ReleaseRating) => void;
}) {
  const lowered =
    value.min_age < storeRating.min_age || storeRating.content_labels.some((l) => !value.content_labels.includes(l));
  return (
    <details className="release-rating">
      <summary className="small">
        Rating for this release: <b>{value.min_age === 0 ? "Everyone" : `${value.min_age}+`}</b>
        {value.content_labels.length > 0 && ` · ${value.content_labels.map((l) => CONTENT_LABEL[l]).join(", ")}`}{" "}
        <span className="muted">(change it if this version's content is different)</span>
      </summary>
      <label>
        Age rating
        <AgeSelect value={value.min_age} onChange={(a) => onChange({ ...value, min_age: a })} />
      </label>
      <ContentLabelPicker value={value.content_labels} onChange={(l) => onChange({ ...value, content_labels: l })} />
      <p className="muted small">
        {lowered
          ? "Lower than the store page now: a SLAPorSLOP staff member checks a lowered rating before you can publish this release."
          : "When you publish this release, the store page shows its rating."}
      </p>
    </details>
  );
}

/** Shared runtimes the release needs: the SLAPorSLOP app installs missing ones from the vendor. */
function RuntimeFields({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const list = useLoad(() => api.runtimes(), []);
  if (!list.data) return null;
  return (
    <details className="release-rating">
      <summary className="small">
        Shared runtimes (Windows):{" "}
        <b>{value.length === 0 ? "none" : list.data.filter((r) => value.includes(r.id)).map((r) => r.name).join(", ")}</b>
      </summary>
      <p className="muted small">
        Tick what your game needs and doesn't ship. Before it starts, the SLAPorSLOP app installs any that are missing, from the
        vendor, once the player agrees.
      </p>
      {list.data.map((r) => (
        <label className="check" key={r.id}>
          <input
            type="checkbox"
            checked={value.includes(r.id)}
            onChange={(e) => onChange(e.target.checked ? [...value, r.id] : value.filter((x) => x !== r.id))}
          />{" "}
          {r.name}
        </label>
      ))}
    </details>
  );
}

/** What changed, for players (Markdown, up to 5,000 characters). */
function NotesField({ value, onChange, fromGithub }: { value: string; onChange: (v: string) => void; fromGithub?: boolean }) {
  return (
    <label>
      What's new, for players (optional)
      <textarea
        value={value}
        maxLength={5000}
        rows={4}
        placeholder={fromGithub ? "Leave empty to use the GitHub release's text" : "What changed since the last version"}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
}
