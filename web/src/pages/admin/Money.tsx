// SPDX-License-Identifier: AGPL-3.0-or-later
// Money for staff: donation pauses and makers' requests to lift them (every staff member sees
// these), and for finance staff the payments record and SLAPorSLOP's fees by month
// (crates/slop-api/src/admin_console.rs).

import { useState } from "react";

import { api, ApiError } from "../../api";
import { ErrorState, Loading } from "../../components/ui";
import { formatDate, money } from "../../format";
import { useLoad } from "../../hooks";
import { HoldCard } from "./DonationHolds";

export function MoneyTab() {
  const [all, setAll] = useState(false);
  const holds = useLoad(() => api.admin.donationHolds(all ? "all" : "open"), [all]);
  return (
    <>
      <section>
        <div className="title-row spread">
          <h2 className="section-title">
            Donation pauses {holds.data && <span className="muted">({holds.data.filter((h) => !h.lifted_at).length} in force)</span>}
          </h2>
          <label className="check small">
            <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} />
            <span>Show lifted ones too</span>
          </label>
        </div>
        <p className="muted small">
          To pause an app's donations, open the app under Apps. Requests from makers who say they've fixed the problem
          come first.
        </p>
        {holds.error && <ErrorState error={holds.error} onRetry={holds.reload} />}
        {!holds.data && !holds.error && <Loading />}
        {holds.data?.length === 0 && <div className="empty-state">No donation pauses.</div>}
        <div className="release-list">
          {holds.data?.map((h) => (
            <HoldCard key={h.id} hold={h} showApp onDone={holds.reload} />
          ))}
        </div>
      </section>
      {/* Its counts of pauses follow the list above as staff answer and lift them. */}
      <Payments holds={holds.data} />
    </>
  );
}

function Payments({ holds }: { holds: unknown }) {
  const m = useLoad(() => api.admin.money(), [holds]);
  if (m.error instanceof ApiError && m.error.status === 403)
    return <p className="muted">The payments record is for staff with the finance role.</p>;
  if (m.error) return <ErrorState error={m.error} onRetry={m.reload} />;
  if (!m.data) return <Loading />;
  const d = m.data;
  return (
    <>
      <section>
        <h2 className="section-title">Payments</h2>
        <p>
          Payments are <b>{d.payments === "stripe" ? "live through Stripe" : d.payments === "simulated" ? "simulated" : "off"}</b> ·{" "}
          {d.stripe_events_today} Stripe events in the last day · {d.active_server_donations} monthly server donations
          active · {d.open_holds} donation pauses in force ({d.hold_requests_waiting} waiting for an answer)
        </p>
      </section>
      <section>
        <h3>The last 12 months</h3>
        {d.months.length === 0 ? (
          <p className="muted">No payments yet.</p>
        ) : (
          <table className="audit">
            <thead>
              <tr>
                <th>Month</th>
                <th>Kind</th>
                <th>Payments</th>
                <th>Amount</th>
                <th>SLAPorSLOP's fees</th>
              </tr>
            </thead>
            <tbody>
              {d.months.map((r) => (
                <tr key={`${r.month}-${r.kind}-${r.currency}`}>
                  <td>{r.month}</td>
                  <td>{r.kind}</td>
                  <td>{r.payments}</td>
                  <td>{money(r.amount_minor, r.currency)}</td>
                  <td>{money(r.slop_fee_minor, r.currency)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <section>
        <h3>Newest payments</h3>
        {d.recent.length === 0 ? (
          <p className="muted">None.</p>
        ) : (
          <table className="audit">
            <thead>
              <tr>
                <th>When</th>
                <th>Kind</th>
                <th>Amount</th>
                <th>Fee</th>
                <th>Stripe</th>
              </tr>
            </thead>
            <tbody>
              {d.recent.map((p) => (
                <tr key={p.id}>
                  <td>{formatDate(p.created_at)}</td>
                  <td>
                    {p.kind}
                    {p.simulated && " (simulated)"}
                  </td>
                  <td>{money(p.amount_minor, p.currency)}</td>
                  <td>{money(p.slop_fee_minor, p.currency)}</td>
                  <td className="muted small">
                    <code>{p.stripe_object_id ?? "—"}</code> on <code>{p.stripe_account_id ?? "—"}</code>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </>
  );
}
