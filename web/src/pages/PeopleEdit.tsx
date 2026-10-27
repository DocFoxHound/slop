// SPDX-License-Identifier: AGPL-3.0-or-later
// A creative's own page editor (DESIGN §15.1): profile, look, packages, portfolio and "Get paid".
// Three steps for a new creative: describe yourself, point SLAPorSLOP at your portfolio, set up Stripe.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";

import {
  api,
  ApiError,
  workKey,
  type Accent,
  type MyProfile,
  type PortfolioOverview,
  type PortfolioWork,
  type ProfileInput,
  type Section,
  type Service,
  type Skill,
} from "../api";
import { ACCENTS, AI_USE, accentStyle } from "../components/People";
import { DesignPreview, portfolioWork } from "../components/PeopleDesign";
import { PasskeyNotice } from "../components/PasskeyNotice";
import { PageStatusDetails, type PeopleSaved } from "../components/PeopleStatus";
import { useTermsGate } from "../components/Terms";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { formatDate, money, toMinor } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { siteOrigin } from "../browsing";
import { isDesktop, openExternal } from "../platform";
import { LinkSignIn } from "../components/LinkSignIn";
import { useSession } from "../session";

/** The site the desktop app was built for (vite.config.ts, from SLOP_API_URL). */
declare const __SLOP_SITE_URL__: string;

const SECTION_NAMES: Record<Section, string> = {
  portfolio: "Portfolio",
  services: "Services",
  reviews: "Reviews",
  worked_on: "Worked on",
  about: "About",
};

const EMPTY: ProfileInput = {
  handle: "",
  name: "",
  kind: "individual",
  team_size: null,
  headline: "",
  about_md: "",
  location: "",
  skills: [],
  ai_use: "none",
  ai_note: "",
  trader: false,
  taking_work: true,
  turnaround: "",
  services: [],
  accent: "slime",
  sections: ["portfolio", "services", "reviews", "worked_on", "about"],
  links: [],
};

