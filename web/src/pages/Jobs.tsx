// People jobs: the list of yours, and one job's page (DESIGN §15.2). Each page says what's
// happening in one sentence and offers only the next step, so neither side has to learn a
// process. The notes here are the contract's record, not a chat (LEGAL-GUIDANCE HUM-17).

import { useEffect, useState, type FormEvent } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { api, ApiError, type Job, type JobCard, type Offer } from "../api";
import { AI_USE } from "../components/People";
import { ErrorState, ExternalLink, Loading, SectionHeader } from "../components/ui";
import { formatDate, money, toMinor } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { renderMarkdown } from "../markdown";
import { isDesktop, openExternal } from "../platform";
import { useSession } from "../session";

const STATUS_LABEL: Record<Job["status"], string> = {
  requested: "Requested",
  offered: "Offer sent",
  awaiting_payment: "Waiting for payment",
  in_progress: "In progress",
  delivered: "Delivered",
  completed: "Completed",
  declined: "Declined",
  cancelled: "Cancelled",
  withdrawn: "Withdrawn",
};

export function MyJobs() {
  useDocumentTitle("Your jobs");
  const { me } = useSession();
  const jobs = useLoad(() => (me ? api.people.jobs() : Promise.resolve(undefined)), [me?.id]);
  if (me === null) {
    return (
      <div className="page">
        <div className="empty-state">Sign in to see your jobs.</div>
      </div>
    );
  }
  if (jobs.error) return <ErrorState error={jobs.error} onRetry={jobs.reload} />;
  if (!jobs.data) return <Loading />;
  const { as_client, as_creative } = jobs.data;
  return (
    <div className="page narrow">
      <div className="crumbs">
        <Link to="/people">People</Link> › Your jobs
      </div>
      <h1>Your jobs</h1>
      {as_client.length === 0 && as_creative.length === 0 && (
        <div className="empty-state">
          <p>No jobs yet.</p>
          <Link className="btn slime" to="/people">
            Find someone to hire
          </Link>
        </div>
      )}
      {as_client.length > 0 && <JobList title="Hiring" sub="People working on your projects" jobs={as_client} />}
      {as_creative.length > 0 && <JobList title="Working on" sub="Jobs you're doing for makers" jobs={as_creative} />}
    </div>
  );
}

