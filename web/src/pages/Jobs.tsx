// People jobs: the list of yours, and one job's page (DESIGN §15.2–15.6). Each page says what's
// happening in one sentence and offers only the next steps. The agreement, every Change to
// Agreement and the steps are the contract record; conversations are recorded too, and both
// sides can download their copy.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { api, ApiError, type ConversationCopy, type Job, type JobCard, type Offer } from "../api";
import { AI_USE } from "../components/People";
import { ReportButton } from "../components/Report";
import { ErrorState, ExternalLink, Loading, SectionHeader } from "../components/ui";
import { formatDate, money, toMinor } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { renderMarkdown } from "../markdown";
import { isDesktop, openExternal, saveFile } from "../platform";
import { useSession } from "../session";

const STATUS_LABEL: Record<Job["status"], string> = {
  requested: "Requested",
  offered: "Offer sent",
  awaiting_payment: "Waiting for payment",
  in_progress: "In progress",
  awaiting_sign_off: "Waiting for sign-off",
  delivered: "Delivered",
  final_files: "Final files next",
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
                {j.inquiry && j.status === "requested" && <span className="badge neutral">INQUIRY</span>}
                {j.conversation_requested && <span className="badge soon">WANTS TO TALK</span>}
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
  const openChange = j.changes.find((c) => c.status === "requested" || c.status === "offered");

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
  const downloadRecord = async () => {
    try {
      saveFile(`slop-job-${j.id}.md`, await api.people.record(j.id), "text/markdown");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };

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
            {j.due_date && <> · due {j.due_date}</>}
          </div>
        </div>
        <div className="job-head-side">
          <span className="badge neutral">{STATUS_LABEL[j.status].toUpperCase()}</span>
          <button type="button" className="link-btn small" onClick={() => void downloadRecord()}>
            Download the full record
          </button>
        </div>
      </div>

      <div className="panel panel-pad next-step" role="status">
        <b>{j.status_text}</b>
        {j.auto_complete_at && isClient && (
          <div className="muted small">If you don't answer by {formatDate(j.auto_complete_at)}, the delivery counts as accepted.</div>
        )}
        {j.completed_automatically && <div className="muted small">Accepted automatically 7 days after delivery.</div>}
        {j.credited && (
          <div className="small">
            ✋ Credited on <Link to={`/app/${j.project.slug}`}>{j.project.title}</Link>'s page.
          </div>
        )}
        {j.client_record && (
          <div className="muted small">
            This client: paid in full on {j.client_record.paid_in_full} of {j.client_record.completed_jobs} finished job
            {j.client_record.completed_jobs === 1 ? "" : "s"}
            {j.client_record.owing > 0 && <b className="bad"> · owes on {j.client_record.owing}</b>}.
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
      {can("send_for_sign_off") && (
        <FilesForm
          title="Send the direction for sign-off"
          hint="A sketch, a rough cut or a sample. Once signed off, changing the direction is a Change to Agreement."
          button="Send for sign-off"
          busy={busy}
          onSend={(body) => step("sign-off/send", body)}
        />
      )}
      {can("deliver") && (
        <FilesForm
          title="Deliver the work"
          hint={
            j.payments.some((p) => p.due_on === "delivery" && p.status !== "paid")
              ? "Money is still owed on delivery, so send a preview (watermarked or reduced). You send the final files after they pay."
              : "Everything is paid, so send the files."
          }
          button="Mark delivered"
          busy={busy}
          onSend={(body) => step("deliver", body)}
        />
      )}
      {can("final_files") && (
        <FilesForm
          title="Send the final files"
          hint="They've paid in full. Sending these completes the job."
          button="Send final files"
          busy={busy}
          onSend={(body) => step("final-files", body)}
        />
      )}

      {/* The client's next step */}
      {j.status === "offered" && j.offer && (
        <OfferSummary j={j} busy={busy} adultConfirmed={Boolean(me?.adult_confirmed)} onAccept={(body) => step("accept", body)} />
      )}
      {can("sign_off") && (
        <div className="panel panel-pad">
          <h2>Sign off on the direction</h2>
          <p className="muted">
            Look at what {j.creative.name} sent (in the timeline below). Signing off makes this direction part of the agreement; changing it
            afterwards is new work. If it doesn't match the agreement, ask for a change or open a problem instead.
          </p>
          <button className="btn slime" disabled={busy} onClick={() => void step("sign-off")}>
            Sign off on this direction
          </button>
        </div>
      )}
      {can("pay") && due && (
        <div className="panel panel-pad pay-box">
          <button className="btn slime big" disabled={busy || j.payment_mode === "off"} onClick={() => void pay()}>
            {j.status === "delivered" ? "Accept the delivery — pay " : "Pay "}
            {money(due.amount_minor, due.currency)}
          </button>
          <p className="muted small">
            {j.status === "delivered" && `Pay only if the preview matches the agreement; ${j.creative.name} then sends the final files. `}
            {j.payment_mode === "simulated"
              ? "Payments are simulated on this server: no money moves."
              : j.payment_mode === "off"
                ? "Payments aren't switched on yet on this server."
                : `Paid directly to ${j.creative.name} through Stripe.`}{" "}
            Your contract is with {j.creative.name}, not Slop. Your rights are against {j.creative.name}.
          </p>
        </div>
      )}
      {can("accept_delivery") && (
        <div className="panel panel-pad">
          <button className="btn slime big" disabled={busy} onClick={() => void step("accept-delivery")}>
            Accept the delivery
          </button>
          <p className="muted small">Accept only if it matches the agreement. Otherwise ask for a change or open a problem.</p>
        </div>
      )}
      {can("review") && <ReviewForm j={j} busy={busy} onSend={(body) => step("review", body)} />}

      <ChangesPanel j={j} busy={busy} step={step} openChange={openChange} />
      <ConversationPanel j={j} busy={busy} step={step} onError={setError} />
      <ProblemPanel j={j} busy={busy} step={step} />

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
          <SectionHeader title="What was asked" sub={j.inquiry ? "An inquiry: they asked for a review first" : undefined} />
          <div className="panel panel-pad">
            <p className="pre">{j.request_md}</p>
            {j.wanted_by && <div className="muted small">Hoped for by {j.wanted_by}</div>}
          </div>
          {j.offer && j.status !== "offered" && (
            <>
              <SectionHeader title="What was agreed" sub="Exactly this, nothing more or less" />
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
          {j.changes
            .filter((c) => c.status === "accepted" && c.text)
            .map((c) => (
              <details className="panel panel-pad agreement" key={c.id}>
                <summary>
                  Change to Agreement {c.number} · accepted {c.decided_at ? formatDate(c.decided_at) : ""}
                </summary>
                <div className="prose" dangerouslySetInnerHTML={{ __html: renderMarkdown(c.text ?? "") }} />
                <div className="faint small">SHA-256 {c.sha256}</div>
              </details>
            ))}
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
                  {j.payments.map((p, i) => (
                    <tr key={i}>
                      <td>
                        {p.part === "deposit"
                          ? "First half"
                          : p.part === "final"
                            ? "Second half"
                            : p.part === "change"
                              ? `Change ${p.change_number}`
                              : "Payment"}
                      </td>
                      <td>{money(p.amount_minor, p.currency)}</td>
                      <td className={p.status === "paid" ? "good" : p.status === "due" ? "warn" : "muted"}>
                        {p.status === "paid"
                          ? `Paid${p.simulated ? " (simulated)" : ""}`
                          : p.status === "due"
                            ? "Due"
                            : p.due_on === "delivery"
                              ? "On delivery"
                              : "Waiting"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {j.refunded_minor > 0 && j.offer && <div className="small">Refunded {money(j.refunded_minor, j.offer.currency)}</div>}
              {j.creative_receives_minor !== null && j.offer && (
                <div className="muted small">
                  You receive about {money(j.creative_receives_minor, j.offer.currency)} of the original price after Slop's 5% fee;
                  Stripe's card fee also comes out.
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
      inquiry: "asked for a review of their request before an offer",
      offered: "sent an offer",
      accepted: "accepted the offer (signed the agreement)",
      paid: "paid",
      sent_for_sign_off: "sent the direction for sign-off",
      signed_off: "signed off on the direction",
      delivered: "delivered",
      delivered_preview: "delivered a preview",
      accepted_delivery: "accepted the delivery",
      final_files: "sent the final files",
      completed: "accepted the delivery",
      completed_automatically: "completed the job (7 days after delivery)",
      change_requested: "asked for a change",
      change_offered: "sent a Change to Agreement",
      change_accepted: "accepted a Change to Agreement",
      change_declined: "declined the change",
      change_withdrawn: "withdrew the change",
      conversation_requested: "asked for a conversation",
      conversation_opened: "opened a conversation (recorded)",
      conversation_closed: "closed the conversation",
      conversation_submitted: "submitted their copy of the conversation",
      problem_opened: "opened a problem",
      problem_answered: "answered the problem",
      problem_proposal: "proposed an outcome",
      problem_proposal_accepted: "accepted a proposed outcome",
      problem_proposal_rejected: "rejected a proposed outcome",
      problem_proposal_withdrawn: "withdrew a proposed outcome",
      problem_withdrawn: "withdrew the problem",
      mediator_suggestion: "(mediator) suggested an outcome; it binds no one",
      problem_closed_unresolved: "(mediator) closed the problem unresolved",
      declined: "declined",
      cancelled: "cancelled the job",
      withdrawn: "withdrew from the contract",
      refunded: "recorded a refund",
      disputed: "recorded a card dispute",
      review_dispute_removed: "(mediator) removed the review",
      review_dispute_noted: "(mediator) added a note to the review",
      review_dispute_kept: "(mediator) kept the review",
    }[kind] ?? kind
  );
}

function ExactAck({ checked, onChange, text }: { checked: boolean; onChange: (v: boolean) => void; text: string }) {
  return (
    <label className="check exact-ack">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} required /> {text}
    </label>
  );
}

function OfferSummary({
  j,
  busy,
  adultConfirmed,
  onAccept,
}: {
  j: Job;
  busy: boolean;
  adultConfirmed: boolean;
  onAccept: (body: { offer_count: number; start_now: boolean; confirm_adult: boolean; exact_terms_ack: boolean }) => void;
}) {
  const [startNow, setStartNow] = useState(false);
  const [adult, setAdult] = useState(false);
  const [exact, setExact] = useState(false);
  const o = j.offer!;
  const isClient = j.role === "client";
  const first = o.payment === "split" ? Math.ceil(o.price_minor / 2) : o.payment === "upfront" ? o.price_minor : 0;
  return (
    <div className="panel panel-pad offer-summary">
      <SectionHeader title="The offer, in plain words" />
      {isClient && (
        <div className="exact-warning" role="note">
          <b>Read every word.</b> If you accept, you get exactly what this says: nothing more and nothing less. Anything that isn't written
          here isn't part of the work, and changes later are new work with a new price. If anything is unclear, ask for a conversation or
          decline and send a clearer request before accepting.
        </div>
      )}
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
            onAccept({ offer_count: j.offer_count, start_now: startNow, confirm_adult: adult, exact_terms_ack: exact });
          }}
        >
          <ExactAck checked={exact} onChange={setExact} text={j.exact_wording} />
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
            <button className="btn slime big" disabled={busy || !exact}>
              {first > 0 ? `Accept & pay ${money(first, o.currency)}` : "Accept (pay when you accept the delivery)"}
            </button>
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
      sign_off: false,
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
  const lastNote = [...j.events].reverse().find((e) => e.kind === "declined" && e.note);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (minor === null) return setError("Write the price as a number, like 120 or 120.50.");
    setError(undefined);
    onSend({ ...o, price_minor: minor });
  };
  return (
    <form className="panel panel-pad form offer-form" onSubmit={submit}>
      <h2>{prev ? "Send a new offer" : "Send an offer"}</h2>
      <p className="muted small">
        Write exactly what you'll deliver: the client gets this, nothing more or less, and changes later are new work you price separately.
        Not sure what they mean? Open a conversation first.
      </p>
      {lastNote?.note && (
        <div className="notice">
          <b>Note:</b> {lastNote.note}
        </div>
      )}
      <label>
        What they'll get
        <input value={o.title} maxLength={100} required onChange={(e) => set("title", e.target.value)} placeholder="Knight sprite sheet, 4 directions" />
      </label>
      <label>
        Exactly what's included <span className="hint">(sizes, quantities, formats, file types)</span>
        <textarea value={o.scope_md} maxLength={4000} rows={5} onChange={(e) => set("scope_md", e.target.value)} />
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
            ["split", "Half now, half when they accept the delivery", "Shares the risk. Most common."],
            ["upfront", "All up front", "You start once it's paid."],
            ["on_delivery", "All when they accept the delivery", "You deliver a preview first; final files after payment."],
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
      <label className="check">
        <input type="checkbox" checked={o.sign_off} onChange={(e) => set("sign_off", e.target.checked)} />
        <span>
          <b>Sign-off on the direction first</b>{" "}
          <span className="hint">You send a sketch or sample; once they sign off, changing the direction is new work.</span>
        </span>
      </label>
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

function FilesForm({
  title,
  hint,
  button,
  busy,
  onSend,
}: {
  title: string;
  hint: string;
  button: string;
  busy: boolean;
  onSend: (body: { note: string; links: string[] }) => void;
}) {
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
      <h2>{title}</h2>
      <p className="muted small">{hint}</p>
      <label>
        Links to the files <span className="hint">(one per line, https://, up to 5)</span>
        <textarea value={links} rows={3} onChange={(e) => setLinks(e.target.value)} />
      </label>
      <label>
        Note <span className="hint">(optional)</span>
        <textarea value={note} rows={3} maxLength={2000} onChange={(e) => setNote(e.target.value)} />
      </label>
      <div className="row-actions">
        <button className="btn slime" disabled={busy}>
          {button}
        </button>
      </div>
    </form>
  );
}

type Step = (name: string, body?: unknown) => Promise<void>;

function ChangesPanel({ j, busy, step, openChange }: { j: Job; busy: boolean; step: Step; openChange: Job["changes"][number] | undefined }) {
  const can = (a: Job["actions"][number]) => j.actions.includes(a);
  const [exact, setExact] = useState(false);
  const currency = j.offer?.currency ?? "usd";
  if (!openChange && !can("request_change") && !can("propose_change")) return null;
  return (
    <section className="panel panel-pad changes-panel">
      <h2>Changes to the agreement</h2>
      <p className="muted small">
        Any change after accepting is new work: {j.creative.name} writes a Change to Agreement with its own price (it can be zero) and date,
        and it only applies if {j.role === "client" ? "you accept" : "the client accepts"} it.
      </p>
      {openChange?.status === "requested" && (
        <div className="notice">
          <b>{j.role === "client" ? "You asked:" : "They asked:"}</b> {openChange.requested_note}
        </div>
      )}
      {openChange?.status === "offered" && (
        <div className="change-offer">
          <b>
            Change {openChange.number}: {openChange.title}
          </b>
          {openChange.scope_md && <div className="prose small" dangerouslySetInnerHTML={{ __html: renderMarkdown(openChange.scope_md) }} />}
          <ul className="summary-list">
            <li>
              Extra price:{" "}
              {openChange.extra_price_minor ? `${money(openChange.extra_price_minor, currency)}, paid when accepted` : "none"}
            </li>
            <li>New due date: {openChange.new_due_date}</li>
            <li>Anything delivered goes back to in progress; it's new work, delivered exactly as written.</li>
          </ul>
          {can("accept_change") && (
            <>
              <ExactAck checked={exact} onChange={setExact} text={`${j.exact_wording} This change too.`} />
              <div className="row-actions">
                <button className="btn slime" disabled={busy || !exact} onClick={() => void step("change/accept", { number: openChange.number, exact_terms_ack: true })}>
                  Accept this change
                  {openChange.extra_price_minor ? ` and pay ${money(openChange.extra_price_minor, currency)}` : ""}
                </button>
                <NoteAction label="Decline the change" busy={busy} onSend={(note) => step("change/decline", { note })} />
              </div>
            </>
          )}
          {can("withdraw_change") && <NoteAction label="Withdraw this change" busy={busy} onSend={(note) => step("change/decline", { note })} />}
        </div>
      )}
      {can("request_change") && (
        <NoteAction
          label="Ask for a change"
          busy={busy}
          required
          hint="Describe exactly what should change. They reply with a Change to Agreement (price and date) or decline."
          onSend={(note) => step("change/request", { note })}
        />
      )}
      {(can("offer_change") || can("propose_change")) && <ChangeOfferForm busy={busy} onSend={(c) => step("change/offer", c)} currency={currency} />}
      {can("decline_change") && j.role === "creative" && (
        <NoteAction
          label="Decline the change"
          busy={busy}
          hint="The work stays as agreed."
          onSend={(note) => step("change/decline", { note })}
        />
      )}
    </section>
  );
}

function ChangeOfferForm({
  busy,
  currency,
  onSend,
}: {
  busy: boolean;
  currency: string;
  onSend: (c: { title: string; scope_md: string; extra_price_minor: number; new_due_date: string }) => void;
}) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [scope, setScope] = useState("");
  const [price, setPrice] = useState("0");
  const [due, setDue] = useState(new Date(Date.now() + 14 * 86400_000).toISOString().slice(0, 10));
  if (!open)
    return (
      <button type="button" className="btn" disabled={busy} onClick={() => setOpen(true)}>
        Write a Change to Agreement
      </button>
    );
  const minor = toMinor(price);
  return (
    <form
      className="form"
      onSubmit={(e) => {
        e.preventDefault();
        if (minor !== null) onSend({ title, scope_md: scope, extra_price_minor: minor, new_due_date: due });
      }}
    >
      <label>
        What changes
        <input value={title} maxLength={100} required onChange={(e) => setTitle(e.target.value)} />
      </label>
      <label>
        Exactly what's included
        <textarea value={scope} rows={4} maxLength={4000} onChange={(e) => setScope(e.target.value)} />
      </label>
      <div className="form-row">
        <label>
          Extra price ({currency.toUpperCase()}, 0 for none)
          <input inputMode="decimal" value={price} onChange={(e) => setPrice(e.target.value)} />
        </label>
        <label>
          New due date
          <input type="date" value={due} required onChange={(e) => setDue(e.target.value)} />
        </label>
      </div>
      <div className="row-actions">
        <button className="btn slime" disabled={busy || minor === null}>
          Send the change
        </button>
        <button type="button" className="btn" onClick={() => setOpen(false)}>
          Never mind
        </button>
      </div>
    </form>
  );
}

function ConversationPanel({ j, busy, step, onError }: { j: Job; busy: boolean; step: Step; onError: (e: string) => void }) {
  const c = j.conversation;
  const can = (a: Job["actions"][number]) => j.actions.includes(a);
  const [text, setText] = useState("");
  const [ack, setAck] = useState(false);
  const [note, setNote] = useState("");
  const isClient = j.role === "client";
  const download = async () => {
    try {
      const copy: ConversationCopy = await api.people.conversationCopy(j.id);
      saveFile(`slop-conversation-${j.id}.json`, JSON.stringify(copy, null, 2), "application/json");
    } catch (err) {
      onError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <section className="panel panel-pad conversation">
      <div className="title-row spread">
        <h2>Conversation</h2>
        {c.messages.length > 0 && (
          <button type="button" className="link-btn small" onClick={() => void download()}>
            Download your copy
          </button>
        )}
      </div>
      <p className="recorded-notice" role="note">
        ● {c.notice}
      </p>
      {c.text_deleted_after && c.messages.some((m) => m.body) && (
        <p className="small warn">Closed. Slop deletes the text after {formatDate(c.text_deleted_after)}: download your copy before then.</p>
      )}
      {c.messages.length > 0 && (
        <ol className="messages">
          {c.messages.map((m) => (
            <li key={m.id} className={m.mine ? "mine" : ""}>
              <span className="faint small">
                {m.mine ? "You" : m.author} · {formatDate(m.created_at)}
                {!m.mine && m.body && (
                  <>
                    {" "}
                    · <ReportButton kind="message" subject={m.id} label="this message" className="link-btn small faint" />
                  </>
                )}
              </span>
              {m.body ? <p className="pre">{m.body}</p> : <p className="faint small">Deleted from Slop · fingerprint {m.sha256.slice(0, 12)}…</p>}
            </li>
          ))}
        </ol>
      )}
      {c.open && can("message") ? (
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void step("messages", { body: text, recording_ack: ack }).then(() => setText(""));
          }}
        >
          <label>
            Message
            <textarea value={text} rows={3} maxLength={4000} required onChange={(e) => setText(e.target.value)} />
          </label>
          {!c.acknowledged && (
            <label className="check">
              <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} required /> I understand this conversation is
              recorded.
            </label>
          )}
          <div className="row-actions">
            <button className="btn slime small" disabled={busy || (!c.acknowledged && !ack)}>
              Send
            </button>
            {can("close_conversation") && (
              <button type="button" className="btn small" disabled={busy} onClick={() => void step("conversation/close")}>
                Close the conversation
              </button>
            )}
          </div>
        </form>
      ) : (
        <>
          {!c.open && c.requested_at && !isClient && (
            <div className="notice">
              <b>The client asked to talk.</b> {c.request_note}
            </div>
          )}
          {!c.open && c.requested_at && isClient && (
            <p className="muted small">You asked for a conversation. {j.creative.name} decides when to open it.</p>
          )}
          {can("open_conversation") && (
            <button className="btn" disabled={busy} onClick={() => void step("conversation/open")}>
              Open a conversation
            </button>
          )}
          {can("request_conversation") && (
            <div className="form">
              <label>
                Ask {j.creative.name} for a conversation <span className="hint">(optional: what about)</span>
                <input value={note} maxLength={300} onChange={(e) => setNote(e.target.value)} />
              </label>
              <div className="row-actions">
                <button className="btn" disabled={busy} onClick={() => void step("conversation/request", { note })}>
                  Ask to talk
                </button>
              </div>
            </div>
          )}
        </>
      )}
    </section>
  );
}

function ProblemPanel({ j, busy, step }: { j: Job; busy: boolean; step: Step }) {
  const p = j.problem;
  const can = (a: Job["actions"][number]) => j.actions.includes(a);
  const [kind, setKind] = useState<"refund" | "cancel" | "redo" | "other">("refund");
  const [amount, setAmount] = useState("");
  const [note, setNote] = useState("");
  const [copyResult, setCopyResult] = useState<string>();
  if (!p && !can("open_problem")) return null;
  const submitCopy = async (file: File) => {
    try {
      const copy = JSON.parse(await file.text()) as ConversationCopy;
      const r = await api.people.submitCopy(j.id, copy);
      setCopyResult(`${r.verified_messages} messages match Slop's fingerprints, ${r.unverified_messages} don't.`);
    } catch (err) {
      setCopyResult(err instanceof ApiError ? err.message : "That isn't a conversation copy downloaded from Slop.");
    }
  };
  return (
    <section className="panel panel-pad problem-panel">
      <h2>Problem with this job</h2>
      {!p || p.status !== "open" ? (
        <>
          {p && (
            <p className="muted small">
              Last problem: {p.status}
              {p.closing_note && ` — ${p.closing_note}`}
            </p>
          )}
          {can("open_problem") && (
            <NoteAction
              label="Open a problem"
              busy={busy}
              required
              hint="State what's wrong, pointing at the agreement. The other side answers, and you both propose outcomes. A Slop mediator can suggest one, but only what you both agree to happens."
              onSend={(statement) => step("problem", { statement })}
            />
          )}
        </>
      ) : (
        <>
          <div className="notice">
            <b>{p.opened_by_me ? "You said:" : "They said:"}</b> {p.statement}
          </div>
          {p.reply ? (
            <div className="notice">
              <b>{p.opened_by_me ? "They answered:" : "You answered:"}</b> {p.reply}
            </div>
          ) : (
            !p.opened_by_me && <NoteAction label="Answer" busy={busy} required onSend={(statement) => step("problem/reply", { statement })} />
          )}
          {p.mediator_suggestion && (
            <div className="mediator-note">
              <b>A Slop mediator suggests</b> (this binds neither of you): {p.mediator_suggestion}
            </div>
          )}
          <ul className="proposals">
            {p.proposals.map((x) => (
              <li key={x.id}>
                <b>{x.mine ? "You" : "They"} proposed:</b> {x.kind}
                {x.amount_minor ? ` ${money(x.amount_minor, j.offer?.currency ?? "usd")}` : ""} — {x.note} <span className="faint">({x.status})</span>
                {x.status === "pending" && !x.mine && (
                  <span className="row-actions">
                    <button className="btn slime small" disabled={busy} onClick={() => void step(`problem/proposals/${x.id}/accept`)}>
                      Accept
                    </button>
                    <button className="btn small" disabled={busy} onClick={() => void step(`problem/proposals/${x.id}/reject`)}>
                      Reject
                    </button>
                  </span>
                )}
                {x.status === "pending" && x.mine && (
                  <button className="link-btn small" disabled={busy} onClick={() => void step(`problem/proposals/${x.id}/withdraw`)}>
                    Withdraw
                  </button>
                )}
              </li>
            ))}
          </ul>
          <form
            className="form"
            onSubmit={(e) => {
              e.preventDefault();
              void step("problem/propose", { kind, note, amount_minor: kind === "refund" ? toMinor(amount) : null }).then(() => setNote(""));
            }}
          >
            <div className="form-row">
              <label>
                Propose
                <select value={kind} onChange={(e) => setKind(e.target.value as typeof kind)}>
                  <option value="refund">A refund</option>
                  <option value="redo">Redo a part at no charge</option>
                  <option value="cancel">Cancel the job, keeping what's paid</option>
                  <option value="other">Something else</option>
                </select>
              </label>
              {kind === "refund" && (
                <label>
                  Amount
                  <input inputMode="decimal" value={amount} required onChange={(e) => setAmount(e.target.value)} />
                </label>
              )}
            </div>
            <label>
              In words
              <input value={note} maxLength={1000} required onChange={(e) => setNote(e.target.value)} />
            </label>
            <button className="btn small" disabled={busy}>
              Propose
            </button>
          </form>
          <label className="small">
            Submit your copy of the conversation for the mediator{" "}
            <input type="file" accept="application/json,.json" onChange={(e) => e.target.files?.[0] && void submitCopy(e.target.files[0])} />
          </label>
          {copyResult && <p className="small">{copyResult}</p>}
          {p.opened_by_me && (
            <button className="link-btn small" disabled={busy} onClick={() => void step("problem/withdraw")}>
              Withdraw the problem
            </button>
          )}
          <p className="muted small">
            If you can't agree: the record (download it above) is yours for a card dispute or a court. Slop doesn't decide who's right.
          </p>
        </>
      )}
    </section>
  );
}

function ReviewForm({ j, busy, onSend }: { j: Job; busy: boolean; onSend: (body: { vote: string; body: string }) => void }) {
  const [vote, setVote] = useState<"not_slop" | "slop" | undefined>(j.review?.vote);
  const [body, setBody] = useState(j.review?.body ?? "");
  const [answer, setAnswer] = useState("");
  const dispute = j.review?.dispute;
  return (
    <form
      className="panel panel-pad form"
      onSubmit={(e) => {
        e.preventDefault();
        if (vote) onSend({ vote, body });
      }}
    >
      <h2>{j.review ? "Your review" : `Was ${j.creative.name}'s work slop?`}</h2>
      <p className="muted small">Judge it against the agreement: did you get exactly what it says?</p>
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
      {j.review?.hidden && <p className="bad small">A mediator removed this review: {dispute?.decision_reason}</p>}
      {j.review?.mediator_note && (
        <div className="mediator-note">
          <b>Mediator's note</b> {j.review.mediator_note}
        </div>
      )}
      {j.review?.reply && (
        <div className="review-reply">
          <b>{j.creative.name} replied</b> {j.review.reply}
        </div>
      )}
      {dispute?.status === "pending" && (
        <div className="notice">
          <b>{j.creative.name} disputes this review</b> ({dispute.reason.replace("_", " ")}): {dispute.statement}
          {dispute.client_statement ? (
            <div>
              <b>Your answer:</b> {dispute.client_statement}
            </div>
          ) : (
            <div className="form">
              <label>
                Your side, for the mediator
                <textarea value={answer} rows={3} maxLength={2000} onChange={(e) => setAnswer(e.target.value)} />
              </label>
              <button
                type="button"
                className="btn small"
                disabled={busy || !answer.trim()}
                onClick={() =>
                  void api.people.step(j.id, "review/dispute-answer", { statement: answer }).then(() => window.location.reload())
                }
              >
                Send your answer
              </button>
            </div>
          )}
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
          placeholder={required ? "Write it out" : "Optional note"}
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
