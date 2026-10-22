// SPDX-License-Identifier: AGPL-3.0-or-later
// The maker's donation settings and totals, on the Manage page (PAYMENTS §4–6). One Stripe
// account per app: the maker's own. Breakdowns appear only from 10 donations up (METRICS §3).

import { useState, type FormEvent } from "react";

import { api, ApiError, type DonationHold, type DonationSettings } from "../api";
import { formatDate, toMinor } from "../format";
import { useLoad } from "../hooks";
import { openExternal } from "../platform";
import { useSession } from "../session";
import { amount } from "./Donate";
import { useTermsGate } from "./Terms";
import { ErrorState } from "./ui";

export function DonationsPanel({ slug }: { slug: string }) {
  const settings = useLoad(() => api.donations.settings(slug), [slug]);
  if (settings.error) return <ErrorState error={settings.error} onRetry={settings.reload} />;
  if (!settings.data) return null;
  const s = settings.data;
  return (
    <section className="panel panel-pad" id="donations">
      <h2>Donations</h2>
      <HoldNotice slug={slug} />
      {s.payments === "off" ? (
        <p className="muted">Payments aren't switched on on this server yet.</p>
      ) : s.account_status !== "active" ? (
        <EnableDonations status={s.account_status} onDone={settings.reload} slug={slug} />
      ) : (
        <>
          <SettingsForm slug={slug} s={s} onSaved={settings.reload} />
          <Summary slug={slug} />
        </>
      )}
    </section>
  );
}

/** Staff stopped this app's donations: why, and the maker's way to ask for them back. */
function HoldNotice({ slug }: { slug: string }) {
  const hold = useLoad(() => api.donations.hold(slug), [slug]);
  if (!hold.data?.open) return null;
  const h = hold.data.open;
  return (
    <div className="notice" role="status">
      <p>
        <b>
          {h.kind === "review"
            ? "Donations to this app are paused while Slop reviews it."
            : "Donations to this app are turned off until the problem below is fixed."}
        </b>{" "}
        Your app stays in the store. Donors see only that donations are paused, and monthly server donations aren't
        charged meanwhile.
      </p>
      <p className="pre-wrap">{h.reason}</p>
      {h.ground && <p className="muted">Ground: {h.ground}</p>}
      <p className="muted">Since {formatDate(h.placed_at)}</p>
      {h.requests.length > 0 && (
        <ol className="plain">
          {h.requests.map((r) => (
            <li key={r.id}>
              <p>
                You asked on {formatDate(r.created_at)}: <span className="pre-wrap">{r.message}</span>
              </p>
              {r.outcome === null ? (
                <p className="muted">Waiting for Slop's answer.</p>
              ) : (
                <p>
                  Slop {r.outcome === "kept" ? "kept the pause" : "lifted it"}: <span className="pre-wrap">{r.answer}</span>
                </p>
              )}
            </li>
          ))}
        </ol>
      )}
      <AskToLift slug={slug} hold={h} canAskAt={hold.data.can_ask_at} onDone={hold.reload} />
    </div>
  );
}

function AskToLift({ slug, hold, canAskAt, onDone }: { slug: string; hold: DonationHold; canAskAt: string | null; onDone: () => void }) {
  const [message, setMessage] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  if (hold.requests.some((r) => r.outcome === null)) return null;
  if (canAskAt) return <p className="muted">You can ask again after {formatDate(canAskAt)}.</p>;
  const send = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.donations.askToLift(slug, message);
      setMessage("");
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="form" onSubmit={send}>
      <label htmlFor="hold-fix">Fixed it? Tell Slop what you changed and ask for donations back.</label>
      <textarea id="hold-fix" rows={3} maxLength={4000} value={message} onChange={(e) => setMessage(e.target.value)} required />
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <button className="btn" disabled={busy || !message.trim()}>
        Ask for a review
      </button>
    </form>
  );
}

