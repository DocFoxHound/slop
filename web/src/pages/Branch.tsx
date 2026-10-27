// SPDX-License-Identifier: AGPL-3.0-or-later
// Registering a branch of an app (DESIGN §9): a mod the game loads, or a modded version, fix,
// translation or port built from your own fork. It's made for one original: the page starts from
// the original's rating, labels and online answers, and asks everything creating an app asks, so
// branches meet the same rules. It starts as a draft; you submit a release, SLAPorSLOP checks and
// reviews it like any other, and it then shows in the Community area of the original's page.
// /app/:slug/branch?kind=addon starts on "Mod".

import { useState, type FormEvent } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";

import { api, ApiError, type AppDetail, type AppPlatform, type ContentLabel, type OnlineKind } from "../api";
import { LicensePicker } from "../components/LicensePicker";
import { useTermsGate } from "../components/Terms";
import { Art, ErrorState, Loading } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { AgeSelect, ContentLabelPicker, OnlinePicker, parseHosts, PlatformPicker, SandboxReadyCheck } from "./Publish";

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
  const parent = useLoad(() => api.app(slug), [slug]);
  useDocumentTitle(parent.data ? `Branch ${parent.data.title}` : "Register a branch");
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
  return <BranchForm parent={parent.data} />;
}

function BranchForm({ parent: p }: { parent: AppDetail }) {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const terms = useTermsGate();
  // The original's answers to start from: a branch carries its content and usually its servers.
  const [form, setForm] = useState({
    slug: "",
    title: "",
    summary: "",
    repo_url: "",
    license_spdx: "",
    kind: params.get("kind") === "addon" ? "addon" : "port",
    note: "",
    description_md: "",
    platforms: [] as AppPlatform[],
    min_age: p.min_age,
    content_labels: p.content_labels as ContentLabel[],
    online: p.online as OnlineKind[],
    sandbox_ready: false,
  });
  const [hostsText, setHostsText] = useState(p.online_hosts.join("\n"));
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const set = <K extends keyof typeof form>(k: K, v: (typeof form)[K]) => setForm((f) => ({ ...f, [k]: v }));
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const res = await terms.guard(() =>
        api.registerBranch(p.slug, { ...form, online_hosts: parseHosts(form.online, hostsText) }),
      );
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
  // Fields with their own error line; any other error (an agreement declined) shows at
  // the bottom.
  const shownByField = new Set<string>([
    "title",
    "slug",
    "summary",
    "note",
    "description_md",
    "repo_url",
    "license_spdx",
    "min_age",
    "content_labels",
    "online",
    "online_hosts",
    ...(form.kind === "addon" ? [] : ["platforms"]),
  ]);
  const missing = (["windows", "linux"] as const).filter((x) => !p.platforms.includes(x));
  const isMod = form.kind === "addon";
  const thing = p.kind === "game" ? "game" : "app";
  return (
    <div className="page narrow">
      {terms.prompt}
      <div className="crumbs">
        <Link to={`/app/${p.slug}`}>{p.title}</Link> › {isMod ? "Publish a mod" : "Register a branch"}
      </div>
      <div className="branch-target panel panel-pad">
        <Link to={`/app/${p.slug}`} className="library-art" aria-label={p.title}>
          <Art app={p} withName={false} />
        </Link>
        <div>
          <h1 className="page-title">{isMod ? `Publish a mod for ${p.title}` : `Register a branch of ${p.title}`}</h1>
          <p className="muted small">
            {p.title} by {p.maker.display_name} · rated {p.min_age === 0 ? "Everyone" : `${p.min_age}+`} · runs on{" "}
            {p.platforms.filter((x) => x === "windows" || x === "linux").join(" and ") || "—"}
          </p>
        </div>
      </div>
      {isMod ? (
        <p className="muted">
          A mod lives in <b>your own repository</b>. Each release is one <code>.zip</code> of the mod's files for every
          platform, with nothing to start: the SLAPorSLOP app adds it to {p.title}, which loads it. It shows in the Community area
          of {p.title}'s page, made by you, not {p.maker.display_name}. Games on SLAPorSLOP only load mods approved here; see{" "}
          <Link to="/publish/guide#online-play-and-mods">Online play and mods</Link>.
        </p>
      ) : (
        <p className="muted">
          A branch is built from <b>your own fork</b> of {p.title}. It shows on {p.title}'s page as a <b>Community</b> build,
          made by you, not {p.maker.display_name}, and players can pick it to play instead of the original.
          {missing.length > 0 && ` ${p.title} doesn't run on ${missing.join(" or ")} yet: a port fills that gap.`}
        </p>
      )}
      <div className="notice small" role="note">
        <b>The same rules as a new app.</b> Every release is checked like any other: the files are scanned, the source is
        read (licence, anything for sale, code from outside SLAPorSLOP, networking, where saves go), and a person reviews it
        before players see it. It can't be rated below {p.title}, and keeps its content labels.
      </div>
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
          <input
            value={form.title}
            maxLength={80}
            required
            onChange={(e) => set("title", e.target.value)}
            placeholder={isMod ? `${p.title}: More dragons` : `${p.title} (Linux port)`}
          />
          {fieldError("title")}
        </label>
        <label>
          Address
          <input
            value={form.slug}
            maxLength={40}
            required
            onChange={(e) => set("slug", e.target.value.toLowerCase())}
            placeholder={isMod ? `${p.slug}-dragons` : `${p.slug}-linux`}
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
          What it changes from {p.title}
          <textarea value={form.note} maxLength={500} rows={3} required onChange={(e) => set("note", e.target.value)} />
          <span className="hint">Shown to players next to the official version.</span>
          {fieldError("note")}
        </label>
        <label>
          Description <span className="muted">(optional)</span>
          <textarea
            value={form.description_md}
            maxLength={20000}
            rows={5}
            onChange={(e) => set("description_md", e.target.value)}
          />
          <span className="hint">Its own store page. Markdown. Left empty, the note above is used.</span>
          {fieldError("description_md")}
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
        <label>
          Age rating
          <AgeSelect value={form.min_age} min={p.min_age} onChange={(a) => set("min_age", a)} />
          <span className="hint">
            At least {p.title}'s. Rate it higher if your {isMod ? "mod" : "branch"} adds stronger content.
          </span>
          {fieldError("min_age")}
        </label>
        <ContentLabelPicker
          value={form.content_labels}
          locked={p.content_labels as ContentLabel[]}
          onChange={(v) => set("content_labels", v)}
          error={error?.field === "content_labels" ? error.message : undefined}
        />
        <OnlinePicker
          value={form.online}
          onChange={(v) => set("online", v)}
          hostsText={hostsText}
          onHostsText={setHostsText}
          error={error?.field === "online" || error?.field === "online_hosts" ? error : undefined}
        />
        {p.online.length > 0 && (
          <p className="muted small">
            Ticked from {p.title}'s page: it plays online. Untick what your {isMod ? "mod" : "branch"} doesn't connect to.
          </p>
        )}
        <SandboxReadyCheck checked={form.sandbox_ready} onChange={(v) => set("sandbox_ready", v)} />
        {!isMod && (
          <p className="muted small">
            Your {thing} gets its own save folder. Keep {p.title}'s save format if you can: players can copy their saves
            across when they pick your build.
          </p>
        )}
        {error && !(error.field && shownByField.has(error.field)) && (
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
