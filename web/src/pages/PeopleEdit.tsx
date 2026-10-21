// SPDX-License-Identifier: AGPL-3.0-or-later
// A creative's own page editor (DESIGN §15.1): profile, look, packages, portfolio and "Get paid".
// Three steps for a new creative: describe yourself, point Slop at your portfolio, set up Stripe.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";

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
import { PasskeyNotice } from "../components/PasskeyNotice";
import { useTermsGate } from "../components/Terms";
import { ErrorState, ExternalLink, Loading } from "../components/ui";
import { formatDate, money, toMinor } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { isDesktop, openExternal } from "../platform";
import { useSession } from "../session";

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
        <div className="empty-state">Sign in to offer your services on People.</div>
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
      {current?.live && (
        <p>
          Your page is live: <Link to={`/people/${current.handle}`}>/people/{current.handle}</Link>
        </p>
      )}
      {current?.hidden_reason && <div className="notice">Your page is hidden: {current.hidden_reason}</div>}
      {current && <GetPaid profile={current} onChange={(status) => setProfile({ ...current, stripe_status: status })} />}
      {current && <PortfolioPanel />}
      <ProfileForm initial={current} skills={skills.data} adultConfirmed={Boolean(me?.adult_confirmed)} onSaved={setProfile} />
    </div>
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
            Makers pay you directly through Stripe: Slop never holds your money. Slop's fee is 5% of each payment, taken from what you
            receive. Stripe asks for your identity, bank and tax details (it may report your income to tax authorities); Slop only keeps
            your Stripe account's ID.
          </p>
          {s === "pending" && <p>Stripe hasn't finished checking your details yet.</p>}
          {s === "restricted" && <p className="bad">Stripe needs more information before you can be paid.</p>}
          <div className="row-actions">
            <button className="btn slime" disabled={busy || profile.payments === "off"} onClick={() => void start()}>
              {s === "none" ? "Set up payments with Stripe" : "Continue on Stripe"}
            </button>
            {s === "pending" && (
              <button className="btn" onClick={() => void api.people.refreshPaid().then((r) => onChange(r.status as MyProfile["stripe_status"]))}>
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
    { "youtube": "https://youtu.be/…", "title": "Showreel" }
  ]
}`;

function PortfolioPanel() {
  const loaded = useLoad(() => api.people.portfolio(), []);
  const [o, setO] = useState<PortfolioOverview>();
  const [repo, setRepo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const ov = o ?? loaded.data;
  useEffect(() => {
    if (loaded.data?.repo_url) setRepo(loaded.data.repo_url);
  }, [loaded.data]);
  if (!ov) return <Loading />;
  const check = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      setO(await api.people.checkPortfolio(repo.trim() || undefined));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
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
    setO(await api.people.setPortfolioOrder(keys));
  };
  return (
    <section className="panel panel-pad">
      <h2>Portfolio</h2>
      <p className="muted">
        Slop doesn't host your files. Put them in a <b>public GitHub repository</b> (any one, e.g. <code>you/portfolio</code>) under a{" "}
        <code>.slop/</code> folder, listed in <code>{ov.manifest_path}</code>. Pictures: PNG, JPEG, WebP or GIF up to 8 MB. Tracks: MP3, Ogg,
        WAV, FLAC or M4A up to 15 MB. Videos: YouTube links. Up to 30 works. Every new portfolio is checked by a person before it shows.
      </p>
      <details>
        <summary>Example {ov.manifest_path}</summary>
        <pre className="code-block">{EXAMPLE}</pre>
        <p className="hint">Paths are inside .slop/. Skills: 2d-art, pixel-art, 3d, animation, ui-ux, music, sound, writing, voice, localization, playtesting.</p>
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
      {ov.result === "none" && <p>No {ov.manifest_path} found in that repository's default branch.</p>}
      {ov.problem && (
        <p className="form-error" role="alert">
          {ov.problem}
        </p>
      )}
      {error && <p className="form-error">{error}</p>}
      {ov.pending && <div className="notice">New portfolio waiting for review ({ov.pending.works.length} works).</div>}
      {ov.rejected && <div className="notice">Your last portfolio was turned down: {ov.rejected.status_reason}</div>}
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
  const [p, setP] = useState<ProfileInput>(initial ?? EMPTY);
  const [adult, setAdult] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const terms = useTermsGate();
  const set = <K extends keyof ProfileInput>(k: K, v: ProfileInput[K]) => {
    setSaved(false);
    setP((x) => ({ ...x, [k]: v }));
  };
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const out = await terms.guard(() => api.people.save({ ...p, confirm_adult: adult }));
      onSaved(out);
      setSaved(true);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
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
        {saved && <span className="good">Saved.</span>}
        {initial && (
          <Link className="btn" to={`/people/${initial.handle}`}>
            View my page
          </Link>
        )}
      </div>
      <p className="muted small">
        You set your own prices, choose which jobs to take and how you work. Slop doesn't settle disputes between you and clients. See also{" "}
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
