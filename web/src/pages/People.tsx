// SPDX-License-Identifier: AGPL-3.0-or-later
// People: the directory of creatives and their pages (DESIGN §15.1). A creative's page is
// built to sell a service — banner, accent colour, their own section order — so it isn't an app
// page with a different header.

import { useEffect, useRef, useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { Link, useLocation, useNavigate, useParams, useSearchParams } from "react-router";

import { api, ApiError, type AiUse, type CreativeCard, type CreativePage, type PageStatus, type PeopleQuery, type Section, type Work } from "../api";
import { Avatar } from "../components/Avatar";
import { AI_USE, AiBadge, Lightbox, PersonCard, RatingLine, ServicesList, WorkTile, accentStyle } from "../components/People";
import { DesignArea, slopParts } from "../components/PeopleDesign";
import {
  LiveBar,
  PageStateBadge,
  PageStatusDetails,
  PreviewBar,
  ownPageView,
  useMyPeoplePage,
  type PeopleSaved,
} from "../components/PeopleStatus";
import { ReportButton } from "../components/Report";
import { useTermsGate } from "../components/Terms";
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
    q: params.get("q")?.trim() || undefined,
  };
  const list = useLoad(() => api.people.list(q), [params.toString()]);
  // A signed-in creative's own page, so it's always easy to find again (with where it stands).
  const mine = useMyPeoplePage();
  // The front page, before any skill, filter or search: the day's shelves above everyone.
  const front = params.size === 0;
  const set = (key: string, value: string | null) => {
    const next = new URLSearchParams(params);
    if (value) next.set(key, value);
    else next.delete(key);
    setParams(next, { replace: true });
  };
  const skillName = list.data?.skills.find((s) => s.slug === q.skill)?.name;
  const order = q.sort === "rating" ? "Best rated first" : q.sort === "clients" ? "Most clients first" : "Shuffled daily, so nobody is buried";

  return (
    <div className="page">
      <nav className="store-nav people-nav" aria-label="People">
        <div className="store-nav-links" role="group" aria-label="Skill">
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
        <PeopleSearch value={q.q ?? ""} onSearch={(text) => set("q", text.trim() || null)} />
      </nav>

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
          {mine.data ? (
            <Link className="btn slime" to={`/people/${mine.data.handle}`}>
              Your People page
              {mine.data.status.state !== "live" && <PageStateBadge status={mine.data.status} />}
            </Link>
          ) : (
            <Link className="btn slime" to="/people/me">
              Offer your services
            </Link>
          )}
        </div>
      </div>

      {front && <PeopleShelves />}

      <section aria-labelledby="people-all-title">
        <div className="section-h">
          <h2 id="people-all-title">{skillName ?? "Everyone"}</h2>
          <span className="sub">{q.q ? `Matching “${q.q}” · ${order}` : order}</span>
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
              {(Object.keys(AI_USE) as AiUse[]).map((k) => (
                <option key={k} value={k}>
                  {AI_USE[k].short}
                </option>
              ))}
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
          <div className="empty-state">{q.q ? `Nobody matches “${q.q}” with those filters.` : "Nobody matches those filters yet."}</div>
        ) : (
          <div className="people-grid">
            {list.data.people.map((c) => (
              <PersonCard c={c} key={c.handle} />
            ))}
          </div>
        )}
      </section>

      {!mine.data && (
        <div className="panel panel-pad people-how people-offer">
          <b>Offering your services?</b>
          <p className="muted">
            A People page is your shop window: it shows makers who might hire you who you are, your style, your best work and your rates.
            Your best work comes from a portfolio, a public GitHub repository of your own with your pictures and tracks in it (videos are
            YouTube links). There are ready-made templates to start from, and a line you can hand an AI assistant to build it with you. A
            person at SLAPorSLOP looks at it, then your page goes public.
          </p>
          <Link className="btn" to="/people/me">
            Make your People page
          </Link>
        </div>
      )}
    </div>
  );
}

/**
 * The front page's shelves, like the store's: a different mix every day (people/discover.rs).
 * None is ordered by ratings or clients (LEGAL-GUIDANCE HUM-10).
 */