export function PeopleEdit() {
  useDocumentTitle("Your People page");
  const { me } = useSession();
  const mine = useLoad(
    () =>
      me
        ? api.people.mine().catch((err: unknown) => {
            if (err instanceof ApiError && err.status === 404) return null;
            throw err;
          })
        : Promise.resolve(undefined),
    [me?.id],
  );
  const skills = useLoad(() => api.people.skills(), []);
  const [profile, setProfile] = useState<MyProfile | null>();
  const current = profile === undefined ? mine.data : profile;

  if (me === null) {
    return (
      <div className="page narrow">
        <div className="empty-state">
          <p>Sign in to offer your services on People.</p>
          <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent("/people/me")}`}>
            Sign in
          </Link>
        </div>
      </div>
    );
  }
  if (mine.error) return <ErrorState error={mine.error} onRetry={mine.reload} />;
  if (current === undefined || !skills.data) return <Loading />;

  return (
    <div className="page narrow people-edit">
      <div className="crumbs">
        <Link to="/people">People</Link> › Your page
      </div>
      <h1>{current ? "Your People page" : "Offer your services"}</h1>
      {current && (
        <ol className="step-track">
          <li className="done">Describe yourself</li>
          <li className={current.live ? "done" : ""}>Portfolio approved</li>
          <li className={current.stripe_status === "active" ? "done" : ""}>Get paid</li>
        </ol>
      )}
      {current && (
        <section className="panel panel-pad people-status" aria-label="Your page's status">
          <PageStatusDetails
            status={current.status}
            inEditor
            notHireable={current.status.state === "live" && current.taking_work && current.stripe_status !== "active"}
          />
          <p className="small">
            Your page: <Link to={`/people/${current.handle}`}>/people/{current.handle}</Link>
          </p>
        </section>
      )}
      {!current && <ShopWindow />}
      {current && <GetPaid profile={current} onChange={(status) => setProfile({ ...current, stripe_status: status })} />}
      {current && <PortfolioPanel profile={current} skills={skills.data} />}
      <ProfileForm initial={current} skills={skills.data} adultConfirmed={Boolean(me?.adult_confirmed)} onSaved={setProfile} />
    </div>
  );
}

/** For someone starting out: what a People page is for, and the three steps to a live one. */
function ShopWindow() {
  return (
    <section className="panel panel-pad shop-window" aria-label="How People pages work">
      <p>
        Your People page is your shop window: it shows makers who might hire you who you are, your style, your best work and your rates.
      </p>
      <ol className="muted">
        <li>
          <b>Describe yourself</b> and what you charge, below.
        </li>
        <li>
          <b>Add your portfolio</b>: your best work, from a public GitHub repository of your own (there are ready-made templates, and a
          line to hand an AI assistant). A person at SLAPorSLOP looks at it, then your page goes public.
        </li>
        <li>
          <b>Get paid</b>: connect Stripe, so makers can hire you.
        </li>
      </ol>
    </section>
  );
}

function GetPaid({ profile, onChange }: { profile: MyProfile; onChange: (s: MyProfile["stripe_status"]) => void }) {
  const [params, setParams] = useSearchParams();
  const gate = useTermsGate();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // Back from Stripe's sign-up: ask Stripe how it went.
  useEffect(() => {
    if (!params.get("stripe")) return;
    void api.people.refreshPaid().then((r) => onChange(r.status as MyProfile["stripe_status"]), () => {});
    setParams({}, { replace: true });
  }, []);
  const start = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const r = await gate.guard(() => api.people.getPaid());
      if (r.url) {
        if (isDesktop) void openExternal(r.url);
        else window.location.assign(r.url);
      } else onChange(r.status as MyProfile["stripe_status"]);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const s = profile.stripe_status;
  return (
    <section className="panel panel-pad">
      {gate.prompt}
      <h2>Get paid</h2>
      <PasskeyNotice />
      {s === "active" ? (
        <p>
          ✓ Payments go straight to your own Stripe account. Refunds and payouts are in your Stripe dashboard.
          {profile.payments === "simulated" && <span className="muted"> (Simulated on this server.)</span>}
        </p>
      ) : (
        <>
          <p className="muted">
            Makers pay you directly through Stripe: SLAPorSLOP never holds your money. SLAPorSLOP's fee is 5% of each payment, taken from what you
            receive. Stripe asks for your identity, bank and tax details (it may report your income to tax authorities); SLAPorSLOP only keeps
            your Stripe account's ID.
          </p>
          {s === "pending" && <p>Stripe hasn't finished checking your details yet.</p>}
          {s === "restricted" && <p className="bad">Stripe needs more information before you can be paid.</p>}
          <div className="row-actions">
            <button className="btn slime" disabled={busy || profile.payments === "off"} onClick={() => void start()}>
              {s === "none" ? "Set up payments with Stripe" : "Continue on Stripe"}
            </button>
            {s === "pending" && (
              <button
                className="btn"
                onClick={() =>
                  void api.people.refreshPaid().then(
                    (r) => onChange(r.status as MyProfile["stripe_status"]),
                    (err: unknown) => setError(err instanceof ApiError ? err.message : String(err)),
                  )
                }
              >
                Check again
              </button>
            )}
          </div>
          {profile.payments === "off" && <p className="muted small">Payments aren't switched on yet on this server.</p>}
        </>
      )}
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}

const EXAMPLE = `{
  "avatar": "avatar.png",
  "banner": "banner.png",
  "works": [
    { "image": "art/knight.png", "title": "Knight sprite sheet", "skill": "pixel-art" },
    { "audio": "music/theme.ogg", "poster": "music/cover.png", "title": "Main theme", "skill": "music" },
    { "youtube": "https://youtu.be/VIDEO_ID", "title": "Showreel", "skill": "animation" }
  ]
}`;

/** Ready-to-copy portfolio repositories (examples/portfolio-templates in SLAPorSLOP's own repository). */
export const PORTFOLIO_TEMPLATES = "https://github.com/DocFoxHound/slop/tree/main/examples/portfolio-templates";

/** The line to hand an AI assistant: the guide, on this site (public/portfolio.md). */
export function portfolioPrompt(origin: string): string {
  return `Read ${origin}/portfolio.md and help me build my SLAPorSLOP portfolio.`;
}

/** A line to copy, with its own Copy button. */
function CopyLine({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  };
  return (
    <div className="ai-prompt">
      <code>{text}</code>
      <button type="button" className="btn small" onClick={copy} aria-label={label}>
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

/** A problem from reading the portfolio: one line each (portfolio.rs lists them all). */
function Problems({ text }: { text: string }) {
  const lines = text.split("\n").filter(Boolean);
  if (lines.length === 1) return <p className="form-error" role="alert">{lines[0]}</p>;
  return (
    <div className="form-error" role="alert">
      <p>SLAPorSLOP found {lines.length} things to fix in your portfolio:</p>
      <ul className="portfolio-problems">
        {lines.map((l, i) => (
          <li key={i}>{l}</li>
        ))}
      </ul>
    </div>
  );
}

function PortfolioPanel({ profile, skills }: { profile: MyProfile; skills: Skill[] }) {
  const loaded = useLoad(() => api.people.portfolio(), []);
  // Which GitHub account the repository has to belong to, or a button to link one.
  const identities = useLoad(() => api.identities().catch(() => null), []);
  const [o, setO] = useState<PortfolioOverview>();
  const [repo, setRepo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError | string>();
  const ov = o ?? loaded.data;
  useEffect(() => {
    if (loaded.data?.repo_url) setRepo(loaded.data.repo_url);
  }, [loaded.data]);
  if (!ov) return loaded.error ? <ErrorState error={loaded.error} onRetry={loaded.reload} /> : <Loading />;
  const check = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      setO(await api.people.checkPortfolio(repo.trim() || undefined));
    } catch (err) {
      setError(err instanceof ApiError ? err : String(err));
    } finally {
      setBusy(false);
    }
  };
  const shown = ov.pending ?? ov.live;
  const move = async (key: string, by: number) => {
    if (!shown) return;
    const keys = shown.works.map(workKey);
    const i = keys.indexOf(key);
    const j = i + by;
    if (j < 0 || j >= keys.length) return;
    [keys[i], keys[j]] = [keys[j], keys[i]];
    setError(undefined);
    try {
      setO(await api.people.setPortfolioOrder(keys));
    } catch (err) {
      setError(err instanceof ApiError ? err : String(err));
    }
  };
  const github = identities.data?.filter((i) => i.provider === "github") ?? [];
  const linkGithub = (
    <LinkSignIn provider="github" returnTo="/people/me#portfolio" onReturn={identities.reload}>
      Link GitHub
    </LinkSignIn>
  );
  const origin = siteOrigin(window.location, typeof __SLOP_SITE_URL__ === "string" ? __SLOP_SITE_URL__ : "");
  return (
    <section className="panel panel-pad" id="portfolio" aria-labelledby="portfolio-title">
      <h2 id="portfolio-title">Portfolio</h2>
      <p>
        Your portfolio is your shop window: your style and your best work, shown on your page next to your rates (your services) for
        makers who might hire you. SLAPorSLOP doesn't host the files: they stay in a public GitHub repository of your own, and SLAPorSLOP shows them
        from there. A person at SLAPorSLOP looks at every new portfolio before it shows.
      </p>
      <details className="portfolio-setup" open={!ov.repo_url}>
        <summary>
          <b>How to set one up</b>
        </summary>
        <ol>
          <li>
            Make a new <b>public</b> repository on GitHub for it, e.g. <code>portfolio</code>.{" "}
            {github.length > 0 ? (
              <>
                It has to belong to your linked GitHub account
                {github[0].login ? (
                  <>
                    {" "}
                    (<b>{github[0].login}</b>)
                  </>
                ) : null}
                .{" "}
              </>
            ) : identities.data ? (
              <>It has to belong to a GitHub account linked to your SLAPorSLOP account: {linkGithub} </>
            ) : null}
            <ExternalLink className="btn small" href="https://github.com/new">
              New repository on GitHub
            </ExternalLink>
          </li>
          <li>
            Add a file called <code>{ov.manifest_path}</code> that lists your works. Start from a{" "}
            <ExternalLink href={PORTFOLIO_TEMPLATES}>ready-made template</ExternalLink> (gallery, showreel, tracklist or studio) or the
            example below.
          </li>
          <li>
            Put your pictures and tracks in the <code>.slop/</code> folder, and link your videos from YouTube.
          </li>
          <li>Doll it up: a banner and avatar, titles and captions, and your best work first.</li>
          <li>
            Paste the repository's address below and choose <b>Check my portfolio now</b>. If anything needs fixing, SLAPorSLOP says exactly
            what and where.
          </li>
        </ol>
        <p className="small">
          <b>Using an AI assistant?</b> It can build the whole repository with you. Paste this in:
        </p>
        <CopyLine text={portfolioPrompt(origin)} label="Copy the line for your AI assistant" />
        <p className="muted small">
          Pictures: PNG, JPEG, WebP or GIF, up to 8 MB; works at least 32×32, avatar at least 128×128, banner at least 1200×300. Tracks:
          MP3, Ogg, WAV, FLAC or M4A, up to 15 MB. Videos: YouTube links. 1 to 30 works. The full list of rules is in the{" "}
          <a href="/portfolio.md">portfolio guide</a>.
        </p>
        <details>
          <summary>Example {ov.manifest_path}</summary>
          <pre className="code-block">{EXAMPLE}</pre>
          <p className="hint">
            Paths are inside .slop/. Skills: 2d-art, pixel-art, 3d, animation, ui-ux, music, sound, writing, voice, localization,
            playtesting.
          </p>
        </details>
      </details>
      <form className="form" onSubmit={check}>
        <label>
          Repository
          <input value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="https://github.com/you/portfolio" required />
        </label>
        <div className="row-actions">
          <button className="btn slime" disabled={busy}>
            {busy ? "Checking…" : "Check my portfolio now"}
          </button>
          {ov.checked_at && <span className="muted small">Last checked {formatDate(ov.checked_at)}</span>}
        </div>
      </form>
      {ov.result === "unchanged" && <p className="muted">Nothing new since the last check.</p>}
      {ov.result === "none" && (
        <p>
          There's no <code>{ov.manifest_path}</code> on that repository's default branch yet. Add it (a{" "}
          <ExternalLink href={PORTFOLIO_TEMPLATES}>template</ExternalLink> is the quickest start), then check again.
        </p>
      )}
      {ov.problem && <Problems text={ov.problem} />}
      {error && (
        <div className="form-error" role="alert">
          <p>{typeof error === "string" ? error : error.message}</p>
          {typeof error !== "string" && error.field === "github_account" && linkGithub}
        </div>
      )}
      {/* Their page previews the portfolio that isn't approved yet, for them only. */}
      {ov.pending && (
        <div className="notice portfolio-waiting">
          <p>
            New portfolio waiting for review ({ov.pending.works.length} works{ov.pending.design ? " and your page's design" : ""}). Your page
            shows it to you as a preview; everyone else sees {ov.live ? "your live page" : "nothing until it's approved"}.
          </p>
          <Link className="btn small slime" to={`/people/${profile.handle}`}>
            See the preview on your page
          </Link>
        </div>
      )}
      {ov.rejected && (
        <div className="notice portfolio-waiting">
          <p>Your last portfolio was turned down: {ov.rejected.status_reason}</p>
          <Link className="btn small" to={`/people/${profile.handle}`}>
            See it on your page
          </Link>
        </div>
      )}
      {shown && (
        <ol className="media-order">
          {shown.works.map((w, i) => (
            <li key={workKey(w)}>
              <WorkThumb w={w} />
              <span className="grow">{w.title ?? workKey(w)}</span>
              <button type="button" className="btn small" disabled={i === 0} onClick={() => void move(workKey(w), -1)} aria-label="Move up">
                ↑
              </button>
              <button
                type="button"
                className="btn small"
                disabled={i === shown.works.length - 1}
                onClick={() => void move(workKey(w), 1)}
                aria-label="Move down"
              >
                ↓
              </button>
            </li>
          ))}
        </ol>
      )}
      <h3 id="design">Your page's design</h3>
      <p className="muted small">
        Optional: design your page yourself with <code>.slop/page/index.html</code> (up to 64 KB) and <code>.slop/page/style.css</code> (up to
        32 KB) in the same repository. SLAPorSLOP checks them with your portfolio, removes anything pages can't have (scripts, forms, other sites'
        files…) and shows the result between SLAPorSLOP's header and your services and reviews. Put your works where you like with{" "}
        <code>{"<slop-works></slop-works>"}</code> and <code>{'<slop-work ref="1"></slop-work>'}</code>. Every rule is in the guide, under <a href="/portfolio.md">Your page's design</a>.
      </p>
      {shown?.design ? (
        <>
          <p className="small">{ov.pending ? "Your new design, as it will show once it's approved:" : "Your design, as your page shows it:"}</p>
          <DesignPreview
            design={shown.design}
            works={shown.works.map(portfolioWork)}
            services={profile.services.map((s) => ({ ...s, skill_name: skills.find((k) => k.slug === s.skill)?.name ?? s.skill }))}
            label={`Preview of ${profile.name}'s page`}
          />
        </>
      ) : (
        shown && <p className="muted small">No design yet: your page uses SLAPorSLOP's standard layout.</p>
      )}
    </section>
  );
}

