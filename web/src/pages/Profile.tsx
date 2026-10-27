// SPDX-License-Identifier: AGPL-3.0-or-later
// A player's public profile (crates/slop-api/src/reviews.rs): their votes and reviews when they
// keep them public (the default; private by default in the UK under-18 view), their apps, and
// their People page.

import { Link, useParams } from "react-router";

import { api } from "../api";
import { Avatar } from "../components/Avatar";
import { AppTile, ErrorState, Loading, Shelf } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { ConnectionButtons } from "./Friends";

export function Profile() {
  const { id = "" } = useParams();
  const profile = useLoad(() => api.profile(id), [id]);
  useDocumentTitle(profile.data?.name);
  if (profile.error) return <ErrorState error={profile.error} onRetry={profile.reload} />;
  if (!profile.data) return <Loading />;
  const p = profile.data;
  const month = new Date(p.joined).toLocaleDateString("en-US", { month: "long", year: "numeric" });
  return (
    <div className="page profile-page">
      <div className="profile-head">
        <Avatar name={p.name} url={p.avatar_url} className="avatar big" />
        <div>
          <h1 className="page-title">{p.name}</h1>
          <p className="muted small">
            On SLAPorSLOP since {month}
            {p.creative_handle && (
              <>
                {" "}
                · <Link to={`/people/${p.creative_handle}`}>Hire them on People</Link>
              </>
            )}
          </p>
          <ConnectionButtons userId={id} name={p.name} />
        </div>
      </div>
      {p.is_me && (
        <p className="notice">
          This is how others see you.{" "}
          {p.public
            ? "Your votes and reviews are public."
            : "Your votes and reviews are private: others see only your name and apps."}{" "}
          <Link to="/account#privacy">Change it</Link>
        </p>
      )}
      {p.apps.length > 0 && (
        <>
          <h2 className="section-title">Apps they make</h2>
          <Shelf apps={p.apps} empty="" />
        </>
      )}
      <h2 className="section-title">Votes and reviews</h2>
      {!p.public && !p.is_me ? (
        <div className="empty-state">{p.name} keeps their votes and reviews private.</div>
      ) : p.votes.length === 0 ? (
        <div className="empty-state">No votes yet.</div>
      ) : (
        <ul className="profile-votes">
          {p.votes.map((v) => (
            <li key={v.slug} className="panel">
              <div className="profile-vote-tile">
                <AppTile app={v} />
              </div>
              <div className="profile-vote-text">
                <span className={`vote-chip ${v.my_vote === "not_slop" ? "up" : "down"}`}>
                  {v.my_vote === "not_slop" ? "👍 Slaps" : "👎 Slop"}
                </span>{" "}
                <span className="faint small">{formatDate(v.voted_at)}</span>
                {v.review && <p className="pre">{v.review}</p>}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
