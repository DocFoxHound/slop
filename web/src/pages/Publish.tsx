// Makers' side: their apps, and the draft editor. Putting a draft in the store needs the
// publishing pipeline (repo link, Slop Build, scans), which comes next.

import { useState, type FormEvent, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { api, ApiError, type AppDetail, type AppKind, type AppStage, type Category, type NewApp } from "../api";
import { AppTile, ErrorState, Loading } from "../components/ui";
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
        <p>
          <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(what)}`}>
            Sign in to start
          </Link>
        </p>
      </div>
    </div>
  );
}

export function Publish() {
  useDocumentTitle("Publish");
  const { me } = useSession();
  const mine = useLoad(() => (me ? api.myApps() : Promise.resolve([])), [me?.id]);

  if (me === undefined) return <Loading />;
  if (!me) return <SignInFirst what="/publish" />;

  return (
    <div className="page">
      <div className="title-row spread">
        <h1 className="page-title">Your apps</h1>
        <Link className="btn slime" to="/publish/new">
          + New app
        </Link>
      </div>
      <p className="muted">
        Start with a draft store page. Linking your repo, Slop Build and scanning come next; once a draft passes, it goes in
        the store.
      </p>
      {mine.error && <ErrorState error={mine.error} onRetry={mine.reload} />}
      {mine.data && mine.data.length === 0 && (
        <div className="empty-state">
          You haven't made anything yet. <Link to="/publish/new">Create your first draft</Link>.
        </div>
      )}
      <div className="shelf">
        {mine.data?.map((a) => (
          <AppTile app={a} key={a.slug} />
        ))}
      </div>
    </div>
  );
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
        }
      : EMPTY,
  );
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
          })
        : await api.createApp(form);
      navigate(`/app/${saved.slug}`);
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };

  const field = (name: keyof NewApp, label: string, input: ReactNode, hint?: string) => (
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
            onChange={(e) => set("repo_url", e.target.value)}
          />,
          "Public, on GitHub, GitLab, Bitbucket or your own server.",
        )}
        <div className="row-2">
          {field(
            "license_spdx",
            "Licence",
            <input
              value={form.license_spdx}
              placeholder="MIT"
              maxLength={64}
              required
              onChange={(e) => set("license_spdx", e.target.value)}
            />,
            "An OSI-approved licence's SPDX ID.",
          )}
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
      </form>
    </div>
  );
}
