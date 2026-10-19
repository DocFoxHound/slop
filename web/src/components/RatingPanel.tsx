// 👍 Not Slop / 👎 Slop on an app's page: the rating (weighted by playtime, with the unweighted
// one beside it), the vote buttons, and the rating, votes and players over time
// (crates/slop-api/src/votes.rs, pulse.rs; DESIGN §7, METRICS §2).

import { useState } from "react";

import { api, ApiError, type Ratings } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { LineChart, UpDownBars } from "./Charts";

function shortDay(day: string) {
  return day.slice(5);
}

export function RatingSummary({ ratings }: { ratings: Ratings }) {
  const total = ratings.not_slop + ratings.slop;
  if (ratings.percent_weighted === null) {
    return (
      <div className="rating-summary">
        <b>{total === 0 ? "No votes yet" : `${total} vote${total === 1 ? "" : "s"}`}</b>
        <span className="muted small">
          {" "}
          · a rating shows from {ratings.min_votes} votes
        </span>
      </div>
    );
  }
  const good = ratings.percent_weighted >= 70;
  const bad = ratings.percent_weighted < 40;
  return (
    <div className="rating-summary">
      <span className={`rating-big ${good ? "good" : bad ? "bad" : "mixed"}`}>{ratings.percent_weighted}% Not Slop</span>{" "}
      <b>{ratings.label}</b>
      <div className="muted small">
        weighted by playtime · {ratings.percent}% unweighted · {total.toLocaleString()} votes ({ratings.not_slop.toLocaleString()}{" "}
        👍 / {ratings.slop.toLocaleString()} 👎)
      </div>
    </div>
  );
}

/** Votes since the first release with work hired through People (DESIGN §15.4). The rating above
 * stays every vote: hiring people never changes how votes count. */
export function SinceNewWork({ s, min }: { s: NonNullable<Ratings["since_new_work"]>; min: number }) {
  const who = s.credits.join(", ");
  if (!s.since) {
    return (
      <p className="since-work muted small">
        ✋ New work by {who} is coming: a "since the new work" rating starts with the next release.
      </p>
    );
  }
  const total = (s.not_slop ?? 0) + (s.slop ?? 0);
  return (
    <div className="since-work">
      <b>Since the new work</b>{" "}
      <span className="muted small">
        (by {who}, from version {s.version} on {formatDate(s.since)})
      </span>
      <div>
        {s.percent_weighted !== null && s.percent_weighted !== undefined ? (
          <>
            <span className={`rating-mid ${s.percent_weighted >= 70 ? "good" : s.percent_weighted < 40 ? "bad" : "mixed"}`}>
              {s.percent_weighted}% Not Slop
            </span>{" "}
            <span className="muted small">
              {s.label} · {total} vote{total === 1 ? "" : "s"} cast or changed since
            </span>
          </>
        ) : (
          <span className="muted small">
            {total} vote{total === 1 ? "" : "s"} since; a rating shows from {min}.
          </span>
        )}
      </div>
    </div>
  );
}

export function RatingPanel({ slug, isMine, signedIn }: { slug: string; isMine: boolean; signedIn: boolean }) {
  const loaded = useLoad(() => api.ratings(slug), [slug, signedIn]);
  const players = useLoad(() => api.players(slug), [slug]);
  const [ratings, setRatings] = useState<Ratings>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [weighted, setWeighted] = useState(true);
  const r = ratings ?? loaded.data;
  if (!r) return null;

  const vote = async (v: "not_slop" | "slop" | null) => {
    setBusy(true);
    setError(undefined);
    try {
      setRatings(await api.vote(slug, v));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const mine = r.mine;
  const months = r.monthly.map((m) => m.month);
  const days = r.daily.map((d) => shortDay(d.day));
  const peaks = players.data?.daily_peaks ?? [];

  return (
    <section className="panel panel-pad rating-panel" aria-label="Ratings">
      <h2>Slop or Not Slop?</h2>
      <RatingSummary ratings={r} />
      {r.since_new_work && <SinceNewWork s={r.since_new_work} min={r.min_votes} />}

      {!r.open ? (
        <p className="muted small">Voting opens when the app is released.</p>
      ) : isMine ? (
        <p className="muted small">Makers can't vote on their own apps.</p>
      ) : !signedIn ? (
        <p className="muted small">Sign in and play it from the Slop app to vote.</p>
      ) : mine?.can_vote ? (
        <div className="vote-row">
          <button
            type="button"
            className={`btn vote ${mine.vote === "not_slop" ? "on up" : ""}`}
            aria-pressed={mine.vote === "not_slop"}
            disabled={busy}
            onClick={() => void vote(mine.vote === "not_slop" ? null : "not_slop")}
          >
            👍 Not Slop
          </button>
          <button
            type="button"
            className={`btn vote ${mine.vote === "slop" ? "on down" : ""}`}
            aria-pressed={mine.vote === "slop"}
            disabled={busy}
            onClick={() => void vote(mine.vote === "slop" ? null : "slop")}
          >
            👎 Slop
          </button>
          <span className="muted small">
            Your vote counts {Math.round(mine.weight * 100)}% (more as you play). Press again to take it back.
          </span>
        </div>
      ) : (
        <p className="muted small">{mine?.reason}</p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      {r.monthly.length > 0 && (
        <>
          <div className="seg" role="group" aria-label="Rating shown">
            <button type="button" className={weighted ? "on" : ""} aria-pressed={weighted} onClick={() => setWeighted(true)}>
              Weighted by playtime
            </button>
            <button type="button" className={!weighted ? "on" : ""} aria-pressed={!weighted} onClick={() => setWeighted(false)}>
              One account, one vote
            </button>
          </div>
          <LineChart
            title="% Not Slop by month"
            summary={`% Not Slop at the end of each month from ${months[0]} to ${months[months.length - 1]}.`}
            labels={months}
            max={100}
            unit="%"
            series={[
              {
                name: weighted ? "Weighted" : "Unweighted",
                values: r.monthly.map((m) => (weighted ? m.percent_weighted : m.percent)),
                className: "line-main",
              },
            ]}
          />
        </>
      )}
      {r.daily.length > 0 && (
        <UpDownBars
          title="Votes per day (90 days)"
          summary={`Votes cast per day over the last 90 days: ${r.daily.reduce((n, d) => n + d.not_slop, 0)} Not Slop and ${r.daily.reduce((n, d) => n + d.slop, 0)} Slop.`}
          labels={days}
          up={r.daily.map((d) => d.not_slop)}
          down={r.daily.map((d) => d.slop)}
          upName="👍 Not Slop"
          downName="👎 Slop"
        />
      )}
      {peaks.length > 0 && (
        <LineChart
          title="Players, busiest moment each day (30 days)"
          summary={`The most players at once each day over the last 30 days; the highest was ${Math.max(...peaks.map((p) => p.peak))}.`}
          labels={peaks.map((p) => shortDay(p.day))}
          max={Math.max(5, ...peaks.map((p) => p.peak))}
          series={[{ name: "Players", values: peaks.map((p) => p.peak), className: "line-players" }]}
        />
      )}
    </section>
  );
}
