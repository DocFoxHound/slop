// SPDX-License-Identifier: AGPL-3.0-or-later
// The law-enforcement request log (crates/slop-api/src/legal_requests.rs, LEGAL-GUIDANCE CHAT-3):
// log a request as it arrives, then record what Slop gave and whether the person was told.

import { useState, type FormEvent } from "react";

import { api, ApiError, type LegalRequest, type LegalRequestKind } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { ErrorState, Loading } from "./ui";

const KIND_LABEL: Record<LegalRequestKind, string> = {
  subpoena: "Subpoena",
  court_order: "Court order",
  warrant: "Warrant",
  eu_production_order: "EU production order (10 days)",
  preservation: "Preservation request",
  emergency: "Emergency request (8 hours)",
  uk_daa: "UK request (US-UK Data Access Agreement)",
  removal_order: "Terrorist content removal order (1 hour)",
  other: "Other",
};

const STATUS_LABEL: Record<LegalRequest["status"], string> = {
  open: "OPEN",
  answered: "ANSWERED",
  partly_answered: "PARTLY ANSWERED",
  refused: "REFUSED",
  withdrawn: "WITHDRAWN",
};

export function LegalRequestsTab() {
  const list = useLoad(() => api.admin.legalRequests(), []);
  if (list.error) return <ErrorState error={list.error} onRetry={list.reload} />;
  if (!list.data) return <Loading />;
  return (
    <>
      <p className="muted small">
        Every request from police, a court or an authority, as it arrives. Slop can only give what it has: account details,
        the metadata it keeps and reported messages; chat is end-to-end encrypted. Tell the person unless a law or the
        order forbids it. Kept 6 years; the yearly transparency report counts them.
      </p>
      <div className="row-actions">
        <NewRequest onLogged={list.reload} />
        <RemovalOrder onDone={list.reload} />
      </div>
      <ul className="release-list">
        {list.data.map((r) => (
          <li key={r.id} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <span className={`badge ${r.status === "open" ? "warn" : "neutral"}`}>{STATUS_LABEL[r.status]}</span>
                <b>
                  {r.reference} · {KIND_LABEL[r.kind]}
                </b>
              </div>
              <span className={`small ${r.status === "open" && r.deadline && Date.parse(r.deadline) < Date.now() ? "bad" : "muted"}`}>
                {r.country} · {r.authority} · received {formatDate(r.received_at)}
                {r.deadline && ` · due ${new Date(r.deadline).toLocaleString()}`}
              </span>
            </div>
            <dl className="kv">
              <dt>Asked</dt>
              <dd className="pre">{r.asked}</dd>
              {r.legal_basis && (
                <>
                  <dt>Legal basis</dt>
                  <dd>{r.legal_basis}</dd>
                </>
              )}
              {r.subject_name && (
                <>
                  <dt>Account</dt>
                  <dd>{r.subject_name}</dd>
                </>
              )}
              {r.response && (
                <>
                  <dt>Response</dt>
                  <dd className="pre">
                    {r.response}
                    <div className="muted small">
                      {r.answered_by} · {r.answered_at && formatDate(r.answered_at)} · person {r.user_notice.replace("_", " ")}
                    </div>
                  </dd>
                </>
              )}
            </dl>
            {r.status === "open" && <AnswerForm r={r} onAnswered={list.reload} />}
          </li>
        ))}
      </ul>
    </>
  );
}

