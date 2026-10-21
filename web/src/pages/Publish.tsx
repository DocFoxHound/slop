// SPDX-License-Identifier: AGPL-3.0-or-later
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
  type ContentLabel,
  type ModsFolder,
  type NewApp,
  type OnlineKind,
  type RepoTokenInput,
  PROVIDER_LABEL,
  type Release,
  type RepoOwnership as Ownership,
} from "../api";
import { LicensePicker } from "../components/LicensePicker";
import { guessKind, isGithub, TokenFields } from "../components/RepoToken";
import { MediaEditor } from "../components/MediaEditor";
import { currentStep, progressSteps, STEP_ICON, STEP_LABEL, type Step } from "../components/PublishProgress";
import { ErrorState, Loading } from "../components/ui";
import { CONTENT_LABEL, ONLINE_LABEL } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { PasskeyNotice } from "../components/PasskeyNotice";
import { useTermsGate } from "../components/Terms";

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
      <PasskeyNotice />
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
  content_labels: [],
  tags: [],
  platforms: [],
  online: [],
  online_hosts: [],
  sandbox_ready: false,
  mods_folder: null,
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
          content_labels: existing.content_labels,
          tags: existing.tags,
          platforms: existing.platforms,
          online: existing.online,
          online_hosts: existing.online_hosts,
          sandbox_ready: existing.sandbox_ready,
          mods_folder: existing.mods_folder,
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
  const terms = useTermsGate();

  const set = <K extends keyof NewApp>(key: K, value: NewApp[K]) => setForm((f) => ({ ...f, [key]: value }));
  // A mod's own page has no mods folder.
  const isMod = existing?.branch?.kind === "addon";
  const [hostsText, setHostsText] = useState((existing?.online_hosts ?? []).join("\n"));
  const hosts = () => parseHosts(form.online ?? [], hostsText);
  // Private Slop from a private repository outside GitHub: a read-only token (repo_tokens.rs).
  const [token, setToken] = useState<RepoTokenInput>({ kind: "gitea", token: "" });
  const needsToken = !existing && form.visibility === "private" && /^https?:\/\/[^/]+\/.+/.test(form.repo_url.trim()) && !isGithub(form.repo_url);
  useEffect(() => setToken((t) => (t.token ? t : { ...t, kind: guessKind(form.repo_url) })), [form.repo_url]);
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
            content_labels: form.content_labels,
            tags: form.tags,
            platforms: form.platforms,
            online: form.online,
            online_hosts: hosts(),
            sandbox_ready: form.sandbox_ready,
            ...(isMod ? {} : { mods_folder: form.mods_folder ?? null }),
          })
        : await terms.guard(() => {
            const app = { ...form, online_hosts: hosts() };
            return api.createApp(needsToken && token.token.trim() ? { ...app, repo_token: token } : app);
          });
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
      {terms.prompt}
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
          <fieldset className="visibility-choice">
            <legend>Who can see it?</legend>
            <label className="check">
              <input type="radio" name="visibility" checked={form.visibility !== "private"} onChange={() => set("visibility", "public")} />
              <span>
                <b>Everyone</b> <span className="muted small">— in the store, from a public open-source repository</span>
              </span>
            </label>
            <label className="check">
              <input type="radio" name="visibility" checked={form.visibility === "private"} onChange={() => set("visibility", "private")} />
              <span>
                <b>Private Slop</b>{" "}
                <span className="muted small">
                  — only you and people you share it with. Never in the store; no donations or voting. The repository can be
                  private: on GitHub, install Slop's GitHub App on it; elsewhere, give Slop a read-only token.
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
            "Yours: owned by an account you've linked (GitHub, GitLab, Bitbucket), an organization you own, or proved with a code in its README on any other host. Collaborators can't publish someone else's repository."
          ),
        )}
        {needsToken && (
          <fieldset className="tag-picker">
            <legend>Private repository on another host</legend>
            <p className="muted small">If the repository is private, Slop needs a read-only token to see it. Public repositories don't.</p>
            <TokenFields value={token} onChange={setToken} />
          </fieldset>
        )}
        {(!existing || form.repo_url !== existing.repo_url) && !repoLocked && (
          <RepoOwnership repoUrl={form.repo_url} token={needsToken && token.token.trim() ? token : undefined} />
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
            <AgeSelect value={form.min_age} onChange={(a) => set("min_age", a)} />,
            "18+ apps are hidden in the UK under-18 view.",
          )}
        </div>

        <ContentLabelPicker
          value={form.content_labels ?? []}
          onChange={(v) => set("content_labels", v)}
          error={error?.field === "content_labels" ? error.message : undefined}
        />
        <OnlinePicker
          value={form.online ?? []}
          onChange={(v) => set("online", v)}
          hostsText={hostsText}
          onHostsText={setHostsText}
          error={error?.field === "online" || error?.field === "online_hosts" ? error : undefined}
        />
        <SandboxReadyCheck checked={form.sandbox_ready ?? false} onChange={(v) => set("sandbox_ready", v)} />
        {!isMod && (
          <ModsFolderPicker
            value={form.mods_folder ?? null}
            onChange={(v) => set("mods_folder", v)}
            error={error?.field === "mods_folder" ? error.message : undefined}
          />
        )}

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
        Tick any that apply, so players know what to expect. Sexually explicit content isn't allowed on Slop at all.
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
  error,
}: {
  value: OnlineKind[];
  onChange: (v: OnlineKind[]) => void;
  hostsText: string;
  onHostsText: (t: string) => void;
  error?: ApiError;
}) {
  return (
    <fieldset className="tag-picker">
      <legend>Online</legend>
      <p className="muted small">
        Tick everything your app connects to. Slop checks your code but can't check servers, so players see this on your
        store page with a suggestion to use sandbox mode. Leave it empty if your app never goes online (Slop Pulse doesn't
        count). The source check notes networking code an app doesn't declare. See{" "}
        <Link to="/publish/guide#online-play-and-mods">Online play and mods</Link>.
      </p>
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
        the computer. Slop's source check looks for it in every release. See{" "}
        <Link to="/publish/guide#saves-and-sandbox-mode">Saves and sandbox mode</Link>.
      </span>
    </label>
  );
}

/** Where the game reads mods from (DESIGN §9). Slop keeps only mods approved on Slop there. */
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
        If your game loads mods, say where it looks for them, and players' mods from Slop go there: Slop keeps only mods
        approved on Slop in that folder, read-only in sandbox mode. Or read <code>SLOP_MODS</code> instead. See{" "}
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

/** Windows and/or Linux. Linux builds come as a .tar.gz plus a .deb and/or .rpm package. */
export function PlatformPicker({
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
        <a className="btn small" href={api.linkUrl(result.link, window.location.pathname)}>
          Link {PROVIDER_LABEL[result.link]}
        </a>
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
