// The account page's sections for moderation, agreements and the user's own data (DESIGN §4,
// §16): decisions about them with the statement of reasons and an appeal, the reports they sent,
// the agreements waiting for them, downloading their data and closing the account.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router";

import { api, ApiError, type CounterNotice, type LegalNotice, type ModerationDecision } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { saveFile } from "../platform";
import { useSession } from "../session";
import { AcceptDocument } from "./Terms";
import { ErrorState } from "./ui";

/** The Content Policy section a rule title points to (headings get ids from their text). */
function ruleLink(title: string): string {
  const slug = title
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");
  return `/legal/content#${slug}`;
}

export function AccountDecisions() {
  const { refresh } = useSession();
  const decisions = useLoad(() => api.decisions.mine(), []);
  // Opening the list marks them seen; the notice on every page goes away.
  useEffect(() => {
    if (decisions.data?.some((d) => d.new)) void refresh();
  }, [decisions.data, refresh]);
  if (decisions.error) return <ErrorState error={decisions.error} onRetry={decisions.reload} />;
  if (!decisions.data?.length) return null;
  return (
    <section id="decisions">
      <h2 className="section-title">Decisions about your content or account</h2>
      <p className="muted small">
        Each one says what we did, why, and which rule it was under. You can appeal each one once, within six months; a
        person looks at it again.
      </p>
      {decisions.data.map((d) => (
        <Decision key={d.id} d={d} onChange={decisions.reload} />
      ))}
    </section>
  );
}

function Decision({ d, onChange }: { d: ModerationDecision; onChange: () => void }) {
  const [appealing, setAppealing] = useState(false);
  return (
    <article className={`panel panel-pad decision${d.new ? " is-new" : ""}`} aria-labelledby={`decision-${d.id}`}>
      <div className="decision-head">
        <h3 id={`decision-${d.id}`}>
          {d.action_label}: {d.subject_label}
        </h3>
        {d.new && <span className="badge warn">NEW</span>}
        {d.reversed_at && <span className="badge official">REVERSED</span>}
      </div>
      <p className="muted small">
        {d.reference} · {formatDate(d.created_at)} · {d.automated ? "made with automated tools" : "decided by a person, not automated tools"}
      </p>
      <dl className="kv">
        <dt>What happened</dt>
        <dd>{d.facts}</dd>
        <dt>The rule</dt>
        <dd>
          <Link to={ruleLink(d.rule_title)}>Content Policy: {d.rule_title}</Link>
        </dd>
        {d.law && (
          <>
            <dt>The law</dt>
            <dd>{d.law}</dd>
          </>
        )}
      </dl>
      {d.appeal_status === "pending" && (
        <p className="notice">
          You appealed on {formatDate(d.appealed_at!)}. A person will look at it again and tell you here.
        </p>
      )}
      {(d.appeal_status === "upheld" || d.appeal_status === "reversed") && (
        <p className="notice">
          <b>{d.appeal_status === "reversed" ? "Your appeal succeeded: the decision is reversed." : "Your appeal was looked at, and the decision stands."}</b>{" "}
          {d.appeal_reason}
        </p>
      )}
      {d.can_appeal &&
        (appealing ? (
          <AppealForm d={d} onDone={onChange} onCancel={() => setAppealing(false)} />
        ) : (
          <p>
            <button type="button" className="btn small" onClick={() => setAppealing(true)}>
              Appeal this decision
            </button>{" "}
            <span className="muted small">until {formatDate(d.appeal_until)}</span>
          </p>
        ))}
    </article>
  );
}

const EMPTY_COUNTER: CounterNotice = {
  name: "",
  address: "",
  phone: "",
  mistake_under_penalty: false,
  consent_to_jurisdiction: false,
  signature: "",
};

