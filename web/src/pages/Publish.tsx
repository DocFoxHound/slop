// SPDX-License-Identifier: AGPL-3.0-or-later
// Makers' side: their apps with where each one is on its way into the store, and the draft editor.
// A store page either offers downloads (Windows and/or Linux builds) or plays in the browser, on
// the maker's own website (DESIGN §8): a new browser game's page takes its first play address,
// sent as its first release once the draft is made.

import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "react-router";

import {
  api,
  ApiError,
  type AppDetail,
  type AppKind,
  ACCESSIBILITY,
  type AppPlatform,
  type AppStage,
  type AppSummary,
  type Category,
  type ContentLabel,
  type ModsFolder,
  type NewApp,
  type OnlineKind,
  type RepoTokenInput,
  PROVIDER_LABEL,
  type Release,
  type RepoOwnership as Ownership,
  type DonationSettings,
} from "../api";
import { AppIcon, appIconProps } from "../components/AppIcon";
import { DonationsPanel } from "../components/DonationsManage";
import { HandoffError, HandoffNotice, HandoffWaiting, useHandoff } from "../components/Handoff";
import { LicensePicker } from "../components/LicensePicker";
import { LinkSignIn } from "../components/LinkSignIn";
import { CopyLine, siteAddress, startPrompt } from "../components/CopyLine";
import { TagPicker } from "../components/TagPicker";
import { guessKind, isGithub, TokenFields } from "../components/RepoToken";
import { MediaEditor } from "../components/MediaEditor";
import { MakerFrame } from "../components/MakerFrame";
import { currentStep, progressSteps, STEP_ICON, STEP_LABEL, type Step } from "../components/PublishProgress";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { CONTENT_LABEL, ONLINE_LABEL, playsInBrowser } from "../format";
import { useDocumentTitle, useInterval, useLoad } from "../hooks";
import { useSession } from "../session";
import { PasskeyNotice } from "../components/PasskeyNotice";
import { useTermsGate } from "../components/Terms";
import { APP_LABELS, appFill, forgetHandoff, type AppFields, type Fill, type HandoffLink } from "../handoff";
import type { FirstAddress } from "./Releases";

function SignInFirst({ what, handoff = null }: { what: string; handoff?: HandoffLink | null }) {
  return (
    <div className="page narrow">
      <HandoffWaiting link={handoff} />
      <div className="panel panel-pad">
        <h1>Publish on SLAPorSLOP</h1>
        <p>
          Any open-source app or game made with AI can go on SLAPorSLOP. Listing is free; SLAPorSLOP's only fee is 5% of donations,
          which go straight to the maker.
        </p>
        <p className="row-actions">
          <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(what)}`}>
            Sign in to start
          </Link>
          <Link className="btn" to="/publish/guide">
            How to publish
          </Link>
        </p>
        <p className="small">
          <b>Using an AI assistant?</b> It can get your project ready and walk you through the rest. Paste this in:
        </p>
        <CopyLine text={startPrompt(siteAddress(), "publish")} label="Copy the line for your AI assistant" />
      </div>
    </div>
  );
}

export function Publish() {
  useDocumentTitle("Publish");
  const { me } = useSession();
  const mine = useLoad(() => (me ? loadStatuses() : Promise.resolve([])), [me?.id]);

  // While something is moving, check again now and then: often while files are being checked,
  // every minute while a review or staff are awaited; never once nothing is under way.
  const checking = mine.data?.some((a) => a.latest?.status === "fetching") ?? false;
  const moving = mine.data?.some((a) => a.current.state === "active") ?? false;
  useInterval(mine.reload, checking ? 15_000 : moving ? 60_000 : null);
  // Released and beta apps without donations get a pointer to where they're turned on (the
  // Get paid section of the app's Edit store page).
  const offerable = (mine.data ?? []).filter((a) => a.app.stage !== "coming_soon").map((a) => a.app.slug);
  const donations = useLoad(() => loadDonations(offerable), [offerable.join(",")]);

  if (me === undefined) return <Loading />;
  if (!me) return <SignInFirst what="/publish" />;

  return (
    <div className="page">
      <div className="title-row spread">
        <h1 className="page-title">Your apps</h1>
        <div className="row-actions">
          <Link className="btn" to="/publish/guide">
            How to publish
          </Link>
          <Link className="btn slime" to="/publish/new">
            + New app
          </Link>
        </div>
      <PasskeyNotice />
      </div>
      <p className="muted">
        Each app goes through the same steps: a store page, a release (for a browser game, its play address), SLAPorSLOP's
        checks, a review by a person, then the store. This is where each one is.
      </p>
      {mine.error && <ErrorState error={mine.error} onRetry={mine.reload} />}
      {mine.data && mine.data.length === 0 && (
        <div className="empty-state">
          You haven't made anything yet. <Link to="/publish/new">Create your first draft</Link>.
        </div>
      )}
      {!mine.data && !mine.error && <Loading />}
      <ul className="status-list">
        {mine.data?.map(({ app, latest, current }) => (
          <li key={app.slug} className="panel panel-pad status-row">
            <div className={`step ${current.state}`}>
              <span className="step-icon" aria-hidden="true">
                {STEP_ICON[current.state]}
              </span>
              <div>
                <div className="title-row">
                  <AppIcon {...appIconProps(app)} size={24} />
                  <Link to={`/app/${app.slug}`}>
                    <b>{app.title}</b>
                  </Link>
                  {app.server_for ? (
                    <span className="muted small">
                      Dedicated server for {mine.data?.find((m) => m.app.slug === app.server_for)?.app.title ?? app.server_for}
                    </span>
                  ) : (
                    <>
                      {!app.icon_url && <span className="muted small">No icon yet</span>}
                      <span className="badge neutral">{STAGE_LABEL[app.stage]}</span>
                    </>
                  )}
                  {latest && <span className="muted small">v{latest.version}</span>}
                </div>
                <div className="small">
                  {statusHeadline(current)} <span className="step-state">{current.stateLabel ?? STEP_LABEL[current.state]}</span>
                </div>
                <div className="muted small">{current.detail}</div>
                {current.more && <div className="muted small step-more">{current.more}</div>}
                {!app.server_for && donationsOff(donations.data?.[app.slug]) && (
                  <div className="small donations-off">
                    Donations: off ·{" "}
                    <Link to={`/publish/${app.slug}#donations`} aria-label={`Set up donations for ${app.title}`}>
                      Set up
                    </Link>
                  </div>
                )}
              </div>
            </div>
            <div className="row-actions">
              <Link className="btn small" to={`/publish/${app.slug}/manage`}>
                Manage
              </Link>
              <Link className="btn small" to={`/publish/${app.slug}`}>
                Edit page
              </Link>
              <Link className="btn small slime" to={`/publish/${app.slug}/releases`}>
                Releases &amp; progress
              </Link>
            </div>
          </li>
        ))}
      </ul>
      {mine.data && (
        <section className="ai-start" aria-label="Using an AI assistant">
          <p className="small">
            <b>Using an AI assistant?</b> It can get {mine.data.length === 0 ? "your project" : "your next project"} ready and
            walk you through the rest. Paste this in:
          </p>
          <CopyLine text={startPrompt(siteAddress(), "publish")} label="Copy the line for your AI assistant" />
        </section>
      )}
    </div>
  );
}