function JobList({ title, sub, jobs }: { title: string; sub: string; jobs: JobCard[] }) {
  return (
    <section>
      <SectionHeader title={title} sub={sub} />
      <ul className="job-list">
        {jobs.map((j) => (
          <li key={j.id}>
            <Link className={`panel job-row ${j.needs_you ? "needs-you" : ""}`} to={`/jobs/${j.id}`}>
              <div>
                <b>{j.title}</b>
                <div className="muted small">
                  {j.other_name} · {j.skill.name} · for {j.project_title}
                  {j.price && ` · ${j.price}`}
                </div>
                <div className="small">{j.status_text}</div>
              </div>
              <div className="job-row-side">
                {j.needs_you && <span className="badge official">YOUR TURN</span>}
                <span className="badge neutral">{STATUS_LABEL[j.status].toUpperCase()}</span>
              </div>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function JobPage() {
  const { id = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const loaded = useLoad(() => api.people.job(id), [id]);
  const [job, setJob] = useState<Job>();
  const j = job ?? loaded.data;
  useDocumentTitle(j?.offer?.title ?? "Job");

  // Back from Stripe: read the payment straight away instead of waiting for Stripe's webhook.
  useEffect(() => {
    if (!params.get("paid")) return;
    void api.people.checkPayment(id).then(setJob, () => {});
    setParams({}, { replace: true });
  }, [id]);

  if (loaded.error) return <ErrorState error={loaded.error} onRetry={loaded.reload} />;
  if (!j) return <Loading />;
  return <JobView j={j} onChange={setJob} />;
}

function JobView({ j, onChange }: { j: Job; onChange: (j: Job) => void }) {
  const { me } = useSession();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [awaitingStripe, setAwaitingStripe] = useState(false);
  const isClient = j.role === "client";
  const can = (a: Job["actions"][number]) => j.actions.includes(a);
  const due = j.payments.find((p) => p.status === "due");

  const run = async (step: () => Promise<Job>) => {
    setBusy(true);
    setError(undefined);
    try {
      onChange(await step());
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const step = (name: string, body?: unknown) => run(() => api.people.step(j.id, name, body));
  const pay = () =>
    run(async () => {
      const res = await api.people.pay(j.id);
      if (res.url) {
        if (isDesktop) {
          void openExternal(res.url);
          setAwaitingStripe(true);
        } else {
          window.location.assign(res.url);
        }
      }
      return res.job;
    });

  return (
    <div className="page narrow job-page">
      <div className="crumbs">
        <Link to="/people">People</Link> › <Link to="/people/jobs">Your jobs</Link> › {j.offer?.title ?? "Request"}
      </div>
      <div className="job-head">
        <div>
          <h1>{j.offer?.title ?? `${j.skill.name} for ${j.project.title}`}</h1>
          <div className="muted">
            {isClient ? (
              <>
                With <Link to={`/people/${j.creative.handle}`}>{j.creative.name}</Link>
              </>
            ) : (
              <>For {j.client_name}</>
            )}{" "}
            · {j.skill.name} · project <Link to={`/app/${j.project.slug}`}>{j.project.title}</Link>
          </div>
        </div>
        <span className="badge neutral">{STATUS_LABEL[j.status].toUpperCase()}</span>
      </div>

      <div className="panel panel-pad next-step" role="status">
        <b>{j.status_text}</b>
        {j.auto_complete_at && isClient && (
          <div className="muted small">If you don't answer by {formatDate(j.auto_complete_at)}, it counts as accepted.</div>
        )}
        {j.completed_automatically && <div className="muted small">Completed automatically 7 days after delivery.</div>}
        {j.credited && (
          <div className="small">
            ✋ Credited on <Link to={`/app/${j.project.slug}`}>{j.project.title}</Link>'s page.
          </div>
        )}
        {awaitingStripe && (
          <div className="row-actions">
            <span className="muted small">Finish paying in your browser, then:</span>
            <button className="btn small" onClick={() => void run(() => api.people.checkPayment(j.id))}>
              I've paid
            </button>
          </div>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>

      {/* The creative's next step */}
      {can("offer") && <OfferForm j={j} busy={busy} onSend={(o) => step("offer", o)} />}
      {can("deliver") && <DeliverForm busy={busy} onSend={(body) => step("deliver", body)} />}

      {/* The client's next step */}
      {j.status === "offered" && j.offer && (
        <OfferSummary
          j={j}
          busy={busy}
          adultConfirmed={Boolean(me?.adult_confirmed)}
          onAccept={(body) => step("accept", body)}
          onChanges={(note) => step("changes", { note })}
        />
      )}
      {can("pay") && due && (
        <div className="panel panel-pad pay-box">
          <button className="btn slime big" disabled={busy || j.payment_mode === "off"} onClick={() => void pay()}>
            {j.status === "delivered" ? "Looks good — pay " : "Pay "}
            {money(due.amount_minor, due.currency)}
          </button>
          <p className="muted small">
            {j.payment_mode === "simulated"
              ? "Payments are simulated on this server: no money moves."
              : j.payment_mode === "off"
                ? "Payments aren't switched on yet on this server."
                : `Paid directly to ${j.creative.name} through Stripe.`}{" "}
            Your contract is with {j.creative.name}, not Slop. Slop doesn't settle disputes; your rights are against {j.creative.name}.
          </p>
        </div>
      )}
      {can("complete") && (
        <button className="btn slime big" disabled={busy} onClick={() => void step("complete")}>
          Looks good
        </button>
      )}
      {can("revision") && <NoteAction label={`Ask for a revision (${j.revisions_left} left)`} busy={busy} onSend={(note) => step("revision", { note })} required />}
      {can("review") && <ReviewForm j={j} busy={busy} onSend={(body) => step("review", body)} />}

      <div className="row-actions job-secondary">
        {can("decline") && <NoteAction label="Decline" busy={busy} onSend={(note) => step("decline", { note })} danger />}
        {can("cancel") && (
          <NoteAction
            label="Cancel the job"
            busy={busy}
            onSend={(note) => step("cancel", { note })}
            danger
            required={!isClient}
            hint={isClient ? undefined : "Say why. Refund what's fair from your Stripe dashboard."}
          />
        )}
        {can("withdraw") && j.withdrawal && (
          <button
            className="btn danger"
            disabled={busy}
            onClick={() => {
              if (window.confirm("Withdraw from this contract? The creative refunds what's owed within 14 days.")) void step("withdraw");
            }}
          >
            Withdraw from this contract
          </button>
        )}
      </div>
      {j.withdrawal?.open && isClient && (
        <p className="muted small">
          You can withdraw from this contract until {formatDate(j.withdrawal.until)} (your legal cancellation right).
          {j.withdrawal.started_early && " You asked for work to start straight away, so you'd pay for work already done."}
        </p>
      )}

      <div className="two-col job-body">
        <div>
          <SectionHeader title="What was asked" />
          <div className="panel panel-pad">
            <p className="pre">{j.request_md}</p>
            {j.wanted_by && <div className="muted small">Hoped for by {j.wanted_by}</div>}
          </div>
          {j.offer && j.status !== "offered" && (
            <>
              <SectionHeader title="What was agreed" />
              <ul className="panel panel-pad summary-list">
                {j.summary.map((l) => (
                  <li key={l}>{l}</li>
                ))}
              </ul>
            </>
          )}
          {j.agreement && j.agreement.accepted_at && (
            <details className="panel panel-pad agreement">
              <summary>
                Service Agreement · accepted {formatDate(j.agreement.accepted_at)} · template {j.agreement.template}
              </summary>
              <div className="prose" dangerouslySetInnerHTML={{ __html: renderMarkdown(j.agreement.text) }} />
              <div className="faint small">SHA-256 {j.agreement.sha256}</div>
            </details>
          )}
          <SectionHeader title="Timeline" />
          <ol className="timeline">
            {j.events.map((e, i) => (
              <li key={i}>
                <span className="faint small">{formatDate(e.created_at)}</span> <b>{e.actor ?? "Slop"}</b> {eventWords(e.kind)}
                {e.note && <div className="pre small">{e.note}</div>}
                {e.links.map((l) => (
                  <div key={l} className="small">
                    <ExternalLink href={l}>{l}</ExternalLink>
                  </div>
                ))}
              </li>
            ))}
          </ol>
        </div>
        <aside className="side-col">
          {j.payments.length > 0 && (
            <div className="panel panel-pad">
              <b>Payments</b>
              <table className="pay-table">
                <tbody>
                  {j.payments.map((p) => (
                    <tr key={p.part}>
                      <td>{p.part === "deposit" ? "First half" : p.part === "final" ? "Second half" : "Payment"}</td>
                      <td>{money(p.amount_minor, p.currency)}</td>
                      <td className={p.status === "paid" ? "good" : p.status === "due" ? "warn" : "muted"}>
                        {p.status === "paid" ? `Paid${p.simulated ? " (simulated)" : ""}` : p.status === "due" ? "Due" : p.due_on === "delivery" ? "On delivery" : "Waiting"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {j.refunded_minor > 0 && j.offer && <div className="small">Refunded {money(j.refunded_minor, j.offer.currency)}</div>}
              {j.creative_receives_minor !== null && j.offer && (
                <div className="muted small">
                  You receive about {money(j.creative_receives_minor, j.offer.currency)} after Slop's 5% fee; Stripe's card fee also comes out.
                </div>
              )}
            </div>
          )}
          <div className="panel panel-pad small muted">
            <b className="text">{j.creative.trader ? `${j.creative.name} works as a business` : `${j.creative.name} works as a private individual`}</b>
            <p>
              {j.creative.trader
                ? "Consumer law applies if you're hiring as a consumer."
                : "Consumer protection law (like a right to cancel) usually doesn't apply."}
            </p>
            <p>AI use: {AI_USE[j.offer?.ai_use ?? j.creative.ai_use].label}.</p>
          </div>
        </aside>
      </div>
    </div>
  );
}

function eventWords(kind: string): string {
  return (
    {
      requested: "asked for work",
      offered: "sent an offer",
      changes_requested: "asked for changes to the offer",
      accepted: "accepted the offer",
      paid: "paid",
      delivered: "delivered",
      revision_requested: "asked for a revision",
      completed: "accepted the work",
      completed_automatically: "completed the job (7 days after delivery)",
      declined: "declined",
      cancelled: "cancelled the job",
      withdrawn: "withdrew from the contract",
      refunded: "recorded a refund from Stripe",
      disputed: "recorded a card dispute",
    }[kind] ?? kind
  );
}

function OfferSummary({
  j,
  busy,
  adultConfirmed,
  onAccept,
  onChanges,
}: {
  j: Job;
  busy: boolean;
  adultConfirmed: boolean;
  onAccept: (body: { offer_count: number; start_now: boolean; confirm_adult: boolean }) => void;
  onChanges: (note: string) => void;
}) {
  const [startNow, setStartNow] = useState(false);
  const [adult, setAdult] = useState(false);
  const o = j.offer!;
  const isClient = j.role === "client";
  const first = o.payment === "split" ? Math.ceil(o.price_minor / 2) : o.payment === "upfront" ? o.price_minor : 0;
  return (
    <div className="panel panel-pad offer-summary">
      <SectionHeader title="The offer, in plain words" />
      <ul className="summary-list">
        {j.summary.map((l) => (
          <li key={l}>{l}</li>
        ))}
      </ul>
      {o.scope_md && <div className="prose small" dangerouslySetInnerHTML={{ __html: renderMarkdown(o.scope_md) }} />}
      {j.agreement && (
        <details className="agreement">
          <summary>Read the full Service Agreement</summary>
          <div className="prose" dangerouslySetInnerHTML={{ __html: renderMarkdown(j.agreement.text) }} />
        </details>
      )}
      {isClient && (
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            onAccept({ offer_count: j.offer_count, start_now: startNow, confirm_adult: adult });
          }}
        >
          {j.withdrawal_days !== null && (
            <label className="check">
              <input type="checkbox" checked={startNow} onChange={(e) => setStartNow(e.target.checked)} /> Start now. I understand I lose my
              right to cancel once the work is fully done.
            </label>
          )}
          {j.withdrawal_days !== null && !startNow && (
            <p className="hint">Without this, {j.creative.name} waits until your {j.withdrawal_days}-day cancellation period ends before starting.</p>
          )}
          {!adultConfirmed && (
            <label className="check">
              <input type="checkbox" checked={adult} onChange={(e) => setAdult(e.target.checked)} required /> I'm 18 or older
            </label>
          )}
          <div className="row-actions">
            <button className="btn slime big" disabled={busy}>
              {first > 0 ? `Accept & pay ${money(first, o.currency)}` : "Accept (pay on delivery)"}
            </button>
            <NoteAction label="Ask for changes" busy={busy} onSend={onChanges} required />
          </div>
          <p className="muted small">Pressing Accept is your signature on the agreement above.</p>
        </form>
      )}
    </div>
  );
}

function OfferForm({ j, busy, onSend }: { j: Job; busy: boolean; onSend: (o: Offer) => void }) {
  const prev = j.offer;
  const inTwoWeeks = new Date(Date.now() + 14 * 86400_000).toISOString().slice(0, 10);
  const [o, setO] = useState<Offer>(
    prev ?? {
      title: "",
      scope_md: "",
      price_minor: 0,
      currency: "usd",
      payment: "split",
      due_date: j.wanted_by ?? inTwoWeeks,
      revisions: 1,
      rights: "transfer",
      open_source: false,
      credit: true,
      ai_use: j.creative.ai_use,
      refund_policy: "",
    },
  );
  const [price, setPrice] = useState(prev ? String(prev.price_minor / 100) : "");
  const [error, setError] = useState<string>();
  const set = <K extends keyof Offer>(k: K, v: Offer[K]) => setO((x) => ({ ...x, [k]: v }));
  const minor = toMinor(price);
  const lastNote = [...j.events].reverse().find((e) => e.kind === "changes_requested");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (minor === null) return setError("Write the price as a number, like 120 or 120.50.");
    setError(undefined);
    onSend({ ...o, price_minor: minor });
  };
  return (
    <form className="panel panel-pad form offer-form" onSubmit={submit}>
      <h2>{prev ? "Send a new offer" : "Send an offer"}</h2>
      {lastNote?.note && j.status === "requested" && (
        <div className="notice">
          <b>They asked for changes:</b> {lastNote.note}
        </div>
      )}
      <label>
        What they'll get
        <input value={o.title} maxLength={100} required onChange={(e) => set("title", e.target.value)} placeholder="Knight sprite sheet, 4 directions" />
      </label>
      <label>
        Details <span className="hint">(optional: sizes, formats, what's included)</span>
        <textarea value={o.scope_md} maxLength={4000} rows={4} onChange={(e) => set("scope_md", e.target.value)} />
      </label>
      <div className="form-row">
        <label>
          Price
          <input inputMode="decimal" value={price} required onChange={(e) => setPrice(e.target.value)} placeholder="120" />
        </label>
        <label>
          Currency
          <select value={o.currency} onChange={(e) => set("currency", e.target.value as Offer["currency"])}>
            <option value="usd">USD $</option>
            <option value="eur">EUR €</option>
            <option value="gbp">GBP £</option>
          </select>
        </label>
        <label>
          Due
          <input type="date" value={o.due_date} required onChange={(e) => set("due_date", e.target.value)} />
        </label>
        <label>
          Revisions
          <input type="number" min={0} max={10} value={o.revisions} onChange={(e) => set("revisions", Number(e.target.value))} />
        </label>
      </div>
      {minor !== null && minor > 0 && (
        <p className="hint">
          They pay {money(minor, o.currency)}. You receive about {money(minor - Math.round(minor * 0.05), o.currency)} after Slop's 5% fee
          (Stripe's card fee also comes out).
        </p>
      )}
      <fieldset className="choice-list">
        <legend>When they pay</legend>
        {(
          [
            ["split", "Half now, half on delivery", "Shares the risk. Most common."],
            ["upfront", "All up front", "You start once it's paid."],
            ["on_delivery", "All on delivery", "They pay when they've seen the work."],
          ] as const
        ).map(([v, label, hint]) => (
          <label className="check" key={v}>
            <input type="radio" name="payment" checked={o.payment === v} onChange={() => set("payment", v)} />
            <span>
              <b>{label}</b> <span className="hint">{hint}</span>
            </span>
          </label>
        ))}
      </fieldset>
      <fieldset className="choice-list">
        <legend>Who owns the work</legend>
        <label className="check">
          <input type="radio" name="rights" checked={o.rights === "transfer"} onChange={() => set("rights", "transfer")} />
          <span>
            <b>They own it</b> <span className="hint">Copyright transfers to them once it's fully paid. You can still show it in your portfolio.</span>
          </span>
        </label>
        <label className="check">
          <input type="radio" name="rights" checked={o.rights === "licence"} onChange={() => set("rights", "licence")} />
          <span>
            <b>Licence for this project</b> <span className="hint">You keep the copyright; they can use and change it in {j.project.title}, forever.</span>
          </span>
        </label>
      </fieldset>
      <label className="check">
        <input type="checkbox" checked={o.open_source} onChange={(e) => set("open_source", e.target.checked)} /> It may be published under the
        project's open-source licence (anyone could reuse it)
      </label>
      <label className="check">
        <input type="checkbox" checked={o.credit} onChange={(e) => set("credit", e.target.checked)} /> Credit me on the project's page
      </label>
      <label>
        AI use for this job
        <select value={o.ai_use} onChange={(e) => set("ai_use", e.target.value as Offer["ai_use"])}>
          {(Object.keys(AI_USE) as Array<keyof typeof AI_USE>).map((k) => (
            <option key={k} value={k}>
              {AI_USE[k].label}: {AI_USE[k].hint}
            </option>
          ))}
        </select>
      </label>
      <label>
        Refunds <span className="hint">(optional; default: full refund before work starts, then for parts not yet delivered)</span>
        <input value={o.refund_policy} maxLength={500} onChange={(e) => set("refund_policy", e.target.value)} />
      </label>
      {error && <p className="form-error">{error}</p>}
      <div className="row-actions">
        <button className="btn slime" disabled={busy}>
          Send offer
        </button>
      </div>
      <p className="muted small">Sending it is your signature on the Service Agreement they'll see.</p>
    </form>
  );
}

function DeliverForm({ busy, onSend }: { busy: boolean; onSend: (body: { note: string; links: string[] }) => void }) {
  const [note, setNote] = useState("");
  const [links, setLinks] = useState("");
  return (
    <form
      className="panel panel-pad form"
      onSubmit={(e) => {
        e.preventDefault();
        onSend({ note, links: links.split(/\s+/).filter(Boolean) });
      }}
    >
      <h2>Deliver the work</h2>
      <label>
        Links to the files <span className="hint">(one per line, https://, up to 5: a drive folder, a pull request…)</span>
        <textarea value={links} rows={3} onChange={(e) => setLinks(e.target.value)} />
      </label>
      <label>
        Note <span className="hint">(optional)</span>
        <textarea value={note} rows={3} maxLength={2000} onChange={(e) => setNote(e.target.value)} />
      </label>
      <div className="row-actions">
        <button className="btn slime" disabled={busy}>
          Mark delivered
        </button>
      </div>
    </form>
  );
}

function ReviewForm({ j, busy, onSend }: { j: Job; busy: boolean; onSend: (body: { vote: string; body: string }) => void }) {
  const [vote, setVote] = useState<"not_slop" | "slop" | undefined>(j.review?.vote);
  const [body, setBody] = useState(j.review?.body ?? "");
  return (
    <form
      className="panel panel-pad form"
      onSubmit={(e) => {
        e.preventDefault();
        if (vote) onSend({ vote, body });
      }}
    >
      <h2>{j.review ? "Your review" : `Was ${j.creative.name}'s work slop?`}</h2>
      <div className="vote-row">
        <button type="button" className={`btn vote ${vote === "not_slop" ? "on up" : ""}`} aria-pressed={vote === "not_slop"} onClick={() => setVote("not_slop")}>
          👍 Not Slop
        </button>
        <button type="button" className={`btn vote ${vote === "slop" ? "on down" : ""}`} aria-pressed={vote === "slop"} onClick={() => setVote("slop")}>
          👎 Slop
        </button>
      </div>
      <label>
        A few words <span className="hint">(optional, public: quality, communication, deadlines, honesty about AI use)</span>
        <textarea value={body} rows={3} maxLength={2000} onChange={(e) => setBody(e.target.value)} />
      </label>
      {j.review?.reply && (
        <div className="review-reply">
          <b>{j.creative.name} replied</b> {j.review.reply}
        </div>
      )}
      <div className="row-actions">
        <button className="btn slime" disabled={busy || !vote}>
          {j.review ? "Update review" : "Post review"}
        </button>
      </div>
    </form>
  );
}

function NoteAction({
  label,
  busy,
  onSend,
  danger = false,
  required = false,
  hint,
}: {
  label: string;
  busy: boolean;
  onSend: (note: string) => void;
  danger?: boolean;
  required?: boolean;
  hint?: string;
}) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  if (!open)
    return (
      <button type="button" className={`btn ${danger ? "danger" : ""}`} disabled={busy} onClick={() => setOpen(true)}>
        {label}
      </button>
    );
  return (
    <div className="note-action form">
      <label>
        {label}
        <textarea
          value={note}
          rows={3}
          maxLength={2000}
          autoFocus
          onChange={(e) => setNote(e.target.value)}
          placeholder={required ? "Say what you need" : "Optional note"}
        />
        {hint && <span className="hint">{hint}</span>}
      </label>
      <div className="row-actions">
        <button
          type="button"
          className={`btn ${danger ? "danger" : "slime"}`}
          disabled={busy || (required && !note.trim())}
          onClick={() => {
            onSend(note);
            setOpen(false);
            setNote("");
          }}
        >
          {label}
        </button>
        <button type="button" className="btn" onClick={() => setOpen(false)}>
          Never mind
        </button>
      </div>
    </div>
  );
}