function EnableDonations({ status, onDone, slug }: { status: string; onDone: () => void; slug: string }) {
  const { me } = useSession();
  const [ack, setAck] = useState(false);
  const [adult, setAdult] = useState(false);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const gate = useTermsGate();
  const go = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const res = await gate.guard(() => api.donations.onboard({ tax_notice_ack: ack, confirm_adult: adult, return_to: `/publish/${slug}/manage#donations` }));
      if (res.url) await openExternal(res.url);
      else onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="form">
      {gate.prompt}
      <p>
        Donations go straight to your own Stripe account. Slop takes a 5% platform fee; donors pay the card processing on
        top, so you receive the amount they chose less 5%. Donations never unlock anything, and your app can't tell who
        donated.
      </p>
      {status === "pending" && <p className="notice">Stripe is still checking your account. Finish Stripe's sign-up, then come back.</p>}
      {status === "restricted" && <p className="notice">Stripe needs more from you before donations can arrive. Open Stripe to fix it.</p>}
      <label className="check">
        <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} />
        <span>
          I understand that <b>donations may be taxable income for me</b>, and that Slop doesn't give tax advice.
        </span>
      </label>
      {me && !me.adult_confirmed && (
        <label className="check">
          <input type="checkbox" checked={adult} onChange={(e) => setAdult(e.target.checked)} />
          <span>I'm 18 or older.</span>
        </label>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="row-actions">
        <button type="button" className="btn slime" disabled={busy || !ack} onClick={() => void go()}>
          {status === "none" ? "Enable donations with Stripe" : "Continue on Stripe"}
        </button>
        {status !== "none" && (
          <button type="button" className="btn" onClick={() => void api.donations.refreshAccount().then(onDone)}>
            I've finished on Stripe
          </button>
        )}
      </div>
    </div>
  );
}

function SettingsForm({ slug, s, onSaved }: { slug: string; s: DonationSettings; onSaved: () => void }) {
  const [on, setOn] = useState(s.donations_on);
  const [suggested, setSuggested] = useState((s.donation_suggested_minor / 100).toFixed(2));
  const [currency, setCurrency] = useState(s.donation_currency);
  const [servers, setServers] = useState(s.runs_servers);
  const [statement, setStatement] = useState(s.servers_statement ?? "");
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const [shutting, setShutting] = useState(false);
  const save = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    setSaved(false);
    try {
      await api.donations.saveSettings(slug, {
        donations_on: on,
        donation_suggested_minor: toMinor(suggested) ?? 0,
        donation_currency: currency,
        runs_servers: servers,
        servers_statement: servers ? statement : null,
      });
      setSaved(true);
      onSaved();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const shutDown = async () => {
    try {
      const res = await api.donations.serversShutDown(slug);
      setShutting(false);
      setServers(false);
      setError(undefined);
      setSaved(false);
      onSaved();
      window.alert(`Done. ${res.cancelled} server donation${res.cancelled === 1 ? " was" : "s were"} cancelled, and the donors told.`);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <form className="form" onSubmit={(e) => void save(e)}>
      <label className="check">
        <input type="checkbox" checked={on} onChange={(e) => setOn(e.target.checked)} />
        <span>Show the Donate button on the store page</span>
      </label>
      <div className="form-row">
        <label>
          Suggested amount
          <input inputMode="decimal" value={suggested} onChange={(e) => setSuggested(e.target.value)} />
          <span className="hint">What you receive before the 5% fee; the box offers amounts around it.</span>
        </label>
        <label>
          Currency
          <select value={currency} onChange={(e) => setCurrency(e.target.value)}>
            <option value="usd">US dollars</option>
            <option value="eur">Euros</option>
            <option value="gbp">Pounds</option>
          </select>
        </label>
      </div>
      <label className="check">
        <input type="checkbox" checked={servers} onChange={(e) => setServers(e.target.checked)} disabled={s.active_server_donations > 0} />
        <span>This app runs servers (offers a monthly server donation)</span>
      </label>
      {servers && (
        <label>
          What the servers are, and what donations help pay for
          <textarea value={statement} rows={2} maxLength={300} onChange={(e) => setStatement(e.target.value)} />
          <span className="hint">Shown to donors. Server access must never depend on donating.</span>
        </label>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="row-actions">
        <button className="btn slime">Save</button>
        {saved && <span className="muted small">Saved.</span>}
      </div>
      {s.runs_servers && (
        <div className="shut-down">
          {!shutting ? (
            <button type="button" className="btn small ghost danger" onClick={() => setShutting(true)}>
              The servers are shut down…
            </button>
          ) : (
            <p className="notice">
              This cancels every monthly server donation to this app and emails the donors.{" "}
              <button type="button" className="btn small danger" onClick={() => void shutDown()}>
                Yes, the servers are shut down
              </button>{" "}
              <button type="button" className="btn small ghost" onClick={() => setShutting(false)}>
                Cancel
              </button>
            </p>
          )}
        </div>
      )}
    </form>
  );
}

function Summary({ slug }: { slug: string }) {
  const summary = useLoad(() => api.donations.summary(slug), [slug]);
  const [error, setError] = useState<string>();
  if (!summary.data) return null;
  const s = summary.data;
  const refund = async (id: string) => {
    if (!window.confirm("Refund this donation in full, from your Stripe account?")) return;
    try {
      await api.donations.refund(id);
      summary.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <div className="donation-summary">
      <h3>So far</h3>
      {s.currency_totals.length === 0 ? (
        <p className="muted small">No donations yet.</p>
      ) : (
        <ul className="plain">
          {s.currency_totals.map((t) => (
            <li key={t.currency}>
              <b>{amount(t.donated_minor - t.slop_fee_minor, t.currency)}</b> to you from {t.donations} donation
              {t.donations === 1 ? "" : "s"} <span className="muted small">({amount(t.slop_fee_minor, t.currency)} Slop fees
              {t.refunded_minor > 0 && `, ${amount(t.refunded_minor, t.currency)} refunded`})</span>
            </li>
          ))}
        </ul>
      )}
      <p className="muted small">
        {s.active_server_donations !== null
          ? `${s.active_server_donations} people give monthly for the servers.`
          : "Monthly breakdowns and server-donor counts show from 10 donors up, so no one stands out."}
      </p>
      {s.months.length > 0 && (
        <table className="table small">
          <thead>
            <tr>
              <th>Month</th>
              <th>Donations</th>
              <th>Donated</th>
            </tr>
          </thead>
          <tbody>
            {s.months.map((m) => (
              <tr key={m.month + m.currency}>
                <td>{m.month}</td>
                <td>{m.donations}</td>
                <td>{amount(m.donated_minor, m.currency)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {s.shared.length > 0 && (
        <>
          <h3>From donors who shared their name</h3>
          <ul className="shared-list">
            {s.shared.map((d, i) => (
              <li key={i}>
                <b>{d.name}</b> <span className="muted small">· {amount(d.donation_minor, d.currency)} · {formatDate(d.paid_at)}</span>
                {d.message && <p className="small">“{d.message}”</p>}
              </li>
            ))}
          </ul>
          <p className="faint small">Only for you. Never show donors publicly, and never give them anything for donating.</p>
        </>
      )}
      {s.recent.some((r) => r.refundable) && (
        <details>
          <summary>Refund a mistake</summary>
          <ul className="plain small">
            {s.recent
              .filter((r) => r.refundable)
              .map((r) => (
                <li key={r.id}>
                  {amount(r.total_minor, r.currency)} on {formatDate(r.paid_at)}{" "}
                  <button type="button" className="link-btn small" onClick={() => void refund(r.id)}>
                    Refund
                  </button>
                </li>
              ))}
          </ul>
        </details>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