function WorkThumb({ w }: { w: PortfolioWork }) {
  const url = w.kind === "image" ? w.url : w.poster?.url;
  return (
    <span className="item-thumb" aria-hidden="true">
      {url ? <img src={url} alt="" /> : w.kind === "audio" ? "♪" : "▶"}
    </span>
  );
}

/**
 * Only what the form sends: the server refuses fields it doesn't know (its status, Stripe and
 * portfolio details), so a page loaded from the server couldn't be saved as it came.
 */
export function toInput(m: MyProfile): ProfileInput {
  const { handle, name, kind, team_size, headline, about_md, location, skills, ai_use, ai_note, trader, taking_work, turnaround, services, accent, sections, links } = m;
  return { handle, name, kind, team_size, headline, about_md, location, skills, ai_use, ai_note, trader, taking_work, turnaround, services, accent, sections, links };
}

function ProfileForm({
  initial,
  skills,
  adultConfirmed,
  onSaved,
}: {
  initial: MyProfile | null;
  skills: Skill[];
  adultConfirmed: boolean;
  onSaved: (p: MyProfile) => void;
}) {
  const [p, setP] = useState<ProfileInput>(() => (initial ? toInput(initial) : EMPTY));
  const [adult, setAdult] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const navigate = useNavigate();
  const terms = useTermsGate();
  const set = <K extends keyof ProfileInput>(k: K, v: ProfileInput[K]) => setP((x) => ({ ...x, [k]: v }));
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const out = await terms.guard(() => api.people.save({ ...p, confirm_adult: adult }));
      onSaved(out);
      // Saved: to the page itself, which says so and where it stands (draft, waiting for review…).
      const saved: PeopleSaved = initial ? "saved" : "created";
      navigate(`/people/${out.handle}`, { state: { peopleSaved: saved } });
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setBusy(false);
    }
  };
  const moveSection = (i: number, by: number) => {
    const s = [...p.sections];
    [s[i], s[i + by]] = [s[i + by], s[i]];
    set("sections", s);
  };

  return (
    <form className="panel panel-pad form" onSubmit={submit} style={accentStyle(p.accent)}>
      {terms.prompt}
      <h2>About you</h2>
      <div className="form-row">
        <label>
          Name
          <input value={p.name} maxLength={60} required onChange={(e) => set("name", e.target.value)} placeholder="You, or your studio" />
        </label>
        <label>
          Page address
          <span className="prefixed">
            /people/
            <input
              value={p.handle}
              maxLength={40}
              required
              onChange={(e) => set("handle", e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ""))}
            />
          </span>
        </label>
      </div>
      <fieldset className="choice-list inline">
        <legend>Who</legend>
        <label className="check">
          <input type="radio" checked={p.kind === "individual"} onChange={() => set("kind", "individual")} /> Just me
        </label>
        <label className="check">
          <input type="radio" checked={p.kind === "studio"} onChange={() => setP((x) => ({ ...x, kind: "studio", team_size: x.team_size ?? 2 }))} />{" "}
          A studio of{" "}
          <input
            type="number"
            min={2}
            max={500}
            className="tiny"
            value={p.team_size ?? 2}
            aria-label="People in the studio"
            onChange={(e) => set("team_size", Number(e.target.value))}
            disabled={p.kind !== "studio"}
          />{" "}
          people
        </label>
      </fieldset>
      <label>
        Headline
        <input value={p.headline} maxLength={140} onChange={(e) => set("headline", e.target.value)} placeholder="Hand-painted characters for cosy games" />
      </label>
      <fieldset>
        <legend>What you do</legend>
        <div className="chips">
          {skills.map((s) => (
            <label key={s.slug} className={`chip toggle ${p.skills.includes(s.slug) ? "on" : ""}`}>
              <input
                type="checkbox"
                checked={p.skills.includes(s.slug)}
                onChange={(e) => set("skills", e.target.checked ? [...p.skills, s.slug] : p.skills.filter((x) => x !== s.slug))}
              />
              {s.name}
            </label>
          ))}
        </div>
      </fieldset>
      <fieldset className="choice-list">
        <legend>How do you use AI? (required, shown on your page)</legend>
        {(Object.keys(AI_USE) as Array<keyof typeof AI_USE>).map((k) => (
          <label className="check" key={k}>
            <input type="radio" checked={p.ai_use === k} onChange={() => set("ai_use", k)} />
            <span>
              <b>{AI_USE[k].label}</b> <span className="hint">{AI_USE[k].hint}</span>
            </span>
          </label>
        ))}
        <input value={p.ai_note} maxLength={300} onChange={(e) => set("ai_note", e.target.value)} placeholder="Anything to add (optional)" aria-label="AI use note" />
      </fieldset>
      <fieldset className="choice-list">
        <legend>Are you offering services as a business?</legend>
        <label className="check">
          <input type="radio" checked={p.trader} onChange={() => set("trader", true)} />
          <span>
            <b>Yes, as a business</b> <span className="hint">Consumer law applies when you work for consumers (for example the EU/UK 14-day right to cancel).</span>
          </span>
        </label>
        <label className="check">
          <input type="radio" checked={!p.trader} onChange={() => set("trader", false)} />
          <span>
            <b>No, as a private individual</b> <span className="hint">Shown to clients on your page and every offer.</span>
          </span>
        </label>
      </fieldset>
      <div className="form-row">
        <label>
          Location <span className="hint">(optional)</span>
          <input value={p.location} maxLength={60} onChange={(e) => set("location", e.target.value)} placeholder="Lisbon, PT" />
        </label>
        <label>
          Usual turnaround
          <input value={p.turnaround} maxLength={40} onChange={(e) => set("turnaround", e.target.value)} placeholder="1–2 weeks" />
        </label>
      </div>
      <label className="check">
        <input type="checkbox" checked={p.taking_work} onChange={(e) => set("taking_work", e.target.checked)} /> Taking work now
      </label>
      <label>
        About <span className="hint">(Markdown)</span>
        <textarea value={p.about_md} maxLength={6000} rows={6} onChange={(e) => set("about_md", e.target.value)} />
      </label>

      <h2>Services</h2>
      <ServicesEditor services={p.services} skills={skills.filter((s) => p.skills.includes(s.slug))} onChange={(s) => set("services", s)} />

      <h2>Your page's look</h2>
      <fieldset>
        <legend>Accent colour</legend>
        <div className="swatches">
          {(Object.keys(ACCENTS) as Accent[]).map((a) => (
            <label key={a} className={`swatch ${p.accent === a ? "on" : ""}`} style={{ background: ACCENTS[a].color }}>
              <input type="radio" name="accent" checked={p.accent === a} onChange={() => set("accent", a)} />
              <span className="sr-only">{ACCENTS[a].name}</span>
              {p.accent === a && <span aria-hidden="true" style={{ color: ACCENTS[a].ink }}>✓</span>}
            </label>
          ))}
        </div>
        <span className="hint">Your banner and avatar come from your portfolio.</span>
      </fieldset>
      <fieldset>
        <legend>Section order</legend>
        <ol className="media-order">
          {p.sections.map((s, i) => (
            <li key={s}>
              <span className="grow">{SECTION_NAMES[s]}</span>
              <button type="button" className="btn small" disabled={i === 0} onClick={() => moveSection(i, -1)} aria-label={`Move ${SECTION_NAMES[s]} up`}>
                ↑
              </button>
              <button
                type="button"
                className="btn small"
                disabled={i === p.sections.length - 1}
                onClick={() => moveSection(i, 1)}
                aria-label={`Move ${SECTION_NAMES[s]} down`}
              >
                ↓
              </button>
            </li>
          ))}
        </ol>
      </fieldset>
      <fieldset>
        <legend>Links (up to 5)</legend>
        {p.links.map((l, i) => (
          <div className="form-row" key={i}>
            <input
              value={l.label}
              maxLength={30}
              placeholder="Label"
              aria-label="Link label"
              onChange={(e) => set("links", p.links.map((x, k) => (k === i ? { ...x, label: e.target.value } : x)))}
            />
            <input
              value={l.url}
              placeholder="https://"
              aria-label="Link address"
              onChange={(e) => set("links", p.links.map((x, k) => (k === i ? { ...x, url: e.target.value } : x)))}
            />
            <button type="button" className="btn small" onClick={() => set("links", p.links.filter((_, k) => k !== i))}>
              Remove
            </button>
          </div>
        ))}
        {p.links.length < 5 && (
          <button type="button" className="btn small" onClick={() => set("links", [...p.links, { label: "", url: "" }])}>
            + Add a link
          </button>
        )}
      </fieldset>

      {!adultConfirmed && (
        <label className="check">
          <input type="checkbox" checked={adult} onChange={(e) => setAdult(e.target.checked)} required /> I'm 18 or older
        </label>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="row-actions">
        <button className="btn slime big" disabled={busy}>
          {initial ? "Save" : "Create my page"}
        </button>
        {initial && (
          <Link className="btn" to={`/people/${initial.handle}`}>
            View my page
          </Link>
        )}
      </div>
      <p className="muted small">
        You set your own prices, choose which jobs to take and how you work. SLAPorSLOP doesn't settle disputes between you and clients. See also{" "}
        <ExternalLink href="https://stripe.com/legal/connect-account">Stripe's terms</ExternalLink>.
      </p>
    </form>
  );
}

