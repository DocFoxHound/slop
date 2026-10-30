// SPDX-License-Identifier: AGPL-3.0-or-later
// The Report button (DESIGN §16; LEGAL-GUIDANCE CON-8): on apps, People profiles, reviews and job
// messages. Anyone can report, signed in or not. The report is acknowledged with a reference
// number; signed-in reporters can follow it on their account page.

import { useEffect, useRef, useState, type FormEvent } from "react";
import { Link } from "react-router";

import { api, ApiError, type CopyrightNotice, type ReportKind, type ReportReason } from "../api";
import { formatDate } from "../format";
import { useSession } from "../session";
import { InlineIcon } from "./StoreIcons";

const EMPTY_COPYRIGHT: CopyrightNotice = {
  work: "",
  owner: "",
  address: "",
  phone: "",
  good_faith: false,
  accurate_under_penalty: false,
  signature: "",
};

/** A small "Report" link that opens the report form in a dialog. */
export function ReportButton({
  kind,
  subject,
  label,
  className = "link-btn small muted",
}: {
  kind: ReportKind;
  /** An app's address, a creative's handle, a review's job ID or a message ID. */
  subject: string;
  /** What's being reported, in words: "this app", "this review". */
  label: string;
  className?: string;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (open) dialog.current?.showModal();
    else dialog.current?.close();
  }, [open]);
  return (
    <>
      <button type="button" className={className} onClick={() => setOpen(true)} aria-haspopup="dialog">
        <InlineIcon name="flag" /> Report
      </button>
      <dialog ref={dialog} className="dialog" aria-label={`Report ${label}`} onClose={() => setOpen(false)}>
        {open && <ReportForm kind={kind} subject={subject} label={label} onClose={() => setOpen(false)} />}
      </dialog>
    </>
  );
}