const STAGE_LABEL: Record<AppStage, string> = { coming_soon: "Coming soon", beta: "Beta", released: "Released" };

interface AppStatusRow {
  app: AppSummary;
  latest: Release | undefined;
  current: Step;
}

async function loadStatuses(): Promise<AppStatusRow[]> {
  const [apps, server] = await Promise.all([api.myApps(), api.server()]);
  return Promise.all(
    apps.map(async (app) => {
      const releases = await api.releases(app.slug);
      return { app, latest: releases[0], current: currentStep(progressSteps(app, releases, server)) };
    }),
  );
}

/** Each app's donation settings, by slug; null where they couldn't be read. */
async function loadDonations(slugs: string[]): Promise<Record<string, DonationSettings | null>> {
  const pairs = await Promise.all(
    slugs.map((slug) =>
      api.donations.settings(slug).then(
        (s): [string, DonationSettings | null] => [slug, s],
        (): [string, DonationSettings | null] => [slug, null],
      ),
    ),
  );
  return Object.fromEntries(pairs);
}

/** Donations could be on for this app, but aren't (a private app takes none). */
function donationsOff(s: DonationSettings | null | undefined): boolean {
  return Boolean(s && !s.donations_on && !s.private && s.payments !== "off");
}

function statusHeadline(step: Step): string {
  if (step.state === "problem") return `Stuck at: ${step.title}`;
  if (step.state === "done") return "In the store";
  if (step.title === "Release") return "Next: submit a release";
  if (step.title === "Play address") return "Next: add the play address";
  return `Waiting on: ${step.title}`;
}

const EMPTY: NewApp = {
  slug: "",
  title: "",
  summary: "",
  description_md: "",
  kind: "game",
  stage: "coming_soon",
  repo_url: "",
  license_spdx: "",
  min_age: 0,
  content_labels: [],
  tags: [],
  platforms: [],
  online: [],
  online_hosts: [],
  sandbox_ready: false,
  mods_folder: null,
  accessibility: [],
  bug_reports: true,
};

function slugify(title: string): string {
  return title
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60);
}

/** /publish/new creates a draft; /publish/:slug edits one. */
export function AppEditor() {
  const { slug } = useParams();
  const { me } = useSession();
  const existing = useLoad(() => (slug && me ? api.app(slug) : Promise.resolve(undefined)), [slug, me?.id]);
  const categories = useLoad(() => api.categories(), []);
  useDocumentTitle(slug ? "Edit app" : "New app");
  // A new app's store page prepared by the person's AI assistant (handoff.ts): kept through
  // signing in, and filled in once the form shows.
  const { link: handoff, n } = useHandoff("app", !slug);
  const [clearedLink, setClearedLink] = useState<number>();
  const fill = useMemo(
    () =>
      handoff && "fields" in handoff && categories.data && clearedLink !== n
        ? appFill(handoff.fields, new Set(categories.data.flatMap((c) => c.tags.map((t) => t.slug))))
        : undefined,
    [handoff, categories.data, clearedLink, n],
  );

  if (me === undefined) return <Loading />;
  if (!me) return <SignInFirst what={slug ? `/publish/${slug}` : "/publish/new"} handoff={handoff} />;
  if (existing.error) return <div className="page"><ErrorState error={existing.error} /></div>;
  if (slug && !existing.data) return <Loading />;
  if (existing.data && !existing.data.is_mine) return <div className="page"><ErrorState error={new ApiError(404, "not_found", "")} /></div>;
  if (!categories.data) return <Loading />;

  return (
    <EditorForm
      key={slug ?? `new-${n}${fill ? "-filled" : ""}`}
      existing={existing.data}
      categories={categories.data}
      fill={fill}
      linkError={handoff && "error" in handoff ? handoff.error : undefined}
      onClearFill={() => setClearedLink(n)}
    />
  );
}

