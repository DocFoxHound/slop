// Written reviews on an app's page (crates/slop-api/src/reviews.rs). Each shows the vote it goes
// with; a private profile's review shows as "A player". Never hidden for being negative.

import { useEffect, useState } from "react";
import { Link } from "react-router";

import { api, type AppReview } from "../api";
import { formatDate } from "../format";
import { ReportButton } from "./Report";

export function Reviews({ slug }: { slug: string }) {
  const [filter, setFilter] = useState<"" | "not_slop" | "slop">("");
  const [reviews, setReviews] = useState<AppReview[]>();
  const [counts, setCounts] = useState({ not_slop: 0, slop: 0 });
  const [next, setNext] = useState<number | null>(null);
  const [nonce, setNonce] = useState(0);
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
    });
    return () => {
      current = false;
    };
  }, [slug, filter, nonce]);
  const more = async () => {
    if (next === null) return;
    const page = await api.reviews.list(slug, filter || undefined, next);
    setReviews((r) => [...(r ?? []), ...page.reviews]);
    setNext(page.next_offset);
  };
  if (!reviews || counts.not_slop + counts.slop === 0) return null;
  return (
    <section className="panel panel-pad app-reviews" aria-labelledby="reviews-title">
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
              <span className="faint small">
                {formatDate(r.updated_at)}
                {r.edited && " · edited"}
              </span>
            </div>
            <p className="pre">{r.body}</p>
            {!r.mine && <ReportButton kind="app_review" subject={r.id} label="this review" />}
          </li>
        ))}
      </ul>
      {next !== null && (
        <button type="button" className="btn small" onClick={() => void more()}>
          More reviews
        </button>
      )}
    </section>
  );
}
