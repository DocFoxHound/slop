// SPDX-License-Identifier: AGPL-3.0-or-later
// App of the Month (crates/slop-api/src/awards.rs): every month's winners, newest first, each
// with the award SLAPorSLOP gave it. The badges on store pages and profiles link here.

import { Link } from "react-router";

import { api } from "../api";
import { awardAmount, monthLabel } from "../components/Awards";
import { StoreFrame } from "../components/StoreNav";
import { AppTile, ErrorState, Loading } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";

export function Awards() {
  useDocumentTitle("App of the Month");
  const awards = useLoad(() => api.awards(), []);
  return (
    <StoreFrame sidebar>
      <div className="crumbs">
        <Link to="/">Store</Link> › Picked by SLAPorSLOP
      </div>
      <h1 className="page-title">
        <span aria-hidden="true">🏆</span> App of the Month
      </h1>
      <p className="muted">
        Every month SLAPorSLOP's staff vote for apps they think players should try, and pick up to three. Each winner
        gets a small cash award, which SLAPorSLOP pays to the maker through the app's Donate button. Nobody can pay for an award, and it doesn't change where an app shows in the store.
      </p>
      {awards.error ? (
        <ErrorState error={awards.error} onRetry={awards.reload} />
      ) : !awards.data ? (
        <Loading />
      ) : awards.data.length === 0 ? (
        <div className="empty-state">No App of the Month yet.</div>
      ) : (
        awards.data.map((m) => (
          <section key={m.month} className="award-month" aria-labelledby={`awards-${m.month}`}>
            <h2 className="section-title" id={`awards-${m.month}`}>
              {monthLabel(m.month)}
            </h2>
            <ul className="award-winners">
              {m.winners.map((w) => (
                <li key={w.slug}>
                  <AppTile app={w} />
                  <span className="small">
                    <b>{awardAmount(w.amount_minor, w.currency)} award</b>
                    {w.maker_id && w.maker_name && (
                      <>
                        {" "}
                        · by <Link to={`/u/${w.maker_id}`}>{w.maker_name}</Link>
                      </>
                    )}
                  </span>
                  {w.blurb && <span className="small muted">{w.blurb}</span>}
                </li>
              ))}
            </ul>
          </section>
        ))
      )}
    </StoreFrame>
  );
}