function NewRequest({ onLogged }: { onLogged: () => void }) {
  const [open, setOpen] = useState(false);
  const [f, setF] = useState({
    received_at: "",
    country: "",
    authority: "",
    kind: "subpoena" as LegalRequestKind,
    legal_basis: "",
    asked: "",
    subject_user_id: "",
  });
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((x) => ({ ...x, [k]: v }));
  if (!open) {
    return (
      <button type="button" className="btn slime" onClick={() => setOpen(true)}>
        Log a request
      </button>
    );
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.admin.logLegalRequest({
        ...f,
        received_at: new Date(f.received_at || Date.now()).toISOString(),
        subject_user_id: f.subject_user_id.trim() || null,
      });
      setOpen(false);
      onLogged();
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };
  const fieldError = (name: string) =>
    error?.field === name ? (
      <span className="field-error" role="alert">
        {error.message}
      </span>
    ) : null;
  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
      <h2>Log a request</h2>
      <div className="row-2">
        <label>
          Received
          <input type="datetime-local" value={f.received_at} onChange={(e) => set("received_at", e.target.value)} />
          <span className="hint">Empty: now.</span>
        </label>
        <label>
          Country
          <input value={f.country} maxLength={2} required placeholder="US" onChange={(e) => set("country", e.target.value)} />
          {fieldError("country")}
        </label>
      </div>
      <label>
        From
        <input value={f.authority} maxLength={200} required placeholder="The authority or court" onChange={(e) => set("authority", e.target.value)} />
        {fieldError("authority")}
      </label>
      <label>
        Kind
        <select value={f.kind} onChange={(e) => set("kind", e.target.value as LegalRequestKind)}>
          {(Object.keys(KIND_LABEL) as LegalRequestKind[]).map((k) => (
            <option key={k} value={k}>
              {KIND_LABEL[k]}
            </option>
          ))}
        </select>
      </label>
      <label>
        Legal basis <span className="muted">(optional)</span>
        <input value={f.legal_basis} maxLength={500} onChange={(e) => set("legal_basis", e.target.value)} />
      </label>
      <label>
        What they ask for
        <textarea value={f.asked} rows={3} maxLength={4000} required onChange={(e) => set("asked", e.target.value)} />
        {fieldError("asked")}
      </label>
      <label>
        Account ID <span className="muted">(if it's about one)</span>
        <input value={f.subject_user_id} onChange={(e) => set("subject_user_id", e.target.value)} />
        {fieldError("subject_user_id")}
      </label>
      {error && !error.field && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="actions">
        <button type="button" className="btn ghost" onClick={() => setOpen(false)}>
          Cancel
        </button>
        <button className="btn slime" disabled={busy}>
          {busy ? "Logging…" : "Log it"}
        </button>
      </div>
    </form>
  );
}

function AnswerForm({ r, onAnswered }: { r: LegalRequest; onAnswered: () => void }) {
  const [status, setStatus] = useState("answered");
  const [response, setResponse] = useState("");
  const [notice, setNotice] = useState(r.subject_user_id ? "notified" : "not_needed");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.admin.answerLegalRequest(r.id, { status, response, user_notice: notice });
      onAnswered();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <details>
      <summary className="small">Record the answer</summary>
      <form className="form" onSubmit={(e) => void submit(e)}>
        <label>
          Outcome
          <select value={status} onChange={(e) => setStatus(e.target.value)}>
            <option value="answered">Answered</option>
            <option value="partly_answered">Partly answered</option>
            <option value="refused">Refused</option>
            <option value="withdrawn">Withdrawn</option>
          </select>
        </label>
        <label>
          What Slop gave, or why not
          <textarea value={response} rows={3} maxLength={4000} required onChange={(e) => setResponse(e.target.value)} />
        </label>
        <label>
          The person
          <select value={notice} onChange={(e) => setNotice(e.target.value)}>
            <option value="notified">Told</option>
            <option value="barred">Not told: a law or the order forbids it</option>
            <option value="not_needed">No account involved</option>
          </select>
        </label>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <button className="btn slime small" disabled={busy}>
          {busy ? "Saving…" : "Save the answer"}
        </button>
      </form>
    </details>
  );
}

/** A terrorist content removal order: take it down now, in one step. */
function RemovalOrder({ onDone }: { onDone: () => void }) {
  const [open, setOpen] = useState(false);
  const [f, setF] = useState({
    received_at: "",
    country: "",
    authority: "",
    order: "",
    subject_kind: "app" as "app" | "app_review" | "profile" | "team",
    subject: "",
  });
  const [done, setDone] = useState<string>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((x) => ({ ...x, [k]: v }));
  if (!open) {
    return (
      <button type="button" className="btn danger" onClick={() => setOpen(true)}>
        Terrorist content removal order
      </button>
    );
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const r = await api.admin.removalOrder({ ...f, received_at: new Date(f.received_at || Date.now()).toISOString() });
      setDone(`${r.label} is down (${r.reference}), ${r.minutes} minutes after the order arrived.`);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
      <h2>Removal order: act within the hour</h2>
      <p className="muted small">
        One step: the content comes down at once as a moderation decision (the person is told why and can appeal), a
        sealed copy is kept for six months, and the order is logged with how long it took. Check the order is genuine and
        from a competent authority first.
      </p>
      {done && (
        <p className="notice" role="status">
          {done}
        </p>
      )}
      <div className="row-2">
        <label>
          Received
          <input type="datetime-local" value={f.received_at} onChange={(e) => set("received_at", e.target.value)} />
          <span className="hint">Empty: now. The hour runs from when it arrived.</span>
        </label>
        <label>
          Country
          <input value={f.country} maxLength={2} required placeholder="FR" onChange={(e) => set("country", e.target.value)} />
        </label>
      </div>
      <label>
        Authority
        <input value={f.authority} maxLength={200} required onChange={(e) => set("authority", e.target.value)} />
      </label>
      <label>
        The order (its reference and what it names)
        <textarea value={f.order} rows={2} maxLength={4000} required onChange={(e) => set("order", e.target.value)} />
      </label>
      <div className="row-2">
        <label>
          What
          <select value={f.subject_kind} onChange={(e) => set("subject_kind", e.target.value as typeof f.subject_kind)}>
            <option value="app">An app (its address)</option>
            <option value="app_review">A review (its ID)</option>
            <option value="profile">A People profile (the handle)</option>
            <option value="team">A team (its address)</option>
          </select>
        </label>
        <label>
          Which
          <input value={f.subject} required onChange={(e) => set("subject", e.target.value)} />
        </label>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="actions">
        <button type="button" className="btn ghost" onClick={() => setOpen(false)}>
          Close
        </button>
        <button className="btn danger" disabled={busy}>
          {busy ? "Taking it down…" : "Take it down now"}
        </button>
      </div>
    </form>
  );
}