function AppealForm({ d, onDone, onCancel }: { d: ModerationDecision; onDone: () => void; onCancel: () => void }) {
  const copyright = d.rule === "copyright";
  const [text, setText] = useState("");
  const [counter, setCounter] = useState<CounterNotice>(EMPTY_COUNTER);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.decisions.appeal(d.id, text, copyright ? counter : undefined);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
      setBusy(false);
    }
  };
  const fieldError = (field: string) =>
    error?.field === field ? (
      <span className="field-error" role="alert">
        {error.message}
      </span>
    ) : null;
  return (
    <form className="form appeal-form" onSubmit={(e) => void submit(e)}>
      <label>
        Why should we look again?
        <textarea value={text} rows={4} maxLength={4000} required onChange={(e) => setText(e.target.value)} />
        <span className="hint">Explain the context, or what you've changed. You can appeal once.</span>
        {fieldError("appeal")}
      </label>
      {copyright && (
        <fieldset>
          <legend>Counter-notice</legend>
          <p className="muted small">
            An appeal against a copyright decision is a counter-notice. We pass these details to the person who reported it;
            if they don't start a court case within 10 to 14 working days, we restore it.
          </p>
          <label>
            Your full name
            <input value={counter.name} maxLength={100} required onChange={(e) => setCounter({ ...counter, name: e.target.value })} />
            {fieldError("counter_notice.name")}
          </label>
          <label>
            Your postal address
            <input value={counter.address} maxLength={300} required onChange={(e) => setCounter({ ...counter, address: e.target.value })} />
            {fieldError("counter_notice.address")}
          </label>
          <label>
            Your phone number
            <input value={counter.phone} maxLength={40} required onChange={(e) => setCounter({ ...counter, phone: e.target.value })} />
            {fieldError("counter_notice.phone")}
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={counter.mistake_under_penalty}
              onChange={(e) => setCounter({ ...counter, mistake_under_penalty: e.target.checked })}
            />
            <span>Under penalty of perjury, I believe in good faith it was removed by mistake or misidentification.</span>
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={counter.consent_to_jurisdiction}
              onChange={(e) => setCounter({ ...counter, consent_to_jurisdiction: e.target.checked })}
            />
            <span>
              I consent to the jurisdiction of the US federal district court for my address (or, outside the US, any district
              where Slop may be found), and will accept service from the person who reported it.
            </span>
          </label>
          {fieldError("counter_notice.statements")}
          <label>
            Your full name, as your signature
            <input value={counter.signature} maxLength={100} required onChange={(e) => setCounter({ ...counter, signature: e.target.value })} />
            {fieldError("counter_notice.signature")}
          </label>
        </fieldset>
      )}
      {error && !error.field && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="row-actions">
        <button className="btn slime" disabled={busy || !text.trim()}>
          {busy ? "Sending…" : "Send appeal"}
        </button>
        <button type="button" className="btn ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

const REPORT_STATUS = { open: "Waiting for review", actioned: "Acted on", dismissed: "No action" } as const;

export function AccountReports() {
  const reports = useLoad(() => api.reports.mine(), []);
  if (reports.error) return <ErrorState error={reports.error} onRetry={reports.reload} />;
  if (!reports.data?.length) return null;
  return (
    <section id="reports">
      <h2 className="section-title">Your reports</h2>
      <ul className="panel report-list">
        {reports.data.map((r) => (
          <li key={r.reference}>
            <div>
              <b>{r.subject_label}</b> <span className="muted small">· {r.reason_label}</span>
              <div className="muted small">
                {r.reference} · sent {formatDate(r.created_at)}
                {r.decided_at && <> · answered {formatDate(r.decided_at)}</>}
              </div>
              {r.outcome && <p className="small">{r.outcome}</p>}
            </div>
            <span className={`badge ${r.status === "actioned" ? "official" : "neutral"}`}>{REPORT_STATUS[r.status].toUpperCase()}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function AccountAgreements() {
  const notices = useLoad(() => api.legal.notices(), []);
  const [accepting, setAccepting] = useState<LegalNotice>();
  return (
    <section id="agreements">
      <h2 className="section-title">Terms and policies</h2>
      <div className="panel panel-pad">
        {notices.data?.length ? (
          <ul className="legal-notices">
            {notices.data.map((n) => (
              <li key={`${n.kind}-${n.version}`}>
                <b>{n.title}</b>{" "}
                <span className="muted small">
                  {n.first
                    ? `published ${formatDate(n.effective_at)}`
                    : `${n.in_effect ? "changed" : "changes"} ${formatDate(n.effective_at)}: ${n.summary}`}
                </span>{" "}
                {n.action === "accept" ? (
                  <button type="button" className="btn small" onClick={() => setAccepting(n)}>
                    Review and accept
                  </button>
                ) : (
                  <Link to={`/legal/${n.kind}?version=${n.version}`}>Read it</Link>
                )}
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted small">You're up to date with everything you've agreed to.</p>
        )}
        <p className="small">
          <Link to="/legal">All terms and policies, with every earlier version</Link>
        </p>
      </div>
      {accepting && (
        <AcceptDocument
          kind={accepting.kind}
          version={accepting.version}
          onAccepted={() => {
            setAccepting(undefined);
            notices.reload();
          }}
          onCancel={() => setAccepting(undefined)}
        />
      )}
    </section>
  );
}

const CONFIRMATION = "close my account";

export function AccountData() {
  const navigate = useNavigate();
  const { refresh } = useSession();
  const [error, setError] = useState<string>();
  const [closing, setClosing] = useState(false);
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);

  const download = async () => {
    setError(undefined);
    try {
      saveFile("slop-data.json", await api.exportData(), "application/json");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const close = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.closeAccount(confirm);
      await refresh();
      navigate("/", { replace: true });
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <section id="data">
      <h2 className="section-title">Your data</h2>
      <div className="panel panel-pad">
        <p>
          <button type="button" className="btn" onClick={() => void download()}>
            Download my data
          </button>{" "}
          <span className="muted small">Everything linked to your account, as one JSON file.</span>
        </p>
        {!closing ? (
          <p>
            <button type="button" className="btn ghost danger" onClick={() => setClosing(true)}>
              Close my account…
            </button>
          </p>
        ) : (
          <form className="form close-account" onSubmit={(e) => void close(e)}>
            <h3>Close your account</h3>
            <p>This can't be undone. When you close your account:</p>
            <ul>
              <li>You're signed out everywhere, and this sign-in can never be used on Slop again.</li>
              <li>Your email and settings are removed, and your library and playtime deleted.</li>
              <li>Your votes still count in ratings, but aren't linked to you any more.</li>
              <li>Your apps leave the store. Players who installed them keep them.</li>
              <li>
                A People page is frozen as it is and takes no new work; job records, agreements and reviews stay, because
                the other side relies on them.
              </li>
            </ul>
            <p className="muted small">Download your data first if you want a copy. Open People jobs must be finished first.</p>
            <label>
              Type <b>{CONFIRMATION}</b> to confirm
              <input value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="off" />
            </label>
            <div className="row-actions">
              <button className="btn danger" disabled={busy || confirm.trim().toLowerCase() !== CONFIRMATION}>
                {busy ? "Closing…" : "Close my account"}
              </button>
              <button type="button" className="btn ghost" onClick={() => setClosing(false)}>
                Keep my account
              </button>
            </div>
          </form>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}
