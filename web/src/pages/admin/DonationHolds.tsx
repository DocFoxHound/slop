// SPDX-License-Identifier: AGPL-3.0-or-later
// Pausing an app's donations (crates/slop-api/src/donation_holds.rs): placing, changing and
// lifting a hold, and answering the maker's request to lift it.

import { useState, type FormEvent } from "react";
import { Link } from "react-router";

import { api, type AdminDonationHold } from "../../api";
import { formatDate } from "../../format";
import { ActionMessages, useAction } from "./shared";

const KIND_LABEL = {
  review: "Paused for review",
  off: "Turned off until fixed",
} as const;

/** Place a hold on an app's donations, or change the one in force. */
export function HoldForm({
  slug,
  current,
  onDone,
}: {
  slug: string;
  current?: AdminDonationHold;
  onDone: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<"review" | "off">(current?.kind ?? "review");
  const [reason, setReason] = useState(current?.reason ?? "");
  const [ground, setGround] = useState(current?.ground ?? "");
  const action = useAction(() => {
    setOpen(false);
    onDone();
  });
  if (!open) {
    return (
      <>
        <button className={`btn small ${current ? "ghost" : "ghost danger"}`} onClick={() => setOpen(true)}>
          {current ? "Change the pause" : "Pause donations"}
        </button>
        <ActionMessages {...action} />
      </>
    );
  }
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void action.run("hold", () =>
      api.admin.holdDonations(slug, { kind, reason: reason.trim(), ground: ground.trim() || undefined }),
    );
  };
  return (
    <form className="form decide-form" onSubmit={submit}>
      <fieldset>
        <legend>How</legend>
        <label className="check">
          <input type="radio" name={`hold-kind-${slug}`} checked={kind === "review"} onChange={() => setKind("review")} />
          <span>
            <b>Pause for review.</b> Quick and reversible, while Slop looks into a complaint.
          </span>
        </label>
        <label className="check">
          <input type="radio" name={`hold-kind-${slug}`} checked={kind === "off"} onChange={() => setKind("off")} />
          <span>
            <b>Turn off until fixed.</b> Slop found a problem the maker must fix. A sensitive action: it may need a
            second staff member's approval.
          </span>
        </label>
      </fieldset>
      <label>
        What's wrong and what would fix it, as the maker will read it
        <textarea value={reason} rows={4} maxLength={4000} required onChange={(e) => setReason(e.target.value)} />
      </label>
      <label>
        Ground (optional): a Content Policy rule, a law, or a complaint reference
        <input value={ground} maxLength={300} onChange={(e) => setGround(e.target.value)} />
      </label>
      <p className="muted small">
        The app stays in the store. New donations stop, monthly server donations aren't charged meanwhile, and donors
        see only "Donations to this app are paused".
      </p>
      <div className="row-actions">
        <button className="btn small danger" disabled={action.busy !== undefined || !reason.trim()}>
          {current ? "Save" : kind === "review" ? "Pause donations" : "Turn off donations"}
        </button>
        <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
      <ActionMessages {...action} />
    </form>
  );
}

/** One hold: what it says, the maker's requests, and the ways to answer. */
export function HoldCard({ hold, showApp, onDone }: { hold: AdminDonationHold; showApp?: boolean; onDone: () => void }) {
  const waiting = hold.requests.find((r) => r.outcome === null);
  return (
    <div className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          {showApp && (
            <Link to={`/admin?tab=apps&app=${encodeURIComponent(hold.slug)}`}>
              <b>{hold.title}</b>
            </Link>
          )}
          {showApp && <span className="muted">by {hold.maker}</span>}
          <span className={`badge ${hold.lifted_at ? "neutral" : "bad"}`}>
            {hold.lifted_at ? "LIFTED" : KIND_LABEL[hold.kind].toUpperCase()}
          </span>
          {waiting && <span className="badge official">MAKER ASKS TO LIFT</span>}
        </div>
        <span className="muted small">
          {hold.placed_by}, {formatDate(hold.placed_at)}
        </span>
      </div>
      <p className="pre-wrap">{hold.reason}</p>
      {hold.ground && <p className="muted small">Ground: {hold.ground}</p>}
      {!hold.lifted_at && hold.active_server_donations > 0 && (
        <p className="muted small">{hold.active_server_donations} monthly server donations paused.</p>
      )}
      {hold.requests.length > 0 && (
        <ol className="plain">
          {hold.requests.map((r) => (
            <li key={r.id}>
              <p>
                <b>Maker, {formatDate(r.created_at)}:</b> <span className="pre-wrap">{r.message}</span>
              </p>
              {r.outcome && (
                <p className="muted">
                  {r.outcome === "kept" ? "Kept" : "Lifted"}: <span className="pre-wrap">{r.answer}</span>
                </p>
              )}
            </li>
          ))}
        </ol>
      )}
      {hold.lifted_at ? (
        <p className="muted small">
          Lifted by {hold.lifted_by} on {formatDate(hold.lifted_at)}: {hold.lift_note}
        </p>
      ) : waiting ? (
        <AnswerForm requestId={waiting.id} onDone={onDone} />
      ) : (
        <div className="row-actions">
          <HoldForm slug={hold.slug} current={hold} onDone={onDone} />
          <LiftForm holdId={hold.id} onDone={onDone} />
        </div>
      )}
    </div>
  );
}

function LiftForm({ holdId, onDone }: { holdId: string; onDone: () => void }) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  const action = useAction(onDone);
  if (!open)
    return (
      <button className="btn small" onClick={() => setOpen(true)}>
        Lift the pause
      </button>
    );
  return (
    <form
      className="form decide-form"
      onSubmit={(e) => {
        e.preventDefault();
        void action.run("lift", () => api.admin.liftDonationHold(holdId, note.trim()));
      }}
    >
      <label>
        Why, as the maker will read it
        <textarea value={note} rows={2} maxLength={4000} required onChange={(e) => setNote(e.target.value)} />
      </label>
      <div className="row-actions">
        <button className="btn small" disabled={action.busy !== undefined || !note.trim()}>
          Lift it: donations back on
        </button>
        <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
      <ActionMessages {...action} />
    </form>
  );
}

function AnswerForm({ requestId, onDone }: { requestId: string; onDone: () => void }) {
  const [answer, setAnswer] = useState("");
  const action = useAction(onDone);
  const send = (lift: boolean) =>
    void action.run(lift ? "lift" : "keep", () => api.admin.answerDonationHoldRequest(requestId, lift, answer.trim()));
  return (
    <form className="form decide-form" onSubmit={(e) => e.preventDefault()}>
      <label>
        Your answer to the maker
        <textarea value={answer} rows={2} maxLength={4000} required onChange={(e) => setAnswer(e.target.value)} />
      </label>
      <div className="row-actions">
        <button
          type="button"
          className="btn small"
          disabled={action.busy !== undefined || !answer.trim()}
          onClick={() => send(true)}
        >
          It's fixed: lift the pause
        </button>
        <button
          type="button"
          className="btn small ghost danger"
          disabled={action.busy !== undefined || !answer.trim()}
          onClick={() => send(false)}
        >
          Not yet: keep it
        </button>
      </div>
      <ActionMessages {...action} />
    </form>
  );
}
