// People: the directory of creatives and their pages (DESIGN §15.1). A creative's page is
// built to sell a service — banner, accent colour, their own section order — so it isn't an app
// page with a different header.

import { useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";

import { api, ApiError, type CreativePage, type PeopleQuery, type Section, type Work } from "../api";
import { AI_USE, AiBadge, Avatar, Lightbox, PersonCard, RatingLine, WorkTile, accentStyle } from "../components/People";
import { ErrorState, ExternalLink, Loading, SectionHeader, Shelf } from "../components/ui";
import { formatDate, money } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { renderMarkdown } from "../markdown";
import { isDesktop, openExternal } from "../platform";
import { useSession } from "../session";

export function People() {
  useDocumentTitle("People");
  const [params, setParams] = useSearchParams();
  const q: PeopleQuery = {
    skill: params.get("skill") ?? undefined,
    taking_work: params.get("taking") === "1" || undefined,
    ai: (params.get("ai") as PeopleQuery["ai"]) ?? undefined,
    studios: params.get("studios") === "1" || undefined,
    sort: (params.get("sort") as PeopleQuery["sort"]) ?? undefined,
  };
  const list = useLoad(() => api.people.list(q), [params.toString()]);
  const set = (key: string, value: string | null) => {
    const next = new URLSearchParams(params);
    if (value) next.set(key, value);
    else next.delete(key);
    setParams(next, { replace: true });
  };

  return (
    <div className="page">
      <div className="people-hero">
        <div>
          <span className="badge neutral">SEPARATE FROM THE STORE</span>
          <h1>People</h1>
          <p className="muted">
            Artists, composers, sound designers, writers, voice actors and testers you can hire to make the parts of your game that
            shouldn't be slop. Rated only by makers who finished a job with them.
          </p>
        </div>
        <div className="row-actions">
          <Link className="btn" to="/people/jobs">
            Your jobs
          </Link>
          <Link className="btn slime" to="/people/me">
            Offer your services
          </Link>
        </div>
      </div>

      <div className="seg wrap" role="group" aria-label="Skill">
        <button type="button" className={!q.skill ? "on" : ""} aria-pressed={!q.skill} onClick={() => set("skill", null)}>
          Everyone
        </button>
        {list.data?.skills.map((s) => (
          <button
            type="button"
            key={s.slug}
            className={q.skill === s.slug ? "on" : ""}
            aria-pressed={q.skill === s.slug}
            onClick={() => set("skill", s.slug)}
          >
            {s.name} <span className="faint">{s.count}</span>
          </button>
        ))}
      </div>
      <div className="filters">
        <label>
          Order{" "}
          <select value={q.sort ?? "shuffle"} onChange={(e) => set("sort", e.target.value === "shuffle" ? null : e.target.value)}>
            <option value="shuffle">Shuffled daily</option>
            <option value="rating">Best rated</option>
            <option value="clients">Most clients</option>
          </select>
        </label>
        <label>
          AI use{" "}
          <select value={q.ai ?? ""} onChange={(e) => set("ai", e.target.value || null)}>
            <option value="">Any</option>
            <option value="none">No AI only</option>
            <option value="assisted">No AI or AI-assisted</option>
          </select>
        </label>
        <label className="check">
          <input type="checkbox" checked={Boolean(q.taking_work)} onChange={(e) => set("taking", e.target.checked ? "1" : null)} /> Taking
          work now
        </label>
        <label className="check">
          <input type="checkbox" checked={Boolean(q.studios)} onChange={(e) => set("studios", e.target.checked ? "1" : null)} /> Studios
          only
        </label>
      </div>

      {list.error ? (
        <ErrorState error={list.error} onRetry={list.reload} />
      ) : !list.data ? (
        <Loading />
      ) : list.data.people.length === 0 ? (
        <div className="empty-state">Nobody matches those filters yet.</div>
      ) : (
        <div className="people-grid">
          {list.data.people.map((c) => (
            <PersonCard c={c} key={c.handle} />
          ))}
        </div>
      )}

      <div className="panel panel-pad people-how">
        <b>How hiring works</b>
        <ol className="muted">
          <li>Pick a creative and one of your projects (any stage, public or private).</li>
          <li>They send an offer: price, date, who owns the work. You see it in plain words.</li>
          <li>Accept and pay them directly (half now and half on delivery, unless they choose otherwise). Slop never holds the money.</li>
          <li>When it's delivered, say it looks good. They're credited on your app's page, and your app gets the ✋ Made with people badge.</li>
        </ol>
        <p className="muted small">
          Your contract is with the creative, not Slop. Slop takes a 5% fee from what the creative receives; nothing is added to your price.
        </p>
      </div>
    </div>
  );
}

export function PersonPage() {
  const { handle = "" } = useParams();
  const page = useLoad(() => api.people.page(handle), [handle]);
  useDocumentTitle(page.data?.name);
  if (page.error)
    return (
      <div className="page">
        <ErrorState error={page.error} onRetry={page.reload} />
      </div>
    );
  if (!page.data) return <Loading />;
  return <PersonView p={page.data} reload={page.reload} />;
}

function PersonView({ p, reload }: { p: CreativePage; reload: () => void }) {
  const { me } = useSession();
  const [hiring, setHiring] = useState<{ serviceId?: string; skill?: string }>();
  const [open, setOpen] = useState<Work>();
  const onProseClick = (e: MouseEvent) => {
    const link = (e.target as HTMLElement).closest("a[data-external]");
    if (link) {
      e.preventDefault();
      void openExternal((link as HTMLAnchorElement).href);
    }
  };

  const sections: Record<Section, ReactNode> = {
    portfolio:
      p.works.length > 0 ? (
        <section key="portfolio" aria-labelledby="sec-portfolio">
          <SectionHeader title="Portfolio" sub={`${p.works.length} work${p.works.length === 1 ? "" : "s"}`} />
          <div className="works">
            {p.works.map((w) => (
              <WorkTile key={w.key} w={w} onOpen={setOpen} />
            ))}
          </div>
        </section>
      ) : null,
    services:
      p.services.length > 0 ? (
        <section key="services">
          <SectionHeader title="Services" sub="Starting prices; the offer has the exact one" />
          <div className="services">
            {p.services.map((s) => (
              <div className="panel panel-pad service" key={s.id}>
                <div className="service-head">
                  <b>{s.name}</b>
                  <span className="service-price">from {money(s.price_minor, s.currency)}</span>
                </div>
                {s.description && <p className="muted small">{s.description}</p>}
                <div className="service-foot">
                  <span className="chip">{s.skill_name}</span>
                  <span className="muted small">about {s.days} day{s.days === 1 ? "" : "s"}</span>
                  {!p.is_me && p.can_hire && (
                    <button type="button" className="btn small accent" onClick={() => setHiring({ serviceId: s.id, skill: s.skill })}>
                      Hire for this
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </section>
      ) : null,
    reviews: (
      <section key="reviews">
        <SectionHeader title="Reviews from clients" sub="Only makers who finished a job through Slop can review" />
        {p.reviews.length === 0 ? (
          <p className="muted">No reviews yet.</p>
        ) : (
          p.reviews.map((r) => <Review key={r.job_id} r={r} canReply={p.is_me} onSaved={reload} />)
        )}
      </section>
    ),
    worked_on:
      p.worked_on.length > 0 ? (
        <section key="worked_on">
          <SectionHeader title="Worked on" sub="Apps on Slop that credit them" />
          <Shelf apps={p.worked_on.map((a) => ({ ...a, reason: `${a.role} by ${p.name}` }))} empty={null} />
        </section>
      ) : null,
    about: p.about_md.trim() ? (
      <section key="about">
        <SectionHeader title="About" />
        {/* Rendered with raw HTML disabled; see markdown.ts. */}
        <div className="prose" onClick={onProseClick} dangerouslySetInnerHTML={{ __html: renderMarkdown(p.about_md) }} />
      </section>
    ) : null,
  };

  return (
    <div className="page person-page" style={accentStyle(p.accent)}>
      <div className="crumbs">
        <Link to="/people">People</Link> › {p.name}
      </div>
      {p.is_me && (
        <div className="notice">
          This is your page.{" "}
          {p.status === "waiting" && "Only you can see it until your first portfolio is approved. "}
          {p.status === "hidden" && `It's hidden: ${p.hidden_reason ?? "it broke Slop's rules"}. `}
          <Link to="/people/me">Edit your page</Link>
        </div>
      )}
      <div className="person-banner" aria-hidden="true">
        {p.banner_url && <img src={p.banner_url} alt="" />}
      </div>
      <div className="person-head panel">
        <Avatar name={p.name} url={p.avatar_url} big />
        <div className="person-head-main">
          <div className="title-row">
            <h1>{p.name}</h1>
            <span className="badge neutral">{p.kind === "studio" ? `STUDIO · ${p.team_size ?? 2} PEOPLE` : "INDIVIDUAL"}</span>
            <AiBadge ai={p.ai_use} />
          </div>
          {p.headline && <p className="lede">{p.headline}</p>}
          <div className="muted small">
            {p.location && `${p.location} · `}
            <span className={`avail ${p.can_hire ? "on" : ""}`} aria-hidden="true" />
            {p.can_hire ? "Taking work" : p.taking_work ? "Not taking paid work yet" : "Fully booked"}
            {p.turnaround && ` · usually ${p.turnaround}`}
          </div>
          <div className="chips">
            {p.skills.map((s) => (
              <Link className="chip" key={s.slug} to={`/people?skill=${s.slug}`}>
                {s.name}
              </Link>
            ))}
          </div>
          {p.links.length > 0 && (
            <div className="person-links">
              {p.links.map((l) => (
                <ExternalLink key={l.url} href={l.url}>
                  {l.label} <span className="faint">({l.domain})</span>
                </ExternalLink>
              ))}
            </div>
          )}
        </div>
        <div className="person-head-side">
          <RatingLine rating={p.rating} clients={p.clients} />
          {!p.is_me &&
            (p.can_hire ? (
              <button type="button" className="btn big accent" onClick={() => setHiring({})}>
                Hire {p.kind === "studio" ? "them" : p.name.split(" ")[0]}
              </button>
            ) : (
              <button type="button" className="btn big" disabled>
                {p.taking_work ? "Not taking paid work yet" : "Fully booked"}
              </button>
            ))}
          <span className="muted small">
            {p.trader ? "Offers services as a business" : "Offers services as a private individual"}
          </span>
        </div>
      </div>

      {hiring && (
        <HireForm
          p={p}
          initial={hiring}
          signedIn={Boolean(me)}
          adultConfirmed={Boolean(me?.adult_confirmed)}
          onClose={() => setHiring(undefined)}
        />
      )}

      <div className="two-col person-body">
        <div className="person-sections">{p.sections.map((s) => sections[s])}</div>
        <aside className="side-col">
          <div className="panel panel-pad">
            <b>AI use</b>
            <div className="ai-box">
              <AiBadge ai={p.ai_use} />
            </div>
            <div className="muted small">{p.ai_note || AI_USE[p.ai_use].hint}</div>
            <div className="faint small">Required on every profile. Each offer says it again for that job.</div>
          </div>
          <div className="panel panel-pad small muted">
            <b className="text">Hiring through Slop</b>
            <p>You pay {p.name} directly through Stripe. Slop never holds the money.</p>
            <p>When the job is done, your app's page credits them and gets the ✋ Made with people badge.</p>
            <p>Your contract is with {p.name}, not Slop. Slop doesn't settle disputes.</p>
          </div>
        </aside>
      </div>
      {open && <Lightbox w={open} onClose={() => setOpen(undefined)} />}
    </div>
  );
}

function Review({ r, canReply, onSaved }: { r: CreativePage["reviews"][number]; canReply: boolean; onSaved: () => void }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(r.reply ?? "");
  const [error, setError] = useState<string>();
  const save = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api.people.reply(r.job_id, text);
      setEditing(false);
      onSaved();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <article className="panel review-card">
      <div className="review-who">
        <b>{r.client_name}</b>
        <span className="badge official">✓ VERIFIED CLIENT</span>
        <span className={`vote-chip ${r.vote === "not_slop" ? "up" : "down"}`}>{r.vote === "not_slop" ? "👍 Not Slop" : "👎 Slop"}</span>
      </div>
      <div className="faint small">
        {r.work} · {r.skill.name}
        {r.project && r.project_slug && (
          <>
            {" "}
            for <Link to={`/app/${r.project_slug}`}>{r.project}</Link>
          </>
        )}{" "}
        · {formatDate(r.created_at)}
      </div>
      {r.body && <p>{r.body}</p>}
      {r.reply && !editing && (
        <div className="review-reply">
          <b>Reply</b> {r.reply}
        </div>
      )}
      {canReply &&
        (editing ? (
          <form className="form" onSubmit={save}>
            <label>
              Your public reply
              <textarea value={text} maxLength={1000} rows={3} onChange={(e) => setText(e.target.value)} />
            </label>
            {error && <p className="form-error">{error}</p>}
            <div className="row-actions">
              <button className="btn slime small">Save reply</button>
              <button type="button" className="btn small" onClick={() => setEditing(false)}>
                Cancel
              </button>
            </div>
          </form>
        ) : (
          <button type="button" className="link-btn small" onClick={() => setEditing(true)}>
            {r.reply ? "Edit your reply" : "Reply"}
          </button>
        ))}
    </article>
  );
}

function HireForm({
  p,
  initial,
  signedIn,
  adultConfirmed,
  onClose,
}: {
  p: CreativePage;
  initial: { serviceId?: string; skill?: string };
  signedIn: boolean;
  adultConfirmed: boolean;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const projects = useLoad(() => (signedIn ? api.myApps() : Promise.resolve([])), [signedIn]);
  const [project, setProject] = useState("");
  const [serviceId, setServiceId] = useState(initial.serviceId ?? "");
  const [skill, setSkill] = useState(initial.skill ?? p.skills[0]?.slug ?? "");
  const [request, setRequest] = useState("");
  const [wantedBy, setWantedBy] = useState("");
  const [business, setBusiness] = useState(false);
  const [adult, setAdult] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  if (!signedIn) {
    return (
      <div className="panel panel-pad hire-form">
        <p>Sign in to hire {p.name}.</p>
        {isDesktop ? null : (
          <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(`/people/${p.handle}`)}`}>
            Sign in
          </Link>
        )}
      </div>
    );
  }
  const apps = projects.data ?? [];
  const chosenProject = project || apps[0]?.slug || "";
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const job = await api.people.hire(p.handle, {
        app_slug: chosenProject,
        skill,
        service_id: serviceId || undefined,
        request_md: request,
        wanted_by: wantedBy || undefined,
        client_is_business: business,
        confirm_adult: adult,
      });
      navigate(`/jobs/${job.id}`);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <form className="panel panel-pad form hire-form" onSubmit={submit} aria-label={`Hire ${p.name}`}>
      <h2>Hire {p.name}</h2>
      {projects.data && apps.length === 0 ? (
        <p>
          Jobs belong to one of your projects, so the work can be credited on its page. It can be a private draft or a Coming soon page.{" "}
          <Link to="/publish/new">Create a project first</Link>.
        </p>
      ) : (
        <>
          <label>
            For which project?
            <select value={chosenProject} onChange={(e) => setProject(e.target.value)} required>
              {apps.map((a) => (
                <option key={a.slug} value={a.slug}>
                  {a.title}
                  {a.status && a.status !== "listed" ? " (draft)" : ""}
                </option>
              ))}
            </select>
          </label>
          {p.services.length > 0 && (
            <label>
              Package
              <select
                value={serviceId}
                onChange={(e) => {
                  setServiceId(e.target.value);
                  const s = p.services.find((x) => x.id === e.target.value);
                  if (s) setSkill(s.skill);
                }}
              >
                <option value="">Something else (describe it below)</option>
                {p.services.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name} · from {money(s.price_minor, s.currency)}
                  </option>
                ))}
              </select>
            </label>
          )}
          <label>
            Skill
            <select value={skill} onChange={(e) => setSkill(e.target.value)}>
              {p.skills.map((s) => (
                <option key={s.slug} value={s.slug}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            What do you need?
            <textarea
              value={request}
              onChange={(e) => setRequest(e.target.value)}
              rows={5}
              maxLength={4000}
              required
              placeholder="Style, how many, sizes or lengths, references, anything they should know."
            />
            <span className="hint">This is part of the job's record. {p.name} replies with an offer you can accept, change or decline.</span>
          </label>
          <label>
            Hoping to have it by <span className="hint">(optional)</span>
            <input type="date" value={wantedBy} onChange={(e) => setWantedBy(e.target.value)} />
          </label>
          {p.trader && (
            <label className="check">
              <input type="checkbox" checked={business} onChange={(e) => setBusiness(e.target.checked)} /> I'm hiring as a business (not
              as a consumer)
            </label>
          )}
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
            <button className="btn slime" disabled={busy || !chosenProject}>
              Send request
            </button>
            <button type="button" className="btn" onClick={onClose}>
              Cancel
            </button>
          </div>
          <p className="muted small">Nothing is paid until you accept their offer.</p>
        </>
      )}
    </form>
  );
}
