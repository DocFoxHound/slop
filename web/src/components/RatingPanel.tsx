// SPDX-License-Identifier: AGPL-3.0-or-later
// 👍 Not Slop / 👎 Slop on an app's page: the rating (weighted by playtime, with the unweighted
// one beside it), the vote buttons, and the rating, votes and players over time
// (crates/slop-api/src/votes.rs, pulse.rs; DESIGN §7, METRICS §2).

import { useEffect, useState, type FormEvent } from "react";
import { Link } from "react-router";

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
          · a rating shows from {ratings.min_votes} votes · <Link to="/ratings">how ratings work</Link>
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
        <Link to="/ratings">weighted by playtime</Link> · {ratings.percent}% unweighted · {total.toLocaleString()} votes ({ratings.not_slop.toLocaleString()}{" "}
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

/** Fired on window (detail: the app's address) when the viewer's vote or review changed. */
export const RATINGS_CHANGED = "slop:ratings-changed";

function changed(slug: string) {
  window.dispatchEvent(new CustomEvent(RATINGS_CHANGED, { detail: slug }));
}

/** An app's ratings, reloaded when the viewer votes somewhere else on the page. */
function useRatings(slug: string, signedIn: boolean) {
  const loaded = useLoad(() => api.ratings(slug), [slug, signedIn]);
  const reload = loaded.reload;
  useEffect(() => {
    const onChange = (e: Event) => {
      if ((e as CustomEvent<string>).detail === slug) reload();
    };
    window.addEventListener(RATINGS_CHANGED, onChange);
    return () => window.removeEventListener(RATINGS_CHANGED, onChange);
  }, [slug, reload]);
  return loaded;
}

function tone(percent: number): "good" | "bad" | "mixed" {
  return percent >= 70 ? "good" : percent < 40 ? "bad" : "mixed";
}

const VOTE_LABEL = { not_slop: "👍 Not Slop", slop: "👎 Slop" } as const;

/** Moves to the rating panel (id "rating") and puts focus there. */
export function jumpToRating() {
  const panel = document.getElementById("rating");
  if (!panel) return;
  panel.scrollIntoView({ block: "start", behavior: "smooth" });
  panel.focus({ preventScroll: true });
}

/**
 * The rating at the top of an app's page: "96% Not Slop · Overwhelmingly Not Slop · 2,914 votes",
 * and what the viewer can do about it: rate it (jumping to the panel), or why they can't yet.
 */
export function RatingHeader({ slug, isMine, signedIn }: { slug: string; isMine: boolean; signedIn: boolean }) {
  const r = useRatings(slug, signedIn).data;
  if (!r) return null;
  const total = r.not_slop + r.slop;
  const mine = r.mine;
  const why = !r.open
    ? "Voting opens when it's released."
    : isMine
      ? "Makers can't rate their own app."
      : !signedIn
        ? null
        : mine && !mine.can_vote
          ? mine.reason?.startsWith("Launch it")
            ? "Play it from the Slop app to rate it."
            : mine.reason
          : null;
  return (
    <div className="rating-head">
      <div className="rating-head-score">
        {r.percent_weighted !== null ? (
          <>
            <span className={`rating-big ${tone(r.percent_weighted)}`}>{r.percent_weighted}% Not Slop</span>{" "}
            <span className="rating-head-label">
              {r.label && <b>{r.label}</b>}
              <span className="muted small">
                {r.label && " · "}
                {total.toLocaleString()} vote{total === 1 ? "" : "s"}
              </span>
            </span>
          </>
        ) : (
          <>
            <b>No ratings yet</b>
            {total > 0 && (
              <span className="muted small">
                {" "}
                · {total} vote{total === 1 ? "" : "s"} so far; a rating shows from {r.min_votes}
              </span>
            )}
          </>
        )}
      </div>
      {r.open && !isMine && signedIn && mine?.can_vote ? (
        <button type="button" className={`btn ${mine.vote ? "" : "slime"}`} onClick={jumpToRating}>
          {mine.vote ? `Your rating: ${VOTE_LABEL[mine.vote]}` : "Rate it"}
        </button>
      ) : r.open && !isMine && !signedIn ? (
        <p className="muted small">
          <Link to={`/signin?return_to=${encodeURIComponent(`/app/${slug}`)}`}>Sign in</Link> and play it from the Slop app to
          rate it.
        </p>
      ) : (
        why && <p className="muted small">{why}</p>
      )}
    </div>
  );
}

/** The rating in a few words, for the library's header: "96% Not Slop", or nothing yet. */
export function RatingBadge({ slug, signedIn }: { slug: string; signedIn: boolean }) {
  const r = useRatings(slug, signedIn).data;
  if (!r || r.percent_weighted === null) return null;
  return (
    <span className={`rating-badge ${tone(r.percent_weighted)}`} title={r.label ?? undefined}>
      {r.percent_weighted}% Not Slop
    </span>
  );
}

/**
 * Steam's "would you recommend this?" for the library: once the player has played an app and
 * hasn't voted, ask; once they have, show their vote with Change, and their review.
 */
export function RatePrompt({ slug, title }: { slug: string; title: string }) {
  const loaded = useRatings(slug, true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [changing, setChanging] = useState(false);
  const r = loaded.data;
  const mine = r?.mine;
  if (!r || !r.open || !mine?.can_vote) return null;
  const vote = async (v: "not_slop" | "slop") => {
    setBusy(true);
    setError(undefined);
    try {
      await api.vote(slug, v);
      setChanging(false);
      changed(slug);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const asking = !mine.vote || changing;
  return (
    <section className={`panel panel-pad rate-prompt${asking ? " asking" : ""}`} aria-label={`Rate ${title}`}>
      {asking ? (
        <>
          <h3>{mine.vote ? `Change your rating of ${title}` : `You've played ${title}: is it Slop or Not Slop?`}</h3>
          <div className="vote-row">
            <button
              type="button"
              className={`btn vote ${mine.vote === "not_slop" ? "on up" : ""}`}
              aria-pressed={mine.vote === "not_slop"}
              disabled={busy}
              onClick={() => void vote("not_slop")}
            >
              👍 Not Slop
            </button>
            <button
              type="button"
              className={`btn vote ${mine.vote === "slop" ? "on down" : ""}`}
              aria-pressed={mine.vote === "slop"}
              disabled={busy}
              onClick={() => void vote("slop")}
            >
              👎 Slop
            </button>
            {changing && (
              <button type="button" className="btn small ghost" onClick={() => setChanging(false)}>
                Cancel
              </button>
            )}
          </div>
          <p className="muted small">
            Your vote counts {Math.round(mine.weight * 100)}% (more as you play). <Link to="/ratings">How ratings work</Link>
          </p>
        </>
      ) : (
        <div className="rate-prompt-done">
          <span>
            Your rating: <b>{VOTE_LABEL[mine.vote!]}</b>
          </span>
          <button type="button" className="btn small ghost" onClick={() => setChanging(true)}>
            Change
          </button>
        </div>
      )}
      {mine.vote && !changing && <MyReview slug={slug} review={mine.review} onSaved={async () => changed(slug)} />}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
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
      changed(slug);
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
    <section className="panel panel-pad rating-panel" aria-label="Ratings" id="rating" tabIndex={-1}>
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
      {r.open && signedIn && !isMine && mine?.vote && (
        <MyReview
          slug={slug}
          review={mine.review}
          onSaved={async () => {
            setRatings(await api.ratings(slug));
            changed(slug);
          }}
        />
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
          summary={`Votes cast per day over the last 90 days: ${r.daily.reduce((n, d) => n + d.not_slop, 0)} Not Slop and ${r.daily.reduce((n, d) => n + d.slop, 0)} Slop.${(r.unusual_days ?? []).length > 0 ? ` ${(r.unusual_days ?? []).length} of those days had unusual voting.` : ""}`}
          labels={days}
          up={r.daily.map((d) => d.not_slop)}
          down={r.daily.map((d) => d.slop)}
          upName="👍 Not Slop"
          downName="👎 Slop"
          marked={r.daily.map((d) => (r.unusual_days ?? []).includes(d.day))}
          markedName="Unusual voting"
        />
      )}
      {r.daily.some((d) => (r.unusual_days ?? []).includes(d.day)) && (
        <p className="muted small">
          Shaded days had unusual voting: a sudden wave of 👎, many votes from brand-new accounts, or many players changing
          their vote at once. Slop's staff look into these; nothing is changed automatically.
        </p>
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

/** The voter's own written review, under their vote (crates/slop-api/src/reviews.rs). */
function MyReview({
  slug,
  review,
  onSaved,
}: {
  slug: string;
  review?: { body: string; hidden: boolean };
  onSaved: () => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(review?.body ?? "");
  const [error, setError] = useState<string>();
  const save = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    try {
      await api.reviews.write(slug, text);
      setEditing(false);
      await onSaved();
      window.dispatchEvent(new Event("slop:reviews-changed"));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const remove = async () => {
    setError(undefined);
    try {
      await api.reviews.remove(slug);
      setText("");
      await onSaved();
      window.dispatchEvent(new Event("slop:reviews-changed"));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  if (review && !editing) {
    return (
      <div className="my-review">
        <b className="small">Your review</b>
        {review.hidden && (
          <p className="small warn">
            Hidden by a moderation decision: see why on your <Link to="/account/moderation#decisions">account page</Link>.
          </p>
        )}
        <p className="pre">{review.body}</p>
        <span className="row-actions">
          <button type="button" className="link-btn small" onClick={() => setEditing(true)}>
            Edit
          </button>
          <button type="button" className="link-btn small" onClick={() => void remove()}>
            Delete
          </button>
        </span>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    );
  }
  if (!editing) {
    return (
      <button type="button" className="link-btn small" onClick={() => setEditing(true)}>
        Add a written review
      </button>
    );
  }
  return (
    <form className="form my-review" onSubmit={(e) => void save(e)}>
      <label>
        Your review
        <textarea value={text} rows={4} maxLength={4000} onChange={(e) => setText(e.target.value)} />
        <span className="hint">Shown with your vote. Honest and about the app; nobody may offer you anything for it.</span>
      </label>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="row-actions">
        <button className="btn small slime" disabled={!text.trim()}>
          Save review
        </button>
        <button type="button" className="btn small ghost" onClick={() => setEditing(false)}>
          Cancel
        </button>
      </div>
    </form>
  );
}
