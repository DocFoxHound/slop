// SPDX-License-Identifier: AGPL-3.0-or-later
// Open requests on People (DESIGN §15.9; crates/slop-api/src/people/work_requests.rs), People's
// second tab (/people?view=requests): a maker says what one of their store projects needs, and
// creatives who can be hired put their name forward. Newest first and nothing else; no budget (the
// creative sets the price in their quote). Nothing here ranks, matches or suggests anyone, and the
// board creates no job: the maker's "Ask … for a quote" opens the usual hire form filled in, with
// every tick still theirs.

import { useEffect, useMemo, useRef, useState, type FormEvent, type MouseEvent, type ReactNode, type Ref } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";

import {
  api,
  ApiError,
  type CreativeCard,
  type MyProfile,
  type MyWorkRequests,
  type Skill,
  type WorkRequest,
  type WorkRequestName,
  type WorkRequestOwner,
} from "../api";
import { HandoffError, HandoffNotice, HandoffWaiting, useHandoff } from "../components/Handoff";
import { PeopleNavActions, PersonCard, SkillButton } from "../components/People";
import { useMyPeoplePage } from "../components/PeopleStatus";
import { ReportButton } from "../components/Report";
import { useTermsGate } from "../components/Terms";
import { ErrorState, Loading } from "../components/ui";
import { formatDate, formatDay } from "../format";
import { REQUEST_LABELS, REQUEST_LIMITS, forgetHandoff, requestFill, type HandoffLink } from "../handoff";
import { useDocumentTitle, useLoad, type Loaded } from "../hooks";
import { renderMarkdown } from "../markdown";
import { isDesktop, openExternal } from "../platform";
import { useSession } from "../session";

/** Requests in one page of the board (work_requests.rs `PAGE`). */
const PAGE = 50;
/** Names put forward in one page of the maker's list (work_requests.rs `NAMES`). */
const NAMES = 100;
const NO_BUDGET = "There's no budget on a request: the creative sets the price in their quote.";
const requestPath = (id: string) => `/people?view=requests&request=${id}`;
/** Today as the API counts it (UTC), YYYY-MM-DD. */
const today = () => new Date().toISOString().slice(0, 10);
/** The last day a request can hope for the work (jobs.rs `parse_due`: two years ahead). */
const inTwoYears = () => new Date(Date.now() + 730 * 86_400_000).toISOString().slice(0, 10);
const message = (err: unknown) => (err instanceof ApiError ? err.message : String(err));

/**
 * /people?view=requests: the board, one request (&request=<id>), and the person's own requests.
 * Inside People's page, under its tabs (pages/People.tsx).
 */