function PeopleShelves() {
  const shelves = useLoad(() => api.people.discover(), []);
  if (shelves.error) return <ErrorState error={shelves.error} onRetry={shelves.reload} />;
  const d = shelves.data;
  if (!d) return null;
  const shelf = (title: string, sub: string, people: CreativeCard[], all?: string) =>
    people.length > 0 && (
      <section className="people-shelf-section" aria-label={title}>
        <SectionHeader title={title} sub={sub}>
          {all && (
            <Link to={all} className="sub">
              See all →
            </Link>
          )}
        </SectionHeader>
        <div className="people-shelf">
          {people.map((c) => (
            <PersonCard c={c} key={c.handle} />
          ))}
        </div>
      </section>
    );
  return (
    <>
      {shelf("Taking work now", "Open for new work and ready to be paid. A different mix every day.", d.taking_work, "/people?taking=1")}
      {d.spotlight &&
        shelf(
          `Spotlight: ${d.spotlight.skill.name}`,
          "A different skill every day",
          d.spotlight.people,
          `/people?skill=${encodeURIComponent(d.spotlight.skill.slug)}`,
        )}
      {shelf("New on People", "Pages that went live lately", d.new)}
      {shelf("Studios", "Teams who take on bigger jobs together", d.studios, "/people?studios=1")}
    </>
  );
}

/** The People search: names, handles and headlines (/people?q=…). The store search is the store's. */
function PeopleSearch({ value, onSearch }: { value: string; onSearch: (text: string) => void }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    onSearch(text);
  };
  return (
    <form className="search store-search people-search" role="search" onSubmit={submit}>
      <input
        type="search"
        placeholder="Search people by name or what they do…"
        aria-label="Search people"
        value={text}
        maxLength={100}
        onChange={(e) => {
          setText(e.target.value);
          // Clearing the box shows everyone again.
          if (!e.target.value && value) onSearch("");
        }}
      />
    </form>
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

/** After saving in the editor: the page says so, with where it stands now. */
function savedMessage(saved: PeopleSaved, status: PageStatus): string {
  const done = saved === "created" ? "Your People page is created" : "Your People page is saved";
  switch (status.state) {
    case "draft":
      return `${done}. Only you can see it for now: add your portfolio next, and SLAPorSLOP reviews it before your page goes public.`;
    case "waiting":
      return `${done} and waiting for SLAPorSLOP's review of your portfolio. Only you can see it until then.`;
    case "turned_down":
      return `${done}. Only you can see it until a portfolio is approved.`;
    case "live":
      return `${done}. Everyone sees the changes now.`;
    case "hidden":
      return `${done}. It's still hidden.`;
  }
}

function PersonView({ p: loaded, reload }: { p: CreativePage; reload: () => void }) {
  const { me } = useSession();
  const location = useLocation();
  const navigate = useNavigate();
  // The creative's own page previews their portfolio waiting for review (or turned down);
  // ?view=live shows exactly what everyone else sees.
  const [params] = useSearchParams();
  const { page: p, view } = ownPageView(loaded, params.get("view") === "live");
  // Arrived here from the editor's Save: say so once (not again on reload or Back).
  const [saved] = useState(() => (location.state as { peopleSaved?: PeopleSaved } | null)?.peopleSaved);
  const savedRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!saved) return;
    navigate(location.pathname, { replace: true, state: null });
    savedRef.current?.focus({ preventScroll: true });
  }, []);
  const [hiring, setHiring] = useState<{ serviceId?: string; skill?: string }>();
  const [open, setOpen] = useState<Work>();
  const onProseClick = (e: MouseEvent) => {
    const link = (e.target as HTMLElement).closest("a[data-external]");
    if (link) {
      e.preventDefault();
      void openExternal((link as HTMLAnchorElement).href);
    }
  };

  const servicesList = () =>
    p.services.length > 0 ? (
      <ServicesList services={p.services} onHire={!p.is_me && p.can_hire ? (s) => setHiring({ serviceId: s.id, skill: s.skill }) : undefined} />
    ) : null;
  // With their own design, the creative's page is: SLAPorSLOP's header, their design (which places
  // their works), then SLAPorSLOP's services, reviews and credits, which a design can't reach.
  const shown: Section[] = p.design ? ["services", "reviews", "worked_on"] : p.sections;

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
          {servicesList()}
        </section>
      ) : null,
    reviews: (
      <section key="reviews">
        <SectionHeader title="Reviews from clients" sub="Only makers who finished a job through SLAPorSLOP can review" />
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
          <SectionHeader title="Worked on" sub="Apps on SLAPorSLOP that credit them" />
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
      {view === "preview" && p.mine && p.preview_set && <PreviewBar handle={p.handle} status={p.mine} set={p.preview_set} />}
      {view === "live" && p.mine && p.preview_set && <LiveBar handle={p.handle} status={p.mine} set={p.preview_set} />}
      {saved && p.mine && (
        <div className="notice people-saved" role="status" tabIndex={-1} ref={savedRef}>
          <span aria-hidden="true">✓ </span>
          {savedMessage(saved, p.mine)}
        </div>
      )}
      {p.is_me && p.mine && (
        <section className="notice people-status" aria-label="Your page's status">
          <p className="muted small">This is your page. Only you see this box.</p>
          <PageStatusDetails status={p.mine} notHireable={p.mine.state === "live" && p.taking_work && !p.can_hire && !p.closed} />
          <p>
            <Link to="/people/me">Edit your page</Link>
          </p>
        </section>
      )}
      {p.closed && (
        <div className="notice">
          {p.name} closed their SLAPorSLOP account. This page is kept as it was, as the record of their work and reviews, and
          takes no new work.
        </div>
      )}
      <PreviewFrame on={view === "preview"}>
        <div className="person-banner" aria-hidden="true">
          {p.banner_url && <img src={p.banner_url} alt="" />}
        </div>
        <div className="person-head panel">
          <Avatar name={p.name} url={p.avatar_url} className="p-avatar big" />
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
                  {p.closed ? "Account closed" : p.taking_work ? "Not taking paid work yet" : "Fully booked"}
                </button>
              ))}
            <span className="muted small">
              {p.trader ? "Offers services as a business" : "Offers services as a private individual"}
            </span>
            {!p.is_me && <ReportButton kind="profile" subject={p.handle} label="this profile" />}
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

        {p.design && (
          <DesignArea design={p.design} label={`${p.name}'s page`} parts={slopParts({ works: p.works, onOpen: setOpen })} />
        )}

        <div className="two-col person-body">
          <div className="person-sections">{shown.map((s) => sections[s])}</div>
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
              <b className="text">Hiring through SLAPorSLOP</b>
              <p>You pay {p.name} directly through Stripe. SLAPorSLOP never holds the money.</p>
              <p>When the job is done, your app's page credits them and gets the ✋ Made with people badge.</p>
              <p>Your contract is with {p.name}, not SLAPorSLOP. SLAPorSLOP doesn't settle disputes.</p>
            </div>
          </aside>
        </div>
      </PreviewFrame>
      {open && <Lightbox w={open} onClose={() => setOpen(undefined)} />}
    </div>
  );
}

