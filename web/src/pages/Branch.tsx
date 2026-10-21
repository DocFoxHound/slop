// Registering a branch of an app (DESIGN §9): a mod the game loads, or a modded version, fix,
// translation or port built from your own fork. It starts as a draft; you submit a release, Slop
// checks and reviews it like any other, and it then shows in the Community area of the original's
// page. /app/:slug/branch?kind=addon starts on "Mod".

import { useState, type FormEvent } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";

import { api, ApiError, type AppPlatform } from "../api";
import { LicensePicker } from "../components/LicensePicker";
import { useTermsGate } from "../components/Terms";
import { ErrorState, Loading } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { PlatformPicker } from "./Publish";

const KINDS = [
  ["addon", "Mod", "Files the game or app loads: new levels, items, rules or scripts. Players add it to the original"],
  ["mod", "Modded version", "A whole changed copy of the game or app, played instead of the original"],
  ["port", "Port", "Runs on a platform the original doesn't"],
  ["fix", "Fix", "Fixes bugs the original still has"],
  ["translation", "Translation", "Another language"],
  ["other", "Other", ""],
] as const;

export function RegisterBranch() {
  const { slug = "" } = useParams();
  const { me } = useSession();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const parent = useLoad(() => api.app(slug), [slug]);
  useDocumentTitle(parent.data ? `Branch ${parent.data.title}` : "Register a branch");
  const terms = useTermsGate();
  const [form, setForm] = useState({
    slug: "",
    title: "",
    summary: "",
    repo_url: "",
    license_spdx: "",
    kind: params.get("kind") === "addon" ? "addon" : "port",
    note: "",
    platforms: [] as AppPlatform[],
  });
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  if (parent.error) return <ErrorState error={parent.error} onRetry={parent.reload} />;
  if (!parent.data || me === undefined) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <p>
          <Link to={`/signin?return_to=/app/${slug}/branch`}>Sign in</Link> to register a branch.
        </p>
      </div>
    );
  }
  const p = parent.data;
  const set = <K extends keyof typeof form>(k: K, v: (typeof form)[K]) => setForm((f) => ({ ...f, [k]: v }));
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const res = await terms.guard(() => api.registerBranch(slug, form));
      navigate(`/publish/${res.slug}/releases`);
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };
  const fieldError = (field: string) =>
    error?.field === field ? (
      <span className="field-error" role="alert">
        {error.message}
      </span>
    ) : null;
  const missing = (["windows", "linux"] as const).filter((x) => !p.platforms.includes(x));
  const isMod = form.kind === "addon";
  return (
    <div className="page narrow">
      {terms.prompt}
      <div className="crumbs">
        <Link to={`/app/${slug}`}>{p.title}</Link> › Register a branch
      </div>
      <h1 className="page-title">{isMod ? `Publish a mod for ${p.title}` : `Register a branch of ${p.title}`}</h1>
      {isMod ? (
        <p className="muted">
          A mod lives in <b>your own repository</b>. Each release is one <code>.zip</code> of the mod's files for every
          platform, with nothing to start: the Slop app adds it to {p.title}, which loads it. It's checked and reviewed like
          any app, then shows in the Community area of {p.title}'s page, made by you, not {p.maker.display_name}. Games on
          Slop only load mods approved here; see{" "}
          <Link to="/publish/guide#online-play-and-mods">Online play and mods</Link>.
        </p>
      ) : (
      <p className="muted">
        A branch is built from <b>your own fork</b> of {p.title}. It's checked and reviewed like any app, then shows on{" "}
        {p.title}'s page as a <b>Community</b> build, made by you, not {p.maker.display_name}. Players tick "I understand"
        before installing it.
        {missing.length > 0 && ` ${p.title} doesn't run on ${missing.join(" or ")} yet: a port fills that gap.`}
      </p>
      )}
      <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
        <fieldset>
          <legend>What is it?</legend>
          {KINDS.map(([k, label, hint]) => (
            <label className="check" key={k}>
              <input type="radio" name="kind" checked={form.kind === k} onChange={() => set("kind", k)} />
              <span>
                <b>{label}</b> {hint && <span className="muted small">— {hint}</span>}
              </span>
            </label>
          ))}
        </fieldset>
        <label>
          Name
          <input value={form.title} maxLength={80} required onChange={(e) => set("title", e.target.value)} placeholder={`${p.title} (Linux port)`} />
          {fieldError("title")}
        </label>
        <label>
          Address
          <input
            value={form.slug}
            maxLength={40}
            required
            onChange={(e) => set("slug", e.target.value.toLowerCase())}
            placeholder={`${slug}-linux`}
          />
          <span className="hint">slop.example/app/{form.slug || "…"}</span>
          {fieldError("slug")}
        </label>
        <label>
          One-line summary
          <input value={form.summary} maxLength={200} required onChange={(e) => set("summary", e.target.value)} />
          {fieldError("summary")}
        </label>
        <label>
          What it changes
          <textarea value={form.note} maxLength={500} rows={3} onChange={(e) => set("note", e.target.value)} />
          <span className="hint">Shown to players next to the official version.</span>
          {fieldError("note")}
        </label>
        <label>
          {isMod ? "Your mod's repository" : "Your fork"}
          <input
            value={form.repo_url}
            required
            onChange={(e) => set("repo_url", e.target.value)}
            placeholder={isMod ? "https://github.com/you/your-mod" : "https://github.com/you/your-fork"}
          />
          {fieldError("repo_url")}
        </label>
        <LicensePicker
          repoUrl={form.repo_url}
          value={form.license_spdx}
          onChange={(l) => set("license_spdx", l)}
          error={error?.field === "license_spdx" ? error.message : undefined}
        />
        {!isMod && (
          <PlatformPicker
            value={form.platforms}
            onChange={(v) => set("platforms", v)}
            error={error?.field === "platforms" ? error.message : undefined}
          />
        )}
        {error && !error.field && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <button className="btn slime" disabled={busy}>
          {busy ? "Registering…" : isMod ? "Publish the mod" : "Register the branch"}
        </button>
      </form>
    </div>
  );
}