function ServicesEditor({ services, skills, onChange }: { services: Service[]; skills: Skill[]; onChange: (s: Service[]) => void }) {
  const [prices, setPrices] = useState<string[]>(services.map((s) => String(s.price_minor / 100)));
  const update = (i: number, change: Partial<Service>) => onChange(services.map((s, k) => (k === i ? { ...s, ...change } : s)));
  if (skills.length === 0) return <p className="muted">Pick what you do first.</p>;
  return (
    <div className="services-edit">
      {services.map((s, i) => (
        <div className="panel panel-pad" key={s.id || i}>
          <div className="form-row">
            <label>
              Name
              <input value={s.name} maxLength={60} required onChange={(e) => update(i, { name: e.target.value })} placeholder="Character sprite sheet" />
            </label>
            <label>
              From
              <input
                inputMode="decimal"
                value={prices[i] ?? ""}
                required
                onChange={(e) => {
                  setPrices((p) => p.map((x, k) => (k === i ? e.target.value : x)));
                  const m = toMinor(e.target.value);
                  if (m !== null) update(i, { price_minor: m });
                }}
              />
            </label>
            <label>
              Currency
              <select value={s.currency} onChange={(e) => update(i, { currency: e.target.value as Service["currency"] })}>
                <option value="usd">USD</option>
                <option value="eur">EUR</option>
                <option value="gbp">GBP</option>
              </select>
            </label>
            <label>
              Days
              <input type="number" min={1} max={365} value={s.days} onChange={(e) => update(i, { days: Number(e.target.value) })} />
            </label>
            <label>
              Skill
              <select value={s.skill} onChange={(e) => update(i, { skill: e.target.value })}>
                {skills.map((k) => (
                  <option key={k.slug} value={k.slug}>
                    {k.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <label>
            What's included
            <input value={s.description} maxLength={600} onChange={(e) => update(i, { description: e.target.value })} />
          </label>
          <div className="row-actions">
            <span className="hint">Shown as "from {money(s.price_minor, s.currency)}".</span>
            <button
              type="button"
              className="btn small"
              onClick={() => {
                setPrices((p) => p.filter((_, k) => k !== i));
                onChange(services.filter((_, k) => k !== i));
              }}
            >
              Remove
            </button>
          </div>
        </div>
      ))}
      {services.length < 8 && (
        <button
          type="button"
          className="btn small"
          onClick={() => {
            setPrices((p) => [...p, "50"]);
            onChange([...services, { id: "", name: "", description: "", price_minor: 5000, currency: "usd", days: 7, skill: skills[0].slug }]);
          }}
        >
          + Add a service
        </button>
      )}
    </div>
  );
}