/**
 * The previewed page, outlined and labelled at its top and bottom, so no part of it (or a
 * screenshot of it) passes for the live page. Off, just the page.
 */
function PreviewFrame({ on, children }: { on: boolean; children: ReactNode }) {
  if (!on) return <>{children}</>;
  return (
    <div className="own-preview-frame">
      <p className="own-preview-ribbon">Preview · not your live page</p>
      {children}
      <p className="own-preview-ribbon end">Preview · not your live page</p>
    </div>
  );
}

function Review({ r, canReply, onSaved }: { r: CreativePage["reviews"][number]; canReply: boolean; onSaved: () => void }) {
  const [editing, setEditing] = useState(false);
  const [disputing, setDisputing] = useState(false);
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
    <article className="panel review-card" id={`review-${r.job_id}`}>
      <div className="review-who">
        <b>{r.client_name}</b>
        <span className="badge official">✓ VERIFIED CLIENT</span>
        <span className={`vote-chip ${r.vote === "not_slop" ? "up" : "down"}`}>{r.vote === "not_slop" ? "👍 Slaps" : "👎 Slop"}</span>
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
      {r.mediator_note && (
        <div className="mediator-note">
          <b>Mediator's note</b> {r.mediator_note}
        </div>
      )}
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
          <span className="row-actions">
            <button type="button" className="link-btn small" onClick={() => setEditing(true)}>
              {r.reply ? "Edit your reply" : "Reply"}
            </button>
            {r.dispute === null && (
              <button type="button" className="link-btn small" onClick={() => setDisputing(true)}>
                Dispute this review
              </button>
            )}
            {r.dispute && <span className="muted small">Dispute: {DISPUTE_STATUS[r.dispute]}</span>}
          </span>
        ))}
      {disputing && <DisputeForm jobId={r.job_id} onDone={() => { setDisputing(false); onSaved(); }} />}
      {!canReply && (
        <div className="review-report">
          <ReportButton kind="review" subject={r.job_id} label="this review" />
        </div>
      )}
    </article>
  );
}

const DISPUTE_STATUS: Record<string, string> = {
  pending: "waiting for a mediator",
  removed: "removed",
  noted: "kept, with a mediator's note",
  kept: "kept",
};