export function OpenRequests() {
  const [params, setParams] = useSearchParams();
  const { me } = useSession();
  const mine = useMyPeoplePage();
  const skills = useLoad(() => api.people.skills(), []);
  const skill = params.get("skill") ?? "";
  const requestId = params.get("request");
  // A request prepared by the maker's AI assistant (/people?view=requests#fill=…, handoff.ts), and
  // the link the form has finished with (posted, cancelled or cleared), so it doesn't open the form
  // again, also after a look at one request and back.
  const { link, n: handoffN } = useHandoff("request", true);
  const [handoffDone, setHandoffDone] = useState<number>();
  const handoff = handoffDone === handoffN ? null : link;
  const my = useLoad(() => (me ? api.people.requests.mine() : Promise.resolve(null)), [me?.id]);
  // Bumped after posting, so the board shows the new request.
  const [fresh, setFresh] = useState(0);
  const pick = (slug: string | null) => {
    const next = new URLSearchParams(params);
    next.delete("request");
    if (slug && slug !== skill) next.set("skill", slug);
    else next.delete("skill");
    setParams(next, { replace: true });
  };
  const skillName = skills.data?.find((s) => s.slug === skill)?.name;

  return (
    <>
      <nav className="store-nav people-nav" aria-label="Open requests">
        <div className="skill-icons" role="group" aria-label="Skill">
          <SkillButton slug="" name="Every skill" on={!skill} onPick={() => pick(null)} />
          {skills.data?.map((s) => (
            <SkillButton key={s.slug} slug={s.slug} name={s.name} on={skill === s.slug} onPick={() => pick(s.slug)} />
          ))}
        </div>
        <PeopleNavActions mine={mine.data} />
      </nav>

      {requestId ? (
        <RequestDetail key={requestId} id={requestId} mine={mine.data} onChanged={my.reload} />
      ) : (
        <>
          {me === null && handoff && (
            <div className="request-signin">
              <HandoffWaiting link={handoff} />
              {isDesktop ? null : (
                <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent("/people?view=requests")}`}>
                  Sign in
                </Link>
              )}
            </div>
          )}
          <RequestBoard skill={skill} skillName={skillName} fresh={fresh} />
          {me && (
            <YourRequests
              my={my}
              handoff={handoff}
              handoffN={handoffN}
              onHandoffDone={() => setHandoffDone(handoffN)}
              skill={skills.data?.some((s) => s.slug === skill) ? skill : ""}
              skills={skills.data}
              adultConfirmed={me.adult_confirmed}
              onPosted={() => setFresh((n) => n + 1)}
            />
          )}
        </>
      )}
    </>
  );
}

/** The board: open requests, newest first (the only order), 50 at a time. */
function RequestBoard({ skill, skillName, fresh }: { skill: string; skillName?: string; fresh: number }) {
  useDocumentTitle("Open requests");
  const list = useLoad(() => api.people.requests.list({ skill: skill || undefined }), [skill, fresh]);
  // Pages after the first ("Show more requests"), and the size of the last one.
  const [extra, setExtra] = useState<WorkRequest[]>([]);
  const [last, setLast] = useState<number>();
  const [busy, setBusy] = useState(false);
  const [moreError, setMoreError] = useState<string>();
  // The first card "Show more" brought, which takes the focus (the button may be gone).
  const [focusAt, setFocusAt] = useState<number>();
  const firstNew = useRef<HTMLAnchorElement>(null);
  useEffect(() => {
    setExtra([]);
    setLast(undefined);
    setMoreError(undefined);
  }, [list.data]);
  useEffect(() => {
    if (focusAt !== undefined) firstNew.current?.focus();
  }, [focusAt]);
  const shown = list.data ? [...list.data, ...extra] : [];
  const full = (last ?? list.data?.length ?? 0) === PAGE;
  const showMore = async () => {
    setBusy(true);
    setMoreError(undefined);
    try {
      const page = await api.people.requests.list({ skill: skill || undefined, offset: shown.length });
      // Posted meanwhile: a request can come round twice; it's shown once.
      const seen = new Set(shown.map((r) => r.id));
      const added = page.filter((r) => !seen.has(r.id));
      setExtra((e) => [...e, ...added]);
      setLast(page.length);
      if (added.length > 0) setFocusAt(shown.length);
    } catch (err) {
      setMoreError(message(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section aria-labelledby="requests-title">
      <div className="section-h requests-head">
        <h2 id="requests-title">{skillName ? `Open requests: ${skillName}` : "Open requests"}</h2>
        <span className="sub requests-order">
          Newest first. Nothing else changes the order, and SLAPorSLOP doesn't suggest requests to anyone.
        </span>
      </div>
      <p className="muted">
        Makers say here what their projects need. If you have a People page, you can put your name forward. {NO_BUDGET}
      </p>
      {list.error ? (
        <ErrorState error={list.error} onRetry={list.reload} />
      ) : !list.data ? (
        <Loading />
      ) : shown.length === 0 ? (
        <div className="empty-state">{skillName ? `No open requests for ${skillName} right now.` : "No open requests right now."}</div>
      ) : (
        <>
          <ul className="request-list">
            {shown.map((r, i) => (
              <RequestCard key={r.id} r={r} linkRef={i === focusAt ? firstNew : undefined} />
            ))}
          </ul>
          {full && (
            <div className="more">
              <button type="button" className="btn" onClick={() => void showMore()} disabled={busy}>
                Show more requests
              </button>
            </div>
          )}
          {moreError && (
            <p className="form-error" role="alert">
              {moreError}
            </p>
          )}
        </>
      )}
    </section>
  );
}

/** "Pixel Art · for Rhea's Rooftops · posted by Rhea Requester on Nov 10, 2026". */
function RequestMeta({ r, until }: { r: WorkRequest; until?: boolean }) {
  return (
    <p className="muted small">
      {r.skill.name} · for <Link to={`/app/${r.project.slug}`}>{r.project.title}</Link> · posted by {r.maker} on {formatDate(r.created_at)}
      {until && ` · open until ${formatDate(r.expires_at)}`}
    </p>
  );
}

/**
 * One request on the board. Nothing on it is about any creative: no count, no price. The two
 * badges are about the viewer, and only they see them.
 */
function RequestCard({ r, level = 3, linkRef }: { r: WorkRequest; level?: 3 | 4; linkRef?: Ref<HTMLAnchorElement> }) {
  const H = level === 3 ? "h3" : "h4";
  return (
    <li className="panel request-card">
      <H>
        <Link to={requestPath(r.id)} ref={linkRef}>
          {r.title}
        </Link>
      </H>
      <RequestMeta r={r} />
      {r.wanted_by && <p className="small">Hoping to have it by {formatDay(r.wanted_by)}</p>}
      {(r.mine || r.put_forward) && (
        <p className="request-badges">
          {r.mine && <span className="badge neutral">YOUR REQUEST</span>}
          {r.put_forward && <span className="badge neutral">YOU PUT YOUR NAME FORWARD</span>}
        </p>
      )}
    </li>
  );
}

/** Links in the request's text leave SLAPorSLOP: the desktop app opens them in the browser. */
function onProseClick(e: MouseEvent) {
  const link = (e.target as HTMLElement).closest("a[data-external]");
  if (link) {
    e.preventDefault();
    void openExternal((link as HTMLAnchorElement).href);
  }
}

/** One request (&request=<id>): its whole text, and what this viewer can do about it. */
function RequestDetail({ id, mine, onChanged }: { id: string; mine: MyProfile | null | undefined; onChanged: () => void }) {
  const { me } = useSession();
  const req = useLoad(() => api.people.requests.one(id), [id, me?.id]);
  const r = req.data;
  useDocumentTitle(r?.title ?? "Open requests");
  const titleRef = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (r) titleRef.current?.focus();
  }, [r?.id]);
  // What the viewer just did, said in a status line that takes the focus once the request is back.
  const [said, setSaid] = useState<"put" | "taken">();
  const saidRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (said) saidRef.current?.focus();
  }, [said, r]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const changed = () => {
    req.reload();
    onChanged();
  };

  if (!r) {
    if (req.error?.status === 404) {
      return (
        <div className="empty-state">
          <p>This request isn't open any more.</p>
          <Link to="/people?view=requests">See open requests</Link>
        </div>
      );
    }
    return req.error ? <ErrorState error={req.error} onRetry={req.reload} /> : <Loading />;
  }

  const takeBack = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await api.people.requests.takeBack(r.id);
      setSaid("taken");
      changed();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const taken =
    said === "taken" && !r.put_forward ? (
      <p role="status" tabIndex={-1} ref={saidRef}>
        You took your name back.
      </p>
    ) : null;
  let action: ReactNode = null;
  if (r.owner) {
    action = <OwnerPanel r={r} owner={r.owner} onChanged={changed} />;
  } else if (me === null) {
    action = (
      <div className="request-signin">
        <p>Have a People page? Sign in to put your name forward.</p>
        {isDesktop ? null : (
          <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(requestPath(r.id))}`}>
            Sign in
          </Link>
        )}
      </div>
    );
  } else if (r.put_forward) {
    const named = r.put_forward.service_id;
    const pkg = named ? mine?.services.find((s) => s.id === named) : undefined;
    // The maker's list leaves out a page that isn't showing (hidden, or the account suspended).
    const showing = mine === undefined || mine?.status.state === "live";
    action = (
      <div className="request-put">
        <p role="status" tabIndex={-1} ref={said === "put" ? saidRef : undefined}>
          You put your name forward on {formatDate(r.put_forward.at)}
          {pkg ? `, naming your package “${pkg.name}”` : ""}.{" "}
          {showing
            ? "The maker sees a link to your People page."
            : "Your People page isn't showing right now, so the maker doesn't see your name."}
        </p>
        <button type="button" className="btn small ghost" onClick={() => void takeBack()} disabled={busy}>
          Take my name back
        </button>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    );
  } else if (!mine || mine.status.state !== "live") {
    // No People page that's live: they read the board, with nothing to explain.
    action = taken;
  } else if (!mine.taking_work || mine.stripe_status !== "active") {
    action = (
      <>
        {taken}
        <p className="muted">
          Your People page can't be hired right now, so you can't put your name forward yet. <Link to="/people/me">See what it needs</Link>.
        </p>
      </>
    );
  } else if (!mine.skills.includes(r.skill.slug)) {
    action = (
      <>
        {taken}
        <p className="muted">
          Your People page doesn't list {r.skill.name}. <Link to="/people/me">Add it to your page</Link> to put your name forward.
        </p>
      </>
    );
  } else {
    action = (
      <>
        {taken}
        <PutForward
          r={r}
          packages={mine.services.filter((s) => s.skill === r.skill.slug)}
          onDone={() => {
            setSaid("put");
            changed();
          }}
        />
      </>
    );
  }

  const ended = r.owner && r.owner.status !== "open";
  return (
    <article className="request-detail" aria-labelledby="request-title">
      <div className="crumbs">
        <Link to="/people?view=requests">Open requests</Link> › {r.skill.name}
      </div>
      <h2 id="request-title" className="request-title" tabIndex={-1} ref={titleRef}>
        {r.title}
      </h2>
      <RequestMeta r={r} until={!ended} />
      {r.wanted_by && <p>Hoping to have it by {formatDay(r.wanted_by)}</p>}
      {/* Rendered with raw HTML and pictures disabled; see markdown.ts. */}
      <div className="prose" onClick={onProseClick} dangerouslySetInnerHTML={{ __html: renderMarkdown(r.description_md) }} />
      <p className="muted small">{NO_BUDGET}</p>
      {action}
      {!r.mine && (
        <p className="request-report">
          <ReportButton kind="work_request" subject={r.id} label="this request" />
        </p>
      )}
    </article>
  );
}

/** A creative who can put their name forward: optionally naming one of their packages, nothing else. */
function PutForward({ r, packages, onDone }: { r: WorkRequest; packages: MyProfile["services"]; onDone: () => void }) {
  const terms = useTermsGate();
  const [serviceId, setServiceId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await terms.guard(() => api.people.requests.putForward(r.id, serviceId || null));
      onDone();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="form request-put" aria-label="Put your name forward" onSubmit={(e) => void submit(e)}>
      {terms.prompt}
      {packages.length > 0 && (
        <label>
          Name one of your packages (optional)
          <select value={serviceId} onChange={(e) => setServiceId(e.target.value)}>
            <option value="">No package</option>
            {packages.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="row-actions">
        <button className="btn slime" disabled={busy}>
          Put my name forward
        </button>
      </div>
      <p className="muted small">
        The maker sees a link to your People page, and the package if you name one. Nothing else is sent, and you can take it back.
      </p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

/** The hire button's own words: "Ask Vera for a quote", "Ask them" for a studio. */
const firstName = (c: CreativeCard) => (c.kind === "studio" ? "them" : c.name.split(" ")[0]);

/**
 * The maker on their own request: where it stands, closing it, and who put their name forward, in
 * the order they did (the usual People card: never sorted, filtered or badged by its rating).
 */
function OwnerPanel({ r, owner, onChanged }: { r: WorkRequest; owner: WorkRequestOwner; onChanged: () => void }) {
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [closed, setClosed] = useState(false);
  const closedRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (closed) closedRef.current?.focus();
  }, [closed, owner.status]);
  // More than the first 100 ("Show the next 100"), with the total as it was then; a reload of the
  // request starts again from the first 100.
  const [more, setMore] = useState<{ names: WorkRequestName[]; total: number }>();
  useEffect(() => setMore(undefined), [owner]);
  const names = more?.names ?? owner.responses;
  const total = more?.total ?? owner.responses_total;
  const [loading, setLoading] = useState(false);
  const moreRef = useRef<HTMLParagraphElement>(null);

  const close = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await api.people.requests.close(r.id);
      setClosed(true);
      onChanged();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };
  // The list is read again from the first name each time: places are counted among the names
  // shown now, so a name taken back (or back again) meanwhile would otherwise make one skip.
  const showNext = async () => {
    if (loading) return;
    setLoading(true);
    setError(undefined);
    try {
      const want = names.length + NAMES;
      const got: WorkRequestName[] = [];
      let latest = total;
      for (let offset = 0; offset < want; offset += NAMES) {
        const next = await api.people.requests.one(r.id, { responses_offset: offset });
        const page = next.owner?.responses ?? [];
        latest = next.owner?.responses_total ?? latest;
        got.push(...page);
        if (page.length < NAMES) break;
      }
      // Each person once, should one have moved between two pages.
      const seen = new Set<string>();
      const list: WorkRequestName[] = [];
      for (const x of got) {
        if (seen.has(x.person.handle)) continue;
        seen.add(x.person.handle);
        list.push(x);
      }
      setMore({ names: list, total: latest });
      // The button goes once they're all here: the count line takes the focus then.
      if (list.length >= latest) moreRef.current?.focus();
    } catch (err) {
      setError(message(err));
    } finally {
      setLoading(false);
    }
  };
  // The hire form, filled in from this request (the existing #fill= handoff). Only these fields:
  // every tick, and Send request, stay the maker's. The router state says where the fill came from.
  const ask = (x: WorkRequestName) => {
    const fields: Record<string, string> = {
      app_slug: r.project.slug,
      skill: r.skill.slug,
      request_md: `${r.title}\n\n${r.description_md}`,
    };
    if (r.wanted_by && r.wanted_by >= today()) fields.wanted_by = r.wanted_by;
    if (x.service) fields.service_id = x.service.id;
    navigate(
      { pathname: `/people/${x.person.handle}`, hash: `#fill=${encodeURIComponent(JSON.stringify(fields))}` },
      { state: { fromRequest: true } },
    );
  };

  const until = formatDate(r.expires_at);
  const where: Record<WorkRequestOwner["status"], ReactNode> = {
    open: owner.on_board ? (
      `Open until ${until}.`
    ) : (
      `Not showing on the board: its project isn't in the store right now. It shows again when the project does, until ${until}.`
    ),
    closed: `You closed this request on ${formatDate(owner.ended_at ?? r.expires_at)}.`,
    expired: `This request ran out on ${formatDate(owner.ended_at ?? r.expires_at)}.`,
    hidden: (
      <>
        SLAPorSLOP hid this request: {owner.hidden_reason}. <Link to="/account#decisions">See the decision</Link>, where you can appeal.
      </>
    ),
  };

  return (
    <div className="request-owner">
      <p className="request-state">{where[owner.status]}</p>
      {closed && (
        <p role="status" tabIndex={-1} ref={closedRef}>
          Request closed.
        </p>
      )}
      {owner.status === "open" && (
        <p>
          <button type="button" className="btn small ghost" onClick={() => void close()} disabled={busy}>
            Close request
          </button>
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <section aria-labelledby="names-title">
        <div className="section-h requests-head">
          <h3 id="names-title">Who put their name forward</h3>
          <span className="sub requests-order">In the order they did. SLAPorSLOP doesn't rank or recommend anyone here.</span>
        </div>
        {names.length === 0 ? (
          <p className="muted">{owner.status === "open" ? "Nobody yet. Requests stay up for 30 days." : "Nobody put their name forward."}</p>
        ) : (
          <ol className="request-names">
            {names.map((x) => (
              <li key={x.person.handle}>
                <PersonCard c={x.person} />
                <p className="muted small">
                  Put forward on {formatDate(x.at)}
                  {x.service ? ` · Package: ${x.service.name}` : ""}
                </p>
                {x.person.can_hire ? (
                  <button type="button" className="btn small" onClick={() => ask(x)}>
                    Ask {firstName(x.person)} for a quote
                  </button>
                ) : (
                  <span className="muted small">{x.person.name} isn't taking paid work right now.</span>
                )}
              </li>
            ))}
          </ol>
        )}
        {(more || total > names.length) && (
          <>
            <p className="muted small" role="status" tabIndex={-1} ref={moreRef}>
              Showing {names.length} of {total}, in the order they put their name forward.
            </p>
            {names.length < total && (
              // Not disabled while it loads, so the focus stays on it (amendment 14.8).
              <button type="button" className="btn small ghost" onClick={() => void showNext()} aria-disabled={loading}>
                Show the next 100
              </button>
            )}
          </>
        )}
      </section>
    </div>
  );
}

/**
 * Under the board, signed in: posting a request (when they have a project in the store and fewer
 * than three open), the requests they posted in every state, and those they put their name
 * forward for. Left out when there's nothing in it.
 */
function YourRequests({
  my,
  handoff,
  handoffN,
  onHandoffDone,
  skill,
  skills,
  adultConfirmed,
  onPosted,
}: {
  my: Loaded<MyWorkRequests | null>;
  /** The assistant's link, until the form has finished with it. */
  handoff: HandoffLink | null;
  handoffN: number;
  onHandoffDone: () => void;
  /** The board's skill filter, when it's one People has: the form starts with it. */
  skill: string;
  skills: Skill[] | undefined;
  adultConfirmed: boolean;
  onPosted: () => void;
}) {
  const { refresh } = useSession();
  const fields = handoff && "fields" in handoff;
  const [writing, setWriting] = useState(false);
  const [posted, setPosted] = useState(false);
  const postedRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (posted) postedRef.current?.focus();
  }, [posted]);
  // The focus follows the form: into it when "Post a request" opens it, back to that button when
  // it's cancelled (the button and the form take turns in one place).
  const [openedHere, setOpenedHere] = useState(false);
  const [cancelled, setCancelled] = useState(false);
  const postRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (cancelled && !writing) {
      postRef.current?.focus();
      setCancelled(false);
    }
  }, [cancelled, writing]);
  const d = my.data;
  const open = d ? d.posted.filter((p) => p.owner.status === "open").length : 0;
  const canPost = Boolean(d && d.projects.length > 0 && open < d.max_open);
  // A prepared request opens the form straight away, once one can be posted.
  useEffect(() => {
    if (fields && canPost) setWriting(true);
  }, [fields, canPost, handoffN]);

  if (my.error) return <ErrorState error={my.error} onRetry={my.reload} />;
  if (!d) return null;

  const finish = () => {
    forgetHandoff("request");
    onHandoffDone();
    setWriting(false);
    setOpenedHere(false);
  };
  let post: ReactNode = null;
  if (d.projects.length === 0) {
    if (fields) {
      post = (
        <p className="notice handoff-waiting" role="status">
          <b>Your AI assistant's request is ready.</b> A request is for a project whose page is in the store. Come back to this page once
          you have one, and the form opens with it filled in. It's kept in this browser tab only, for two hours.
        </p>
      );
    } else if (d.has_apps) {
      post = <p className="muted">A request is for a project whose page is in the store.</p>;
    }
  } else if (open >= d.max_open) {
    post = (
      <>
        <p className="muted">
          {open === d.max_open
            ? `You have ${open} open requests, the most at one time. Close one to post another.`
            : `You have ${open} open requests, and ${d.max_open} at one time is the most. Close some to post another.`}
        </p>
        {fields && (
          <p className="notice handoff-waiting" role="status">
            <b>Your AI assistant's request is ready.</b> It opens here, filled in, once you have fewer than {d.max_open} open requests. It's
            kept in this browser tab only, for two hours.
          </p>
        )}
      </>
    );
  } else if (writing) {
    post = (
      <RequestForm
        key={`request-${handoffN}`}
        projects={d.projects}
        skills={skills}
        initialSkill={skill}
        handoff={handoff}
        adultConfirmed={adultConfirmed}
        focusFirst={openedHere}
        onCleared={onHandoffDone}
        onPosted={() => {
          finish();
          setPosted(true);
          my.reload();
          onPosted();
          // Confirmed now, so the tick isn't asked for again.
          if (!adultConfirmed) void refresh();
        }}
        onCancel={() => {
          finish();
          setCancelled(true);
        }}
      />
    );
  } else {
    post = (
      <button
        type="button"
        className="btn slime"
        ref={postRef}
        onClick={() => {
          setPosted(false);
          setOpenedHere(true);
          setWriting(true);
        }}
      >
        Post a request
      </button>
    );
  }
  const unreadable = handoff && "error" in handoff;
  if (!post && !unreadable && !posted && d.posted.length === 0 && d.put_forward.length === 0) return null;

  return (
    <section className="your-requests" aria-labelledby="your-requests-title">
      <div className="section-h">
        <h2 id="your-requests-title">Your requests</h2>
      </div>
      {posted && (
        <p role="status" tabIndex={-1} ref={postedRef}>
          Your request is on the board.
        </p>
      )}
      {unreadable && (
        <HandoffError error={handoff.error} then="The form is empty: fill it in yourself, or ask your assistant for a new link." />
      )}
      {post}
      {d.posted.length > 0 && (
        <>
          <h3>Requests you posted</h3>
          <ul className="request-list">
            {d.posted.map((p) => (
              <PostedCard key={p.id} r={p} />
            ))}
          </ul>
        </>
      )}
      {d.put_forward.length > 0 && (
        <>
          <h3>Requests you put your name forward for</h3>
          <ul className="request-list">
            {d.put_forward.map((r) => (
              <RequestCard key={r.id} r={r} level={4} />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

/** One of the maker's own requests, in any state, with how many put their name forward. */
function PostedCard({ r }: { r: WorkRequest & { owner: WorkRequestOwner } }) {
  const o = r.owner;
  const ended = formatDate(o.ended_at ?? r.expires_at);
  const state: Record<WorkRequestOwner["status"], ReactNode> = {
    open: o.on_board ? `Open until ${formatDate(r.expires_at)}` : "Not showing on the board: its project isn't in the store right now.",
    closed: `Closed on ${ended}`,
    expired: `Ran out on ${ended}`,
    hidden: (
      <>
        Hidden by SLAPorSLOP. <Link to="/account#decisions">See the decision</Link>
      </>
    ),
  };
  const n = o.responses_total;
  return (
    <li className="panel request-card">
      <h4>
        <Link to={requestPath(r.id)}>{r.title}</Link>
      </h4>
      <p className="muted small">
        {r.skill.name} · for <Link to={`/app/${r.project.slug}`}>{r.project.title}</Link>
      </p>
      <p className="small">{state[o.status]}</p>
      <p className="small">{n === 0 ? "Nobody has put their name forward yet" : `${n.toLocaleString()} put their name forward`}</p>
    </li>
  );
}

/** Posting a request. A prepared one (handoff) fills it in; nothing is posted until Post request. */
function RequestForm({
  projects,
  skills,
  initialSkill,
  handoff,
  adultConfirmed,
  focusFirst,
  onCleared,
  onPosted,
  onCancel,
}: {
  projects: MyWorkRequests["projects"];
  skills: Skill[] | undefined;
  initialSkill: string;
  handoff: HandoffLink | null;
  adultConfirmed: boolean;
  /** Opened by its button: the focus moves to the form's heading. */
  focusFirst: boolean;
  /** "Clear the form": the assistant's link is done with. */
  onCleared: () => void;
  onPosted: () => void;
  onCancel: () => void;
}) {
  const terms = useTermsGate();
  const [project, setProject] = useState(projects.length === 1 ? projects[0].slug : "");
  const [skill, setSkill] = useState(initialSkill);
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [wantedBy, setWantedBy] = useState("");
  const [adult, setAdult] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ message: string; field?: string }>();
  const fieldRefs = {
    app_slug: useRef<HTMLSelectElement>(null),
    skill: useRef<HTMLSelectElement>(null),
    title: useRef<HTMLInputElement>(null),
    description_md: useRef<HTMLTextAreaElement>(null),
    wanted_by: useRef<HTMLInputElement>(null),
    confirm_adult: useRef<HTMLInputElement>(null),
  };
  // A refusal about one field marks it and takes the focus there.
  useEffect(() => {
    if (error?.field && error.field in fieldRefs) fieldRefs[error.field as keyof typeof fieldRefs].current?.focus();
  }, [error]);
  const headingRef = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (focusFirst) headingRef.current?.focus();
  }, []);
  const bad = (field: keyof typeof fieldRefs) =>
    error?.field === field ? { "aria-invalid": true as const, "aria-describedby": "request-form-error" } : {};

  // The assistant's request goes in once the projects and skills are known, only as the fields'
  // values. The 18+ tick is never ticked for them.
  const [cleared, setCleared] = useState(false);
  const fill = useMemo(
    () =>
      handoff && "fields" in handoff && !cleared && skills
        ? requestFill(handoff.fields, { projects: projects.map((p) => p.slug), skills: skills.map((s) => s.slug) })
        : undefined,
    [handoff, cleared, projects, skills],
  );
  const filledIn = useRef(false);
  useEffect(() => {
    if (!fill || filledIn.current) return;
    filledIn.current = true;
    const v = fill.values;
    if (v.app_slug !== undefined) setProject(v.app_slug);
    if (v.skill !== undefined) setSkill(v.skill);
    if (v.title !== undefined) setTitle(v.title);
    if (v.description_md !== undefined) setText(v.description_md);
    if (v.wanted_by !== undefined) setWantedBy(v.wanted_by);
    // In the form now: a reload doesn't fill it in again over what the person changed.
    forgetHandoff("request");
  }, [fill]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await terms.guard(() =>
        api.people.requests.post({
          app_slug: project,
          skill,
          title,
          description_md: text,
          wanted_by: wantedBy || null,
          confirm_adult: adult,
        }),
      );
      onPosted();
    } catch (err) {
      setError(err instanceof ApiError ? { message: err.message, field: err.field } : { message: String(err) });
      setBusy(false);
    }
  };

  return (
    <form className="panel panel-pad form request-form" aria-label="Post a request" onSubmit={(e) => void submit(e)}>
      {terms.prompt}
      <h3 tabIndex={-1} ref={headingRef}>
        Post a request
      </h3>
      {fill && (
        <HandoffNotice
          fill={fill}
          labels={REQUEST_LABELS}
          check="the title and what you need, since both are public"
          button="Post request"
          onClear={() => {
            setCleared(true);
            setProject(projects.length === 1 ? projects[0].slug : "");
            setSkill("");
            setTitle("");
            setText("");
            setWantedBy("");
            forgetHandoff("request");
            onCleared();
            // The notice and its button go: the focus moves to the first field.
            fieldRefs.app_slug.current?.focus();
          }}
          clearLabel="Clear the form"
        />
      )}
      <label>
        For which project?
        <select value={project} onChange={(e) => setProject(e.target.value)} required ref={fieldRefs.app_slug} {...bad("app_slug")}>
          {projects.length !== 1 && <option value="">Choose a project</option>}
          {projects.map((p) => (
            <option key={p.slug} value={p.slug}>
              {p.title}
            </option>
          ))}
        </select>
      </label>
      <label>
        Skill
        <select value={skill} onChange={(e) => setSkill(e.target.value)} required ref={fieldRefs.skill} {...bad("skill")}>
          <option value="">Choose a skill</option>
          {skills?.map((s) => (
            <option key={s.slug} value={s.slug}>
              {s.name}
            </option>
          ))}
        </select>
      </label>
      <label>
        Title
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          maxLength={REQUEST_LIMITS.title}
          required
          ref={fieldRefs.title}
          {...bad("title")}
        />
      </label>
      <label>
        What do you need?
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          maxLength={REQUEST_LIMITS.description_md}
          rows={8}
          required
          ref={fieldRefs.description_md}
          {...bad("description_md")}
        />
        <span className="hint">Up to 2,000 characters. Simple Markdown works. Anyone can read this.</span>
      </label>
      <label>
        Hoping to have it by <span className="hint">(optional)</span>
        <input
          type="date"
          value={wantedBy}
          min={today()}
          max={inTwoYears()}
          onChange={(e) => setWantedBy(e.target.value)}
          ref={fieldRefs.wanted_by}
          {...bad("wanted_by")}
        />
      </label>
      <p className="muted small">{NO_BUDGET}</p>
      <p className="muted small">
        Anyone can read this request, with your display name and your project. It stays up for 30 days unless you close it sooner.
      </p>
      {!adultConfirmed && (
        <label className="check">
          <input
            type="checkbox"
            checked={adult}
            onChange={(e) => setAdult(e.target.checked)}
            required
            ref={fieldRefs.confirm_adult}
            {...bad("confirm_adult")}
          />{" "}
          I'm 18 or older
        </label>
      )}
      {error && (
        <p className="form-error" role="alert" id="request-form-error">
          {error.message}
        </p>
      )}
      <div className="row-actions">
        <button className="btn slime" disabled={busy}>
          Post request
        </button>
        <button type="button" className="btn ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