export function ReportForm({
  kind,
  subject,
  label,
  onClose,
}: {
  kind: ReportKind;
  subject: string;
  label: string;
  onClose: () => void;
}) {
  const { me } = useSession();
  const [reasons, setReasons] = useState<ReportReason[]>();
  const [reason, setReason] = useState("");
  const [details, setDetails] = useState("");
  const [goodFaith, setGoodFaith] = useState(false);
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [copyright, setCopyright] = useState<CopyrightNotice>(EMPTY_COPYRIGHT);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<{ reference: string; due_at: string }>();

  useEffect(() => {
    api.reports.reasons(kind).then(setReasons, (e: unknown) => e instanceof ApiError && setError(e));
  }, [kind]);

  const chosen = reasons?.find((r) => r.code === reason);
  const signedOut = me === null;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      setDone(
        await api.reports.send({
          kind,
          subject,
          reason,
          details,
          good_faith: goodFaith,
          ...(signedOut ? { name, email } : {}),
          ...(reason === "copyright" ? { copyright } : {}),
        }),
      );
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
  // Errors about a field the form shows go next to it; any other (such as reporting your own
  // bounty) goes at the end, so none is lost.
  const inline = [
    "reason",
    "details",
    "good_faith",
    ...(reason === "copyright"
      ? ["copyright.work", "copyright.address", "copyright.phone", "copyright.statements", "copyright.signature"]
      : []),
    ...(signedOut ? ["name", "email"] : []),
  ];

  if (done) {
    return (
      <div className="report-form">
        <h2>Thanks: we've got your report</h2>
        <p>
          Your reference is <b className="mono">{done.reference}</b>. A person will review it by{" "}
          <b>{formatDate(done.due_at)}</b>.
        </p>
        {me ? (
          <p className="muted small">
            You'll see what we decided under <Link to="/account/moderation#reports" onClick={onClose}>Your reports</Link> on your
            account page.
          </p>
        ) : (
          <p className="muted small">Keep the reference if you need to ask us about it.</p>
        )}
        <button type="button" className="btn" onClick={onClose} autoFocus>
          Close
        </button>
      </div>
    );
  }

  return (
    <form className="report-form form" onSubmit={(e) => void submit(e)}>
      <h2>Report {label}</h2>
      <p className="muted small">
        A person at SLAPorSLOP reviews every report. If someone is in immediate danger, call your local emergency number first.
      </p>
      <fieldset>
        <legend>What's wrong?</legend>
        {!reasons && <p className="muted small">Loading…</p>}
        <div className="report-reasons">
          {reasons?.map((r) => (
            <label key={r.code} className="check">
              <input type="radio" name="reason" value={r.code} checked={reason === r.code} onChange={() => setReason(r.code)} />
              <span>{r.label}</span>
            </label>
          ))}
        </div>
        {fieldError("reason")}
        {chosen?.urgent && <p className="small">We review reports like this within 24 hours.</p>}
      </fieldset>

      <label>
        What did you see, and where?
        <textarea value={details} maxLength={4000} rows={4} required onChange={(e) => setDetails(e.target.value)} />
        <span className="hint">Be specific: which version, which part of the page, what it does.</span>
        {fieldError("details")}
      </label>

      {reason === "copyright" && (
        <fieldset className="report-copyright">
          <legend>Copyright notice</legend>
          <p className="muted small">The law asks for these details. We pass them to the person the report is about.</p>
          <label>
            The work that's being copied
            <textarea value={copyright.work} rows={2} maxLength={1000} required onChange={(e) => setCopyright({ ...copyright, work: e.target.value })} />
            {fieldError("copyright.work")}
          </label>
          <label>
            Who owns it, if that isn't you
            <input value={copyright.owner} maxLength={200} onChange={(e) => setCopyright({ ...copyright, owner: e.target.value })} />
          </label>
          <label>
            Your postal address
            <input value={copyright.address} maxLength={300} required onChange={(e) => setCopyright({ ...copyright, address: e.target.value })} />
            {fieldError("copyright.address")}
          </label>
          <label>
            Your phone number
            <input value={copyright.phone} maxLength={40} required onChange={(e) => setCopyright({ ...copyright, phone: e.target.value })} />
            {fieldError("copyright.phone")}
          </label>
          <label className="check">
            <input type="checkbox" checked={copyright.good_faith} onChange={(e) => setCopyright({ ...copyright, good_faith: e.target.checked })} />
            <span>I believe in good faith that this use isn't authorised by the owner, its agent or the law.</span>
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={copyright.accurate_under_penalty}
              onChange={(e) => setCopyright({ ...copyright, accurate_under_penalty: e.target.checked })}
            />
            <span>The information in this notice is accurate and, under penalty of perjury, I'm the owner or authorised to act for them.</span>
          </label>
          {fieldError("copyright.statements")}
          <label>
            Your full name, as your signature
            <input value={copyright.signature} maxLength={100} required onChange={(e) => setCopyright({ ...copyright, signature: e.target.value })} />
            {fieldError("copyright.signature")}
          </label>
        </fieldset>
      )}

      {signedOut && (
        <>
          <label>
            Your name {reason === "child_abuse" && <span className="muted">(optional)</span>}
            <input value={name} maxLength={100} autoComplete="name" onChange={(e) => setName(e.target.value)} />
            {fieldError("name")}
          </label>
          <label>
            Your email {reason === "child_abuse" && <span className="muted">(optional)</span>}
            <input type="email" value={email} maxLength={254} autoComplete="email" onChange={(e) => setEmail(e.target.value)} />
            <span className="hint">Only to tell you what we decided.</span>
            {fieldError("email")}
          </label>
        </>
      )}

      <label className="check">
        <input type="checkbox" checked={goodFaith} onChange={(e) => setGoodFaith(e.target.checked)} />
        <span>I believe this report is accurate and complete.</span>
      </label>
      {fieldError("good_faith")}

      {error && !(error.field && inline.includes(error.field)) && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="row-actions">
        <button className="btn danger" disabled={busy || !reason || !details.trim() || !goodFaith}>
          {busy ? "Sending…" : "Send report"}
        </button>
        <button type="button" className="btn ghost" onClick={onClose}>
          Cancel
        </button>
      </div>
    </form>
  );
}