const DISPUTE_REASONS: Array<[string, string]> = [
  ["false_fact", "It states something the job record shows is false"],
  ["extortion", "It was used as a threat (\"more for free or I'll leave a bad review\")"],
  ["not_about_job", "It isn't about the work on this job"],
  ["rules", "It breaks the rules: insults, threats or private information"],
];

function DisputeForm({ jobId, onDone }: { jobId: string; onDone: () => void }) {
  const [reason, setReason] = useState("false_fact");
  const [statement, setStatement] = useState("");
  const [error, setError] = useState<string>();
  return (
    <form
      className="form dispute-form"
      onSubmit={(e) => {
        e.preventDefault();
        api.people.disputeReview(jobId, reason, statement).then(onDone, (err: unknown) =>
          setError(err instanceof ApiError ? err.message : String(err)),
        );
      }}
    >
      <p className="muted small">
        A mediator checks the review against SLAPorSLOP's rules and the job record. Reviews aren't removed for being negative: an honest opinion
        stays, and your reply is your answer to it.
      </p>
      <fieldset className="choice-list">
        <legend>Why</legend>
        {DISPUTE_REASONS.map(([v, label]) => (
          <label className="check" key={v}>
            <input type="radio" name={`reason-${jobId}`} checked={reason === v} onChange={() => setReason(v)} /> {label}
          </label>
        ))}
      </fieldset>
      <label>
        Explain, pointing to the job record
        <textarea value={statement} rows={4} maxLength={2000} required onChange={(e) => setStatement(e.target.value)} />
      </label>
      {error && <p className="form-error">{error}</p>}
      <div className="row-actions">
        <button className="btn slime small">Send to a mediator</button>
        <button type="button" className="btn small" onClick={onDone}>
          Cancel
        </button>
      </div>
    </form>
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
  const [exact, setExact] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // Before the early return below: the form opens signed out while the session is still loading,
  // and every hook has to run on every render (rules of hooks).
  const terms = useTermsGate();

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
  const send = async (inquiry: boolean) => {
    setBusy(true);
    setError(undefined);
    try {
      const job = await terms.guard(() => api.people.hire(p.handle, {
        app_slug: chosenProject,
        skill,
        service_id: serviceId || undefined,
        request_md: request,
        wanted_by: wantedBy || undefined,
        client_is_business: business,
        inquiry,
        exact_wording_ack: exact,
        confirm_adult: adult,
      }));
      navigate(`/jobs/${job.id}`);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <form
      className="panel panel-pad form hire-form"
      onSubmit={(e) => {
        e.preventDefault();
        void send(false);
      }}
      aria-label={`Hire ${p.name}`}
    >
      {terms.prompt}
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
          <div className="exact-warning" role="note">
            <b>Write exactly what you need.</b> Once you accept an offer, {p.name} is expected to deliver exactly what it says: nothing
            more and nothing less. Anything you leave out isn't part of the work, and any change afterwards is new work with its own price
            and date. Be precise: sizes, quantities, formats, style, references, deadlines.
          </div>
          <label>
            What do you need, exactly?
            <textarea
              value={request}
              onChange={(e) => setRequest(e.target.value)}
              rows={5}
              maxLength={4000}
              required
              placeholder="For example: 1 character, 4 directions (up, down, left, right), walk cycle of 8 frames each, 32×32 pixels, transparent PNG sheets, palette like the attached reference."
            />
            <span className="hint">
              This becomes part of the job's record. Not sure how to word it? Use <b>Ask about it first</b>: {p.name} reviews it, can open a
              conversation to talk it through, and then sends an offer.
            </span>
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
          <label className="check">
            <input type="checkbox" checked={exact} onChange={(e) => setExact(e.target.checked)} required /> I understand: {p.name} will
            deliver exactly what the agreement says, nothing more or less, and changes later are new work with a new price.
          </label>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <div className="row-actions">
            <button
              type="button"
              className="btn"
              disabled={busy || !chosenProject || !exact || !request.trim() || (!adultConfirmed && !adult)}
              onClick={() => void send(true)}
            >
              Ask about it first
            </button>
            <button className="btn slime" disabled={busy || !chosenProject}>
              Send request
            </button>
            <button type="button" className="btn" onClick={onClose}>
              Cancel
            </button>
          </div>
          <p className="muted small">
            Nothing is paid until you accept their offer. You can ask {p.name} for a conversation at any time; they decide when to open it, and
            it's recorded so you both have proof of what was said.
          </p>
        </>
      )}
    </form>
  );
}