function EditorForm({
  existing,
  categories,
  fill,
  linkError,
  onClearFill,
}: {
  existing?: AppDetail;
  categories: Category[];
  /** What the person's AI assistant filled in (a new app only). */
  fill?: Fill<AppFields>;
  linkError?: string;
  onClearFill?: () => void;
}) {
  const navigate = useNavigate();
  // A play address from the link isn't part of the store page: it's sent as the first release.
  const { play_address: filledAddress, ...filled } = fill?.values ?? {};
  const [playAddress, setPlayAddress] = useState(filledAddress ?? "");
  const [form, setForm] = useState<NewApp>(
    existing
      ? {
          slug: existing.slug,
          title: existing.title,
          summary: existing.summary,
          description_md: existing.description_md,
          kind: existing.kind,
          stage: existing.stage,
          repo_url: existing.repo_url,
          license_spdx: existing.license_spdx,
          min_age: existing.min_age,
          content_labels: existing.content_labels,
          tags: existing.tags,
          platforms: existing.platforms,
          online: existing.online,
          online_hosts: existing.online_hosts,
          sandbox_ready: existing.sandbox_ready,
          mods_folder: existing.mods_folder,
          accessibility: existing.accessibility,
          bug_reports: existing.bug_reports,
        }
      : {
          ...EMPTY,
          ...filled,
          slug: filled.slug ?? slugify(filled.title ?? ""),
        },
  );
  // In the form now: a reload shows it empty again, not the link's fill over the person's changes.
  useEffect(() => {
    if (fill) forgetHandoff("app");
  }, [fill]);
  // Once players have the app, moving it to another repository is reviewed first (Manage).
  const repoLocked = useLoad(
    () => (existing ? api.manage(existing.slug).then((m) => m.repo_locked) : Promise.resolve(false)),
    [existing?.slug],
  ).data;
  // An app with releases keeps its kind of release: files, or play addresses (catalog.rs update_app).
  const hasReleases = useLoad(
    () => (existing ? api.releases(existing.slug).then((r) => r.length > 0) : Promise.resolve(false)),
    [existing?.slug],
  ).data;
  const [slugTouched, setSlugTouched] = useState(Boolean(existing) || Boolean(fill?.values.slug));
  // A new app's address is checked as it's typed, with a free one offered when it's taken.
  const [address, setAddress] = useState<{ slug: string; available: boolean; suggestion: string | null }>();
  useEffect(() => {
    if (existing || !/^[a-z0-9]+(-[a-z0-9]+)*$/.test(form.slug) || form.slug.length < 3) {
      setAddress(undefined);
      return;
    }
    let stale = false;
    const timer = window.setTimeout(() => {
      api.addressAvailable(form.slug).then(
        (a) => !stale && setAddress(a),
        () => !stale && setAddress(undefined),
      );
    }, 350);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [existing, form.slug]);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const terms = useTermsGate();

  const set = <K extends keyof NewApp>(key: K, value: NewApp[K]) => setForm((f) => ({ ...f, [key]: value }));
  // A mod's own page has no mods folder, and neither has a dedicated server (SLAPorSLOP's mods go
  // into the game players play).
  const isMod = existing?.branch?.kind === "addon" || Boolean(existing?.server_game);
  const inBrowser = form.platforms.includes("browser");
  // A browser game is a public game of its own: not an app, not private, not a community build.
  const isPrivate = existing ? existing.private : form.visibility === "private";
  const noBrowser =
    form.kind === "app"
      ? "Only games can play in the browser for now."
      : isPrivate
        ? "A browser game's address is public on the web, so it can't be private."
        : undefined;
  const [hostsText, setHostsText] = useState((existing?.online_hosts ?? fill?.values.online_hosts ?? []).join("\n"));
  const hosts = () => parseHosts(form.online ?? [], hostsText);
  // A private app from a private repository outside GitHub: a read-only token (repo_tokens.rs).
  const [token, setToken] = useState<RepoTokenInput>({ kind: "gitea", token: "" });
  const needsToken = !existing && form.visibility === "private" && /^https?:\/\/[^/]+\/.+/.test(form.repo_url.trim()) && !isGithub(form.repo_url);
  useEffect(() => setToken((t) => (t.token ? t : { ...t, kind: guessKind(form.repo_url) })), [form.repo_url]);
  // Errors for a field the form shows next to that field; any other (an agreement or passkey the
  // user declined, a field this form doesn't have) shows at the bottom, so none goes unseen.
  const inline = new Set<string>([
    "title",
    "slug",
    "summary",
    "description_md",
    "repo_url",
    "license_spdx",
    "min_age",
    "content_labels",
    "online",
    "online_hosts",
    "platforms",
    "tags",
    ...(existing ? [] : ["kind", "stage"]),
    ...(isMod ? [] : ["mods_folder"]),
    ...(needsToken ? ["repo_token"] : []),
  ]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      if (existing) {
        const saved = await api.updateApp(existing.slug, {
          title: form.title,
          summary: form.summary,
          description_md: form.description_md,
          repo_url: form.repo_url,
          license_spdx: form.license_spdx,
          min_age: form.min_age,
          content_labels: form.content_labels,
          tags: form.tags,
          // Only when they change: switching between a download and a browser game is refused
          // once the app has releases, which saving the same choice again mustn't trip over.
          ...(samePlatforms(form.platforms, existing.platforms) ? {} : { platforms: form.platforms }),
          online: form.online,
          online_hosts: hosts(),
          // A browser game has no sandbox mode or mods folder.
          ...(inBrowser ? {} : { sandbox_ready: form.sandbox_ready }),
          accessibility: form.accessibility,
          bug_reports: form.bug_reports,
          ...(isMod || inBrowser ? {} : { mods_folder: form.mods_folder ?? null }),
        });
        navigate(`/app/${saved.slug}`);
        return;
      }
      const saved = await terms.guard(() => {
        const app = { ...form, online_hosts: hosts(), ...(inBrowser ? { sandbox_ready: false, mods_folder: null } : {}) };
        return api.createApp(needsToken && token.token.trim() ? { ...app, repo_token: token } : app);
      });
      const address = playAddress.trim();
      if (!inBrowser || !address) {
        navigate(`/app/${saved.slug}`);
        return;
      }
      // The page is made: its first release is the play address (the commit is left out, so
      // SLAPorSLOP takes the repository's newest on GitHub). If that's refused, the Releases page
      // shows why, with the address filled in to fix and send again.
      const releases = `/publish/${saved.slug}/releases`;
      try {
        await api.submitRelease(saved.slug, { version: "1.0.0", play_address: address });
        navigate(releases);
      } catch (err) {
        const firstAddress: FirstAddress = {
          address,
          message: err instanceof ApiError ? err.message : String(err),
          field: err instanceof ApiError ? err.field : undefined,
        };
        navigate(releases, { state: { firstAddress } });
      }
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };

  const field = (name: keyof NewApp, label: string, input: ReactNode, hint?: ReactNode) => (
    <label>
      {label}
      {input}
      {hint && <span className="hint">{hint}</span>}
      {error?.field === name && (
        <span className="field-error" role="alert">
          {error.message}
        </span>
      )}
    </label>
  );

  const body = (
    <>
      {terms.prompt}
      {linkError && (
        <HandoffError error={linkError} then="The form is empty: fill it in yourself, or ask your assistant for a new link." />
      )}
      {fill && onClearFill && (
        <HandoffNotice
          fill={fill}
          labels={APP_LABELS}
          check={
            filledAddress || filled.platforms?.includes("browser")
              ? "the age rating, content labels, licence, source repository and play address (it decides where Play sends players)"
              : "the age rating, content labels, licence and source repository"
          }
          button="Create draft"
          onClear={onClearFill}
          clearLabel="Clear the form"
        />
      )}
      <form className="maker-form" onSubmit={(e) => void submit(e)}>
        <div className="maker-cols">
          <div className="maker-main">
            <section className="panel panel-pad form" aria-labelledby="basics-title">
              <h2 id="basics-title">Basics</h2>
              {field(
                "title",
                "Name",
                <input
                  value={form.title}
                  maxLength={80}
                  required
                  onChange={(e) => {
                    set("title", e.target.value);
                    if (!slugTouched) set("slug", slugify(e.target.value));
                  }}
                />,
              )}
              {field(
                "slug",
                "Store address",
                <div className="prefixed">
                  <span>slop/app/</span>
                  <input
                    value={form.slug}
                    maxLength={60}
                    required
                    disabled={Boolean(existing)}
                    pattern="[a-z0-9]+(-[a-z0-9]+)*"
                    onChange={(e) => {
                      setSlugTouched(true);
                      set("slug", e.target.value);
                    }}
                  />
                </div>,
                existing ? (
                  "The address can't change once created."
                ) : (
                  <>
                    Lowercase letters, numbers and dashes. It can't change once created.{" "}
                    <span role="status">
                      {address?.slug === form.slug &&
                        (address.available ? (
                          <span className="good">✓ slop/app/{address.slug} is free.</span>
                        ) : (
                          <span className="bad">
                            Taken.{" "}
                            {address.suggestion && (
                              <button
                                type="button"
                                className="link-btn"
                                onClick={() => {
                                  setSlugTouched(true);
                                  set("slug", address.suggestion!);
                                }}
                              >
                                Use {address.suggestion}
                              </button>
                            )}
                          </span>
                        ))}
                    </span>
                  </>
                ),
              )}
              {field(
                "summary",
                "One-line summary",
                <input value={form.summary} maxLength={200} required onChange={(e) => set("summary", e.target.value)} />,
              )}
              {field(
                "description_md",
                "Description",
                <textarea
                  rows={14}
                  value={form.description_md}
                  maxLength={20000}
                  onChange={(e) => set("description_md", e.target.value)}
                />,
                "Markdown. HTML and images aren't allowed.",
              )}

              {!existing && (
                <fieldset className="visibility-choice">
                  <legend>Who can see it?</legend>
                  <label className="check">
                    <input type="radio" name="visibility" checked={form.visibility !== "private"} onChange={() => set("visibility", "public")} />
                    <span>
                      <b>Everyone</b> <span className="muted small">— in the store, from a public open-source repository</span>
                    </span>
                  </label>
                  <label className="check">
                    <input
                      type="radio"
                      name="visibility"
                      checked={form.visibility === "private"}
                      disabled={inBrowser}
                      onChange={() => set("visibility", "private")}
                    />
                    <span>
                      <b>Private</b>{" "}
                      <span className="muted small">
                        — only you and people you share it with. Never in the store; no donations or voting. The repository can be
                        private: on GitHub, install SLAPorSLOP's GitHub App on it; elsewhere, give SLAPorSLOP a read-only token.
                        {inBrowser && " Not for a game that plays in the browser: its address is public on the web."}
                      </span>
                    </span>
                  </label>
                </fieldset>
              )}

              {!existing && (
                <div className="row-2">
                  {field(
                    "kind",
                    "Type",
                    <select
                      value={form.kind}
                      onChange={(e) => {
                        const kind = e.target.value as AppKind;
                        // Only games play in the browser: an app goes back to the download choice.
                        setForm((f) => ({ ...f, kind, platforms: kind === "app" ? chooseDelivery(f.platforms, false) : f.platforms }));
                      }}
                    >
                      <option value="game">Game</option>
                      <option value="app">App</option>
                    </select>,
                  )}
                  {field(
                    "stage",
                    "Stage",
                    <select value={form.stage} onChange={(e) => set("stage", e.target.value as AppStage)}>
                      <option value="coming_soon">{inBrowser ? "Coming soon (not playable yet)" : "Coming soon (no build yet)"}</option>
                      <option value="beta">Beta</option>
                      <option value="released">Released</option>
                    </select>,
                    "You move it on yourself later, from the stage bar at the top of its pages.",
                  )}
                </div>
              )}
            </section>

            <section className="panel panel-pad form" aria-labelledby="source-title">
              <h2 id="source-title">Source and licence</h2>
              {field(
                "repo_url",
                "Source repository",
                <input
                  type="url"
                  value={form.repo_url}
                  placeholder="https://github.com/you/your-app"
                  required
                  readOnly={repoLocked}
                  onChange={(e) => set("repo_url", e.target.value)}
                />,
                repoLocked && existing ? (
                  <>
                    Players already have this app, so moving it is checked by a person first.{" "}
                    <Link to={`/publish/${existing.slug}/manage`}>Request a move from Manage</Link>.
                  </>
                ) : (
                  "Yours: owned by an account you've linked (GitHub, GitLab, Bitbucket), an organization you own, or proved with a code in its README on any other host. Collaborators can't publish someone else's repository."
                ),
              )}
              {needsToken && (
                <fieldset className="tag-picker">
                  <legend>Private repository on another host</legend>
                  <p className="muted small">If the repository is private, SLAPorSLOP needs a read-only token to see it. Public repositories don't.</p>
                  <TokenFields value={token} onChange={setToken} />
                  {error?.field === "repo_token" && (
                    <span className="field-error" role="alert">
                      {error.message}
                    </span>
                  )}
                </fieldset>
              )}
              {(!existing || form.repo_url !== existing.repo_url) && !repoLocked && (
                <RepoOwnership repoUrl={form.repo_url} token={needsToken && token.token.trim() ? token : undefined} />
              )}
              <LicensePicker
                repoUrl={form.repo_url}
                value={form.license_spdx}
                onChange={(spdx) => set("license_spdx", spdx)}
                error={error?.field === "license_spdx" ? error.message : undefined}
              />
            </section>

            <section className="panel panel-pad form" aria-label="Tags">
              <TagPicker
                categories={categories}
                value={form.tags}
                onChange={(t) => set("tags", t)}
                kind={form.kind}
                minAge={form.min_age}
                labels={form.content_labels ?? []}
                error={error?.field === "tags" ? error.message : undefined}
              />
            </section>
          </div>

          <div className="maker-side">
            <section className="panel panel-pad form" aria-labelledby="rating-title">
              <h2 id="rating-title">Age and content</h2>
              {field(
                "min_age",
                "Age rating",
                <AgeSelect value={form.min_age} onChange={(a) => set("min_age", a)} />,
                "18+ apps are hidden in the UK under-18 view.",
              )}
              <ContentLabelPicker
                value={form.content_labels ?? []}
                onChange={(v) => set("content_labels", v)}
                error={error?.field === "content_labels" ? error.message : undefined}
              />
            </section>

            <section className="panel panel-pad form" aria-label="Runs on">
              <PlatformPicker
                value={form.platforms}
                onChange={(p) => set("platforms", p)}
                error={error?.field === "platforms" ? error.message : undefined}
                allowBrowser={!existing?.branch && !existing?.server_game}
                browserUnavailable={noBrowser}
                locked={
                  hasReleases
                    ? "It has releases, so it can't switch between a download and a browser game: for the other, make a new store page."
                    : undefined
                }
              />
              {inBrowser && !existing && (
                <label>
                  Play address{form.stage === "coming_soon" && <span className="muted"> (optional for Coming soon)</span>}
                  <input
                    type="url"
                    value={playAddress}
                    maxLength={2000}
                    required={form.stage !== "coming_soon"}
                    placeholder="https://yourgame.example/"
                    onChange={(e) => setPlayAddress(e.target.value)}
                  />
                  <span className="hint">
                    Where Play sends players: https, on a site of yours. It's sent as version 1.0.0 once the draft is made. A person
                    at SLAPorSLOP checks it before players see it, and again whenever it changes.
                  </span>
                </label>
              )}
              {inBrowser &&
                existing &&
                (playsInBrowser(existing) ? (
                  <p className="small play-address-line">
                    Play address:{" "}
                    {existing.play_address ? (
                      <ExternalLink href={existing.play_address}>{existing.play_address}</ExternalLink>
                    ) : (
                      <span className="muted">none Play opens right now</span>
                    )}{" "}
                    · <Link to={`/publish/${existing.slug}/releases`}>change it on Releases</Link>
                  </p>
                ) : (
                  <p className="small muted">Save, then give its play address on the Releases page.</p>
                ))}
            </section>

            <section className="panel panel-pad form" aria-label={inBrowser ? "Online" : "Online and sandbox mode"}>
              <OnlinePicker
                value={form.online ?? []}
                onChange={(v) => set("online", v)}
                hostsText={hostsText}
                onHostsText={setHostsText}
                browser={inBrowser}
                error={error?.field === "online" || error?.field === "online_hosts" ? error : undefined}
              />
              {/* A website has no sandbox mode: nothing of it runs in the SLAPorSLOP app. */}
              {!inBrowser && <SandboxReadyCheck checked={form.sandbox_ready ?? false} onChange={(v) => set("sandbox_ready", v)} />}
            </section>

            <section className="panel panel-pad form" aria-labelledby="players-title">
              <h2 id="players-title">For players</h2>
              <label className="check">
                <input type="checkbox" checked={form.bug_reports ?? true} onChange={(e) => set("bug_reports", e.target.checked)} />{" "}
                Take bug reports from players on SLAPorSLOP (you see them on the Manage page; turn off if you only use your own issue
                tracker)
              </label>
              <fieldset>
                <legend>Accessibility</legend>
                <p className="hint">Only what it really has: players who play it confirm or dispute each one on its store page.</p>
                {ACCESSIBILITY.map(([key, name]) => (
                  <label className="check" key={key}>
                    <input
                      type="checkbox"
                      checked={(form.accessibility ?? []).includes(key)}
                      onChange={(e) =>
                        set(
                          "accessibility",
                          e.target.checked
                            ? [...(form.accessibility ?? []), key]
                            : (form.accessibility ?? []).filter((k) => k !== key),
                        )
                      }
                    />{" "}
                    {name}
                  </label>
                ))}
              </fieldset>
            </section>

            {!isMod && !inBrowser && (
              <section className="panel panel-pad form" aria-label="Mods">
                <ModsFolderPicker
                  value={form.mods_folder ?? null}
                  onChange={(v) => set("mods_folder", v)}
                  error={error?.field === "mods_folder" ? error.message : undefined}
                />
              </section>
            )}
          </div>
        </div>

        <div className="save-bar">
          <div className="actions">
            <button className="btn slime" disabled={busy}>
              {busy ? "Saving…" : existing ? "Save changes" : "Create draft"}
            </button>
            <Link className="btn ghost" to={existing ? `/app/${existing.slug}` : "/publish"}>
              Cancel
            </Link>
          </div>
          {error && !(error.field && inline.has(error.field)) && (
            <p className="form-error" role="alert">
              {error.message}
            </p>
          )}
          {error && error.field && inline.has(error.field) && (
            <p className="form-error small">Something needs fixing above.</p>
          )}
        </div>
        {!existing && (
          <p className="muted small new-app-pictures">
            Pictures come next: your app's icon, thumbnail, screenshots and videos live in your repository, listed in{" "}
            <code>.slop/media.json</code>. Add them right after creating the draft, on its Edit store page; see{" "}
            <Link to="/publish/guide#pictures-and-videos">Pictures and videos</Link>.
          </p>
        )}
        {!existing && form.visibility !== "private" && (
          <p className="muted small new-app-donations">
            Getting paid: players can donate to you once it's in Beta or released. The draft's Edit store page has a{" "}
            <b>Get paid</b> section to set it up with Stripe (18 or older; SLAPorSLOP's fee is 5%).
          </p>
        )}
      </form>
      {existing?.server_game && (
        <p className="notice">
          A dedicated server for <Link to={`/publish/${encodeURIComponent(existing.server_game.slug)}`}>{existing.server_game.title}</Link>
          : it has no store page of its own, and shows on {existing.server_game.title}'s, with its own releases.
        </p>
      )}
      {existing && !existing.server_game && (
        <div className="maker-cols">
          <div className="maker-main">
            <MediaEditor app={existing} />
          </div>
          <div className="maker-side">
            {!existing.branch && !playsInBrowser(existing) && <ServersPanel game={existing} />}
            {/* Getting paid is part of the page (owner decision 2026-10-25): its state and every
                step to set it up, right here. */}
            {!existing.private && (
              <DonationsPanel
                slug={existing.slug}
                title="Get paid: donations"
                returnTo={`/publish/${existing.slug}#donations`}
                note={
                  existing.stage === "coming_soon"
                    ? "The Donate button shows on your store page from Beta on; you can set it up now."
                    : undefined
                }
              />
            )}
          </div>
        </div>
      )}
    </>
  );

  if (existing) {
    return (
      <MakerFrame app={existing} tab="page">
        {body}
      </MakerFrame>
    );
  }
  return (
    <div className="page maker">
      <div className="crumbs">
        <Link to="/publish">Your apps</Link> › New app
      </div>
      <h1 className="page-title">New app</h1>
      {body}
    </div>
  );
}

const AGES = [0, 3, 7, 12, 16, 18];

/** The age rating; `min` for a branch, which can't be rated below the original. */
export function AgeSelect({ value, onChange, min = 0 }: { value: number; onChange: (a: number) => void; min?: number }) {
  return (
    <select value={value} onChange={(e) => onChange(Number(e.target.value))}>
      {AGES.filter((a) => a >= min).map((a) => (
        <option key={a} value={a}>
          {a === 0 ? "Everyone" : `${a}+`}
        </option>
      ))}
    </select>
  );
}

/** Content labels; `locked` ones (a branch's original's) can't be taken off. */
export function ContentLabelPicker({
  value,
  onChange,
  locked = [],
  error,
}: {
  value: ContentLabel[];
  onChange: (v: ContentLabel[]) => void;
  locked?: ContentLabel[];
  error?: string;
}) {
  return (
    <fieldset className="tag-picker">
      <legend>Content labels</legend>
      <p className="muted small">
        Tick any that apply, so players know what to expect. Sexually explicit content isn't allowed on SLAPorSLOP at all.
        {locked.length > 0 && " The original's labels stay on."}
      </p>
      <div className="chips">
        {(Object.keys(CONTENT_LABEL) as ContentLabel[]).map((l) => {
          const on = value.includes(l);
          const fixed = locked.includes(l);
          return (
            <button
              type="button"
              key={l}
              className={`chip ${on ? "on" : ""}`}
              aria-pressed={on}
              disabled={fixed}
              onClick={() => onChange(on ? value.filter((x) => x !== l) : [...value, l])}
            >
              {CONTENT_LABEL[l]}
            </button>
          );
        })}
      </div>
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
    </fieldset>
  );
}

/** What the app connects to online, and the maker's servers. */
export function OnlinePicker({
  value,
  onChange,
  hostsText,
  onHostsText,
  browser = false,
  error,
}: {
  value: OnlineKind[];
  onChange: (v: OnlineKind[]) => void;
  hostsText: string;
  onHostsText: (t: string) => void;
  /** A browser game: it's on the web anyway, so this is about what else it connects to. */
  browser?: boolean;
  error?: ApiError;
}) {
  return (
    <fieldset className="tag-picker">
      <legend>Online</legend>
      {browser ? (
        <p className="muted small">
          Beyond your own site, tick everything your game connects to: other servers, or other players. Players see this on
          your store page. Leave it empty if it only uses your site.
        </p>
      ) : (
        <p className="muted small">
          Tick everything your app connects to. SLAPorSLOP checks your code but can't check servers, so players see this on your
          store page with a suggestion to use sandbox mode. Leave it empty if your app never goes online (Slop Pulse doesn't
          count). The source check notes networking code an app doesn't declare. See{" "}
          <Link to="/publish/guide#online-play-and-mods">Online play and mods</Link>.
        </p>
      )}
      <div className="chips">
        {(Object.keys(ONLINE_LABEL) as OnlineKind[]).map((k) => {
          const on = value.includes(k);
          return (
            <button
              type="button"
              key={k}
              className={`chip ${on ? "on" : ""}`}
              aria-pressed={on}
              onClick={() => onChange(on ? value.filter((x) => x !== k) : [...value, k])}
            >
              {ONLINE_LABEL[k]}
            </button>
          );
        })}
      </div>
      {error?.field === "online" && (
        <span className="field-error" role="alert">
          {error.message}
        </span>
      )}
      {value.length > 0 && (
        <label>
          Your servers <span className="muted">(optional, up to 10)</span>
          <textarea rows={2} value={hostsText} placeholder="play.example.com" onChange={(e) => onHostsText(e.target.value)} />
          <span className="hint">Server names, like play.example.com or play.example.com:7777, one per line.</span>
          {error?.field === "online_hosts" && (
            <span className="field-error" role="alert">
              {error.message}
            </span>
          )}
        </label>
      )}
    </fieldset>
  );
}

/** Server names as typed, one per line or separated by commas. */
export function parseHosts(online: OnlineKind[], text: string): string[] {
  return online.length === 0 ? [] : text.split(/[\s,]+/).map((h) => h.trim()).filter(Boolean);
}

export function SandboxReadyCheck({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="check">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} /> Works in sandbox mode
      <span className="hint">
        It keeps local saves, settings and logs only in SLOP_SAVE_DIR when that's set, and runs with no access to the rest of
        the computer. SLAPorSLOP's source check looks for it in every release. See{" "}
        <Link to="/publish/guide#saves-and-sandbox-mode">Saves and sandbox mode</Link>.
      </span>
    </label>
  );
}

/** Where the game reads mods from (DESIGN §9). SLAPorSLOP keeps only mods approved on SLAPorSLOP there. */
export function ModsFolderPicker({
  value,
  onChange,
  error,
}: {
  value: ModsFolder | null;
  onChange: (v: ModsFolder | null) => void;
  error?: string;
}) {
  const base = value?.base ?? "none";
  const pick = (b: "none" | "game" | "saves") =>
    onChange(b === "none" ? null : { base: b, path: value?.path ?? "mods", layout: value?.layout ?? "folder" });
  return (
    <fieldset>
      <legend>Mods</legend>
      <p className="muted small">
        If your game loads mods, say where it looks for them, and players' mods from SLAPorSLOP go there: SLAPorSLOP keeps only mods
        approved on SLAPorSLOP in that folder, read-only in sandbox mode. Or read <code>SLOP_MODS</code> instead. See{" "}
        <Link to="/publish/guide#loading-mods-in-your-game">Loading mods in your game</Link>.
      </p>
      {(
        [
          ["none", "It reads SLOP_MODS, or has no mods"],
          ["game", "A folder in the game's files (like mods/ next to the game)"],
          ["saves", "A folder in its save folder"],
        ] as const
      ).map(([b, label]) => (
        <label className="check" key={b}>
          <input type="radio" name="mods-base" checked={base === b} onChange={() => pick(b)} /> {label}
        </label>
      ))}
      {value && (
        <>
          <label>
            Folder
            <input
              value={value.path}
              maxLength={200}
              placeholder="mods"
              onChange={(e) => onChange({ ...value, path: e.target.value })}
            />
            <span className="hint">
              Relative to {value.base === "game" ? "the game's files" : "the save folder"}, with / between parts: mods,
              Content/Paks/~mods.
            </span>
          </label>
          <label>
            Layout
            <select value={value.layout} onChange={(e) => onChange({ ...value, layout: e.target.value as ModsFolder["layout"] })}>
              <option value="folder">Each mod in its own folder</option>
              <option value="flat">All mods' files straight in the folder (for games that read .pak or .zip files there)</option>
            </select>
          </label>
        </>
      )}
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
    </fieldset>
  );
}

/** Whether two platform lists say the same, in any order. */
export function samePlatforms(a: readonly AppPlatform[], b: readonly AppPlatform[]): boolean {
  return a.length === b.length && a.every((p) => b.includes(p));
}

/**
 * The store page's platforms after choosing how players get it: In the browser alone (a browser
 * game plays only there), or a download, which starts from the Windows/Linux builds ticked so far.
 * Exported for tests.
 */
export function chooseDelivery(value: readonly AppPlatform[], browser: boolean): AppPlatform[] {
  return browser ? ["browser"] : value.filter((p) => p !== "browser");
}

/**
 * Windows and/or Linux. Linux builds come as a .tar.gz plus a .deb and/or .rpm package. With
 * `allowBrowser` (a store page of its own, not a community build), first an exclusive choice: a
 * download, or a game that plays in the browser on the maker's website.
 */
export function PlatformPicker({
  value,
  onChange,
  error,
  allowBrowser = false,
  browserUnavailable,
  locked,
}: {
  value: AppPlatform[];
  onChange: (p: AppPlatform[]) => void;
  error?: string;
  allowBrowser?: boolean;
  /** Why In the browser can't be chosen now (an app, or a private page). */
  browserUnavailable?: string;
  /** Why it can't switch between a download and a browser game (it has releases). */
  locked?: string;
}) {
  const has = (p: AppPlatform) => value.includes(p);
  const browser = has("browser");
  const linuxNeedsPackage = has("linux") && !has("linux-deb") && !has("linux-rpm");
  return (
    <fieldset className="platform-picker">
      <legend>Runs on</legend>
      {allowBrowser && (
        <div className="delivery-choice">
          <label className="check">
            <input
              type="radio"
              name="delivery"
              checked={!browser}
              disabled={Boolean(locked) && browser}
              onChange={() => onChange(chooseDelivery(value, false))}
            />
            <span>
              <b>Download</b> <span className="muted small">(Windows and/or Linux)</span>
            </span>
          </label>
          <label className="check">
            <input
              type="radio"
              name="delivery"
              checked={browser}
              disabled={!browser && Boolean(locked || browserUnavailable)}
              onChange={() => onChange(chooseDelivery(value, true))}
            />
            <span>
              <b>In the browser</b> <span className="muted small">(it runs on your own website; nothing to download)</span>
            </span>
          </label>
          {locked ? (
            <p className="hint">{locked}</p>
          ) : (
            !browser && browserUnavailable && <p className="hint">{browserUnavailable}</p>
          )}
        </div>
      )}
      {browser ? (
        <p className="notice small browser-note">
          Players press Play and your website opens in their browser. SLAPorSLOP checks the address, not what the site serves:
          it can't build, sign or scan a website, so your store page never says scanned or built from source. Votes count by the
          days people played. No sandbox mode, save folder or mods. See{" "}
          <Link to="/publish/guide#browser-games">Browser games</Link>.
        </p>
      ) : (
        <PlatformBoxes value={value} onChange={onChange} has={has} linuxNeedsPackage={linuxNeedsPackage} />
      )}
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
    </fieldset>
  );
}

/** The Windows and Linux boxes of a download, with Linux's packages. */
function PlatformBoxes({
  value,
  onChange,
  has,
  linuxNeedsPackage,
}: {
  value: AppPlatform[];
  onChange: (p: AppPlatform[]) => void;
  has: (p: AppPlatform) => boolean;
  linuxNeedsPackage: boolean;
}) {
  const toggle = (p: AppPlatform, on: boolean) => {
    let next = on ? [...value, p] : value.filter((x) => x !== p);
    if (p === "linux" && on && !next.includes("linux-deb") && !next.includes("linux-rpm")) next.push("linux-deb");
    if (p === "linux" && !on) next = next.filter((x) => x !== "linux-deb" && x !== "linux-rpm");
    onChange(next);
  };
  return (
    <>
      <p className="hint">Every release must include a build for each one you tick.</p>
      <label className="check">
        <input type="checkbox" checked={has("windows")} onChange={(e) => toggle("windows", e.target.checked)} />
        <span>
          <b>Windows</b> <span className="muted small">— a .zip of the app's files</span>
        </span>
      </label>
      <label className="check">
        <input type="checkbox" checked={has("linux")} onChange={(e) => toggle("linux", e.target.checked)} />
        <span>
          <b>Linux</b> <span className="muted small">— a .tar.gz of the app's files, which the SLAPorSLOP app installs</span>
        </span>
      </label>
      {has("linux") && (
        <div className="sub-options">
          <span className="small">Plus a package, for players who prefer one (at least one):</span>
          <label className="check">
            <input type="checkbox" checked={has("linux-deb")} onChange={(e) => toggle("linux-deb", e.target.checked)} />
            <span>
              <b>.deb</b> <span className="muted small">— Debian, Ubuntu, Mint</span>
            </span>
          </label>
          <label className="check">
            <input type="checkbox" checked={has("linux-rpm")} onChange={(e) => toggle("linux-rpm", e.target.checked)} />
            <span>
              <b>.rpm</b> <span className="muted small">— Fedora, RHEL, Rocky, Alma</span>
            </span>
          </label>
        </div>
      )}
      {linuxNeedsPackage && <span className="field-error">Pick .deb, .rpm or both.</span>}
    </>
  );
}

/** Whether the repository is yours, and how to prove it if not yet (crates/slop-api/src/repo_owner.rs). */
function RepoOwnership({ repoUrl, token }: { repoUrl: string; token?: RepoTokenInput }) {
  const [result, setResult] = useState<Ownership>();
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string>();
  const url = repoUrl.trim();
  useEffect(() => {
    setResult(undefined);
    setError(undefined);
    if (!/^https?:\/\/[^/]+\/.+/.test(url)) return;
    const timer = window.setTimeout(() => {
      api.repoOwnership(url, token).then(setResult, (err: unknown) => setError(err instanceof ApiError ? err.message : String(err)));
    }, 600);
    return () => window.clearTimeout(timer);
  }, [url, token?.kind, token?.token]);
  const check = async () => {
    setChecking(true);
    setError(undefined);
    try {
      setResult(await api.checkRepoCode(url, token));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setChecking(false);
    }
  };
  if (error) return <p className="form-error small">{error}</p>;
  if (!result) return null;
  if (result.ok) return <p className="small ownership ok">✓ {result.message}</p>;
  return (
    <div className="notice ownership small" role="status">
      <p>{result.message.charAt(0).toUpperCase() + result.message.slice(1)}</p>
      {result.link && (
        <LinkSignIn
          provider={result.link}
          returnTo={window.location.pathname}
          onReturn={() => void api.repoOwnership(url, token).then(setResult, () => undefined)}
        >
          Link {PROVIDER_LABEL[result.link]}
        </LinkSignIn>
      )}
      {result.method === "code" && result.code && (
        <div className="row-actions">
          <code className="ownership-code">{result.code}</code>
          <button type="button" className="btn small ghost" onClick={() => void navigator.clipboard?.writeText(result.code!)}>
            Copy
          </button>
          <button type="button" className="btn small" disabled={checking} onClick={() => void check()}>
            {checking ? "Checking…" : "Check"}
          </button>
        </div>
      )}
    </div>
  );
}

/**
 * "Dedicated server (optional)" on a game's Edit store page (docs/DEDICATED-SERVERS.md §6): the
 * programs players run on their own computer or a host so others can play, each its own app from
 * its own repository, shown on the game's store page.
 */
export function ServersPanel({ game }: { game: AppDetail }) {
  const navigate = useNavigate();
  const terms = useTermsGate();
  const servers = game.servers ?? [];
  const taken = new Set([game.slug, ...servers.map((s) => s.slug)]);
  let freeSlug = `${game.slug}-server`;
  for (let n = 2; taken.has(freeSlug); n++) freeSlug = `${game.slug}-server-${n}`;
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState({
    title: `${game.title} Dedicated server`.slice(0, 80),
    slug: freeSlug,
    repo_url: "",
    license_spdx: game.license_spdx,
    summary: `Run your own ${game.title} server, so others can play.`.slice(0, 200),
    platforms: (game.platforms.filter((p) => p !== "browser" && p !== "any") as AppPlatform[]) || [],
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError>();
  const set = <K extends keyof typeof form>(key: K, value: (typeof form)[K]) => setForm((f) => ({ ...f, [key]: value }));
  const fieldError = (field: string) => (error?.field === field ? error.message : undefined);

  const add = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const res = await terms.guard(() => api.addServer(game.slug, form));
      navigate(`/publish/${encodeURIComponent(res.slug)}/releases`);
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "error", String(err)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel panel-pad form servers-panel" id="servers" aria-labelledby="servers-panel-title">
      <h2 id="servers-panel-title">Dedicated server (optional)</h2>
      <p className="muted small">
        A program players run on their own computer or a host so others can play your game. It lives in its own repository and is
        built, checked and reviewed like any release. Not for servers only you run: for those, pick{" "}
        <b>{ONLINE_LABEL.own_servers}</b> under Online instead.
      </p>
      {servers.length > 0 && (
        <ul className="server-list">
          {servers.map((s) => (
            <li key={s.slug} className="server-row">
              <div className="server-head">
                <b>{s.title}</b> {s.status === "draft" && <span className="badge neutral">DRAFT</span>}
              </div>
              <p className="small">
                <Link to={`/publish/${encodeURIComponent(s.slug)}/releases`}>Releases &amp; progress</Link> ·{" "}
                <Link to={`/publish/${encodeURIComponent(s.slug)}`}>Edit</Link>
              </p>
            </li>
          ))}
        </ul>
      )}
      {!open ? (
        <button type="button" className="btn" onClick={() => setOpen(true)}>
          {servers.length > 0 ? "Add another dedicated server" : "Add a dedicated server"}
        </button>
      ) : (
        <form className="form" onSubmit={(e) => void add(e)} aria-label="Add a dedicated server">
          <label>
            Its repository
            <input
              type="url"
              required
              value={form.repo_url}
              placeholder="https://github.com/you/your-game-server"
              onChange={(e) => set("repo_url", e.target.value)}
              aria-invalid={Boolean(fieldError("repo_url"))}
            />
            <span className="muted small">Its own, not the game's: a repository with both has to be split first.</span>
          </label>
          {fieldError("repo_url") && <p className="form-error">{fieldError("repo_url")}</p>}
          <label>
            Name
            <input required maxLength={80} value={form.title} onChange={(e) => set("title", e.target.value)} />
          </label>
          {fieldError("title") && <p className="form-error">{fieldError("title")}</p>}
          <label>
            Address
            <input required value={form.slug} onChange={(e) => set("slug", e.target.value)} aria-invalid={Boolean(fieldError("slug"))} />
            <span className="muted small">It shows {game.title}'s store page.</span>
          </label>
          {fieldError("slug") && <p className="form-error">{fieldError("slug")}</p>}
          <label>
            One line for players
            <input required maxLength={200} value={form.summary} onChange={(e) => set("summary", e.target.value)} />
          </label>
          {fieldError("summary") && <p className="form-error">{fieldError("summary")}</p>}
          <LicensePicker
            repoUrl={form.repo_url}
            value={form.license_spdx}
            onChange={(spdx) => set("license_spdx", spdx)}
            error={fieldError("license_spdx")}
          />
          <PlatformPicker value={form.platforms} onChange={(p) => set("platforms", p)} error={fieldError("platforms")} />
          {error && !error.field && (
            <p className="form-error" role="alert">
              {error.message}
            </p>
          )}
          {error?.field && (
            <p className="form-error" role="alert">
              Check the fields above.
            </p>
          )}
          <div className="actions">
            <button type="button" className="btn ghost" onClick={() => setOpen(false)}>
              Cancel
            </button>
            <button className="btn slime" disabled={busy}>
              {busy ? "Adding…" : "Add the server"}
            </button>
          </div>
        </form>
      )}
      {terms.prompt}
    </section>
  );
}
