// Makers' side: their apps with where each one is on its way into the store, and the draft editor.

import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "react-router";

import {
  api,
  ApiError,
  type AppDetail,
  type AppKind,
  type AppPlatform,
  type AppStage,
  type AppSummary,
  type Category,
  type NewApp,
  type Release,
} from "../api";
import { LicensePicker } from "../components/LicensePicker";
import { MediaEditor } from "../components/MediaEditor";
import { currentStep, progressSteps, STEP_ICON, STEP_LABEL, type Step } from "../components/PublishProgress";
import { ErrorState, Loading } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";

function SignInFirst({ what }: { what: string }) {
  return (
    <div className="page narrow">
      <div className="panel panel-pad">
        <h1>Publish on Slop</h1>
        <p>
          Any open-source app or game made with AI can go on Slop. Listing is free; Slop's only fee is 5% of donations,
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
      </div>
    </div>
  );
}

export function Publish() {
  useDocumentTitle("Publish");
  const { me } = useSession();
  const mine = useLoad(() => (me ? loadStatuses() : Promise.resolve([])), [me?.id]);

  // While something is moving (files being fetched, a review pending), check again now and then.
  const moving = mine.data?.some((a) => a.current.state === "active") ?? false;
  useEffect(() => {
    if (!moving) return;
    const timer = setInterval(mine.reload, 20_000);
    return () => clearInterval(timer);
  }, [moving, mine.reload]);

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
      </div>
      <p className="muted">
        Each app goes through the same steps: a store page, a release, file checks, a review by a person, then the store.
        This is where each one is.
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
                  <Link to={`/app/${app.slug}`}>
                    <b>{app.title}</b>
                  </Link>
                  <span className="badge neutral">{STAGE_LABEL[app.stage]}</span>
                  {latest && <span className="muted small">v{latest.version}</span>}
                </div>
                <div className="small">
                  {statusHeadline(current)} <span className="step-state">{STEP_LABEL[current.state]}</span>
                </div>
                <div className="muted small">{current.detail}</div>
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

function statusHeadline(step: Step): string {
  if (step.state === "problem") return `Stuck at: ${step.title}`;
  if (step.state === "done") return "In the store";
  if (step.title === "Release") return "Next: submit a release";
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
  tags: [],
  platforms: [],
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

  if (me === undefined) return <Loading />;
  if (!me) return <SignInFirst what={slug ? `/publish/${slug}` : "/publish/new"} />;
  if (existing.error) return <div className="page"><ErrorState error={existing.error} /></div>;
  if (slug && !existing.data) return <Loading />;
  if (existing.data && !existing.data.is_mine) return <div className="page"><ErrorState error={new ApiError(404, "not_found", "")} /></div>;
  if (!categories.data) return <Loading />;

  return <EditorForm key={slug ?? "new"} existing={existing.data} categories={categories.data} />;
}

function EditorForm({ existing, categories }: { existing?: AppDetail; categories: Category[] }) {
  const navigate = useNavigate();
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
          tags: existing.tags,
          platforms: existing.platforms,
        }
      : EMPTY,
  );
  // Once players have the app, moving it to another repository is reviewed first (Manage).
  const repoLocked = useLoad(
    () => (existing ? api.manage(existing.slug).then((m) => m.repo_locked) : Promise.resolve(false)),
    [existing?.slug],
  ).data;
  const [slugTouched, setSlugTouched] = useState(Boolean(existing));
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);

  const set = <K extends keyof NewApp>(key: K, value: NewApp[K]) => setForm((f) => ({ ...f, [key]: value }));
  const toggleTag = (tag: string) =>
    set("tags", form.tags.includes(tag) ? form.tags.filter((t) => t !== tag) : [...form.tags, tag]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const saved = existing
        ? await api.updateApp(existing.slug, {
            title: form.title,
            summary: form.summary,
            description_md: form.description_md,
            repo_url: form.repo_url,
            license_spdx: form.license_spdx,
            min_age: form.min_age,
            tags: form.tags,
            platforms: form.platforms,
          })
        : await api.createApp(form);
      navigate(`/app/${saved.slug}`);
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

  return (
    <div className="page narrow">
      <div className="crumbs">
        <Link to="/publish">Your apps</Link> › {existing ? existing.title : "New app"}
      </div>
      <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
        <h1>{existing ? "Edit store page" : "New app"}</h1>

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
          existing ? "The address can't change once created." : "Lowercase letters, numbers and dashes.",
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
            rows={8}
            value={form.description_md}
            maxLength={20000}
            onChange={(e) => set("description_md", e.target.value)}
          />,
          "Markdown. HTML and images aren't allowed.",
        )}

        {!existing && (
          <div className="row-2">
            {field(
              "kind",
              "Type",
              <select value={form.kind} onChange={(e) => set("kind", e.target.value as AppKind)}>
                <option value="game">Game</option>
                <option value="app">App</option>
              </select>,
            )}
            {field(
              "stage",
              "Stage",
              <select value={form.stage} onChange={(e) => set("stage", e.target.value as AppStage)}>
                <option value="coming_soon">Coming soon (no build yet)</option>
                <option value="beta">Beta</option>
                <option value="released">Released</option>
              </select>,
            )}
          </div>
        )}

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
            "Public, on GitHub, GitLab, Bitbucket or your own server."
          ),
        )}
        <div className="row-2">
          <LicensePicker
            repoUrl={form.repo_url}
            value={form.license_spdx}
            onChange={(spdx) => set("license_spdx", spdx)}
            error={error?.field === "license_spdx" ? error.message : undefined}
          />
          {field(
            "min_age",
            "Age rating",
            <select value={form.min_age} onChange={(e) => set("min_age", Number(e.target.value))}>
              {[0, 3, 7, 12, 16, 18].map((a) => (
                <option key={a} value={a}>
                  {a === 0 ? "Everyone" : `${a}+`}
                </option>
              ))}
            </select>,
            "18+ apps are hidden in the UK under-18 view.",
          )}
        </div>

        <PlatformPicker value={form.platforms} onChange={(p) => set("platforms", p)} error={error?.field === "platforms" ? error.message : undefined} />

        <fieldset className="tag-picker">
          <legend>
            Tags <span className="muted">({form.tags.length}/10)</span>
          </legend>
          {categories.map((c) => (
            <div key={c.slug}>
              <h4>{c.name}</h4>
              <div className="chips">
                {c.tags.map((t) => {
                  const on = form.tags.includes(t.slug);
                  return (
                    <button
                      type="button"
                      key={t.slug}
                      className={`chip ${on ? "on" : ""}`}
                      aria-pressed={on}
                      disabled={!on && form.tags.length >= 10}
                      onClick={() => toggleTag(t.slug)}
                    >
                      {t.name}
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
          {error?.field === "tags" && (
            <span className="field-error" role="alert">
              {error.message}
            </span>
          )}
        </fieldset>

        {error && !error.field && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <div className="actions">
          <Link className="btn ghost" to={existing ? `/app/${existing.slug}` : "/publish"}>
            Cancel
          </Link>
          <button className="btn slime" disabled={busy}>
            {busy ? "Saving…" : existing ? "Save changes" : "Create draft"}
          </button>
        </div>
        {!existing && <p className="muted small">You can add a thumbnail, screenshots and videos once the draft is created.</p>}
      </form>
      {existing && <MediaEditor app={existing} />}
    </div>
  );
}

/** Windows and/or Linux. Linux builds come as a .tar.gz plus a .deb and/or .rpm package. */
function PlatformPicker({
  value,
  onChange,
  error,
}: {
  value: AppPlatform[];
  onChange: (p: AppPlatform[]) => void;
  error?: string;
}) {
  const has = (p: AppPlatform) => value.includes(p);
  const toggle = (p: AppPlatform, on: boolean) => {
    let next = on ? [...value, p] : value.filter((x) => x !== p);
    if (p === "linux" && on && !next.includes("linux-deb") && !next.includes("linux-rpm")) next.push("linux-deb");
    if (p === "linux" && !on) next = next.filter((x) => x !== "linux-deb" && x !== "linux-rpm");
    onChange(next);
  };
  const linuxNeedsPackage = has("linux") && !has("linux-deb") && !has("linux-rpm");
  return (
    <fieldset className="platform-picker">
      <legend>Runs on</legend>
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
          <b>Linux</b> <span className="muted small">— a .tar.gz of the app's files, which the Slop app installs</span>
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
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
    </fieldset>
  );
}
