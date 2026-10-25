// SPDX-License-Identifier: AGPL-3.0-or-later
// Written reviews on an app's page (crates/slop-api/src/reviews.rs). Each shows the vote it goes
// with; a private profile's review shows as "A player". Never hidden for being negative.

import { useEffect, useState } from "react";
import { Link } from "react-router";

import { api, ApiError, type AppReview } from "../api";
import { formatDate } from "../format";
import { ReportButton } from "./Report";

export function Reviews({ slug }: { slug: string }) {
  const [filter, setFilter] = useState<"" | "not_slop" | "slop">("");
  const [reviews, setReviews] = useState<AppReview[]>();
  const [counts, setCounts] = useState({ not_slop: 0, slop: 0 });
  const [next, setNext] = useState<number | null>(null);
  const [nonce, setNonce] = useState(0);
  const [canReply, setCanReply] = useState(false);
  const [moreError, setMoreError] = useState<string>();
  useEffect(() => {
    const changed = () => setNonce((n) => n + 1);
    window.addEventListener("slop:reviews-changed", changed);
    return () => window.removeEventListener("slop:reviews-changed", changed);
  }, []);
  useEffect(() => {
    let current = true;
    api.reviews.list(slug, filter || undefined).then((page) => {
      if (!current) return;
      setReviews(page.reviews);
      setCounts({ not_slop: page.not_slop, slop: page.slop });
      setNext(page.next_offset);
      setCanReply(page.can_reply);
    });
    return () => {
      current = false;
    };
  }, [slug, filter, nonce]);
  const more = async () => {
    if (next === null) return;
    setMoreError(undefined);
    try {
      const page = await api.reviews.list(slug, filter || undefined, next);
      setReviews((r) => [...(r ?? []), ...page.reviews]);
      setNext(page.next_offset);
    } catch (err) {
      setMoreError(err instanceof ApiError ? err.message : String(err));
    }
  };
  if (!reviews || counts.not_slop + counts.slop === 0) return null;
  return (
    <section className="panel panel-pad app-reviews" id="reviews" aria-labelledby="reviews-title">
      <div className="title-row spread">
        <h2 id="reviews-title">Reviews</h2>
        <div className="seg small-seg" role="group" aria-label="Show">
          {(
            [
              ["", `All ${counts.not_slop + counts.slop}`],
              ["not_slop", `👍 ${counts.not_slop}`],
              ["slop", `👎 ${counts.slop}`],
            ] as const
          ).map(([key, label]) => (
            <button key={key} type="button" className={filter === key ? "on" : ""} aria-pressed={filter === key} onClick={() => setFilter(key)}>
              {label}
            </button>
          ))}
        </div>
      </div>
      <ul className="review-list">
        {reviews.map((r) => (
          <li key={r.id} id={`review-${r.id}`}>
            <div className="review-who">
              {r.author_id ? <Link to={`/u/${r.author_id}`}>{r.author}</Link> : <span className="muted">{r.author}</span>}
              <span className={`vote-chip ${r.vote === "not_slop" ? "up" : "down"}`}>{r.vote === "not_slop" ? "👍 Not Slop" : "👎 Slop"}</span>
              {r.in_beta && (
                <span className="badge neutral" title="This player's vote was given while the app was in Beta">
                  RATED IN BETA
                </span>
              )}
              <span className="faint small">
                {formatDate(r.updated_at)}
                {r.edited && " · edited"}
              </span>
            </div>
            <p className="pre">{r.body}</p>
            {r.reply && (
              <div className="review-reply">
                <div className="review-who">
                  <b>The maker replied</b>
                  <span className="faint small">
                    {r.reply_at && formatDate(r.reply_at)}
                    {r.reply_outdated && " · to an earlier version of this review"}
                  </span>
                </div>
                <p className="pre">{r.reply}</p>
              </div>
            )}
            {canReply && (
              <ReplyForm slug={slug} review={r} onDone={() => setNonce((n) => n + 1)} />
            )}
            {!r.mine && <ReportButton kind="app_review" subject={r.id} label="this review or its reply" />}
            {r.mine && r.reply && <ReportButton kind="app_review" subject={r.id} label="the maker's reply" />}
          </li>
        ))}
      </ul>
      {next !== null && (
        <button type="button" className="btn small" onClick={() => void more()}>
          More reviews
        </button>
      )}
      {moreError && (
        <p className="form-error" role="alert">
          {moreError}
        </p>
      )}
    </section>
  );
}

/** The app's maker writes, changes or takes back their reply under a review. */
function ReplyForm({ slug, review, onDone }: { slug: string; review: AppReview; onDone: () => void }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState(review.reply ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      setOpen(false);
      onDone();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't save the reply.");
    } finally {
      setBusy(false);
    }
  };
  if (!open) {
    return (
      <button type="button" className="btn small ghost" onClick={() => setOpen(true)}>
        {review.reply ? "Change your reply" : "Reply"}
      </button>
    );
  }
  return (
    <form
      className="form review-reply-form"
      onSubmit={(e) => {
        e.preventDefault();
        void run(() => api.reviews.reply(slug, review.id, text));
      }}
    >
      <label>
        Your reply, shown under the review
        <textarea value={text} maxLength={2000} rows={3} required onChange={(e) => setText(e.target.value)} />
      </label>
      <p className="faint small">Thank them, say what you fixed or plan. Never ask for a better vote or offer anything for one.</p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="row-actions">
        <button className="btn small slime" disabled={busy || !text.trim()}>
          Post reply
        </button>
        {review.reply && (
          <button type="button" className="btn small ghost" disabled={busy} onClick={() => void run(() => api.reviews.removeReply(slug, review.id))}>
            Delete reply
          </button>
        )}
        <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
    </form>
  );
}
