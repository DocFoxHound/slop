// SPDX-License-Identifier: AGPL-3.0-or-later
// The admin panel's first page: everything waiting for staff, with a count and a link, and one
// box to find an app or an account.

import { useState } from "react";
import { Link, useNavigate } from "react-router";

import { api } from "../../api";
import { ErrorState, Loading } from "../../components/ui";
import { useLoad } from "../../hooks";

export function AdminHome() {
  const queue = useLoad(() => api.admin.queue(), []);
  const holds = useLoad(() => api.admin.donationHolds("open"), []);
  const actions = useLoad(() => api.admin.actions(), []);
  const ops = useLoad(() => api.admin.ops(), []);
  if (queue.error) return <ErrorState error={queue.error} onRetry={queue.reload} />;
  if (!queue.data) return <Loading />;
  const q = queue.data;
  const waiting: Array<[number, string, string]> = [
    [ops.data?.length ?? 0, "Server alerts", "/admin?tab=queue"],
    [actions.data?.actions.filter((a) => a.status === "pending").length ?? 0, "Waiting for a second approval", "/admin?tab=queue"],
    [q.reports.length, "Reports", "/admin?tab=queue#reports"],
    [q.appeals.length, "Appeals", "/admin?tab=queue#appeals"],
    [holds.data?.filter((h) => h.requests.some((r) => r.outcome === null)).length ?? 0, "Makers asking for donations back", "/admin?tab=money"],
    [q.rescans.length, "Live releases a re-scan flagged", "/admin?tab=queue"],
    [q.releases.length, "Releases to review", "/admin?tab=queue"],
    [q.changes.length, "Changes to review", "/admin?tab=queue"],
    [q.media.length, "Pictures to review", "/admin?tab=queue"],
    [q.portfolios.length, "People portfolios to review", "/admin?tab=queue"],
    [q.problems.length, "People problems", "/admin?tab=queue"],
    [q.review_disputes.length, "Review disputes", "/admin?tab=queue"],
    [q.abuse.length, "Possible abuse", "/admin?tab=queue"],
    [q.ready_to_list.length, "Ready for the store", "/admin?tab=queue"],
  ];
  const todo = waiting.filter(([n]) => n > 0);
  return (
    <>
      <Find />
      <section>
        <h2 className="section-title">Needs you</h2>
        {todo.length === 0 ? (
          <div className="empty-state">Nothing waiting. 🎉</div>
        ) : (
          <ul className="admin-todo">
            {todo.map(([n, label, to]) => (
              <li key={label}>
                <Link to={to} className="panel panel-pad">
                  <span className="admin-count">{n}</span> {label}
                </Link>
              </li>
            ))}
          </ul>
        )}
        {holds.data && holds.data.length > 0 && (
          <p className="muted small">
            {holds.data.length} apps have donations paused. <Link to="/admin?tab=money">See them under Money</Link>.
          </p>
        )}
      </section>
    </>
  );
}

function Find() {
  const [text, setText] = useState("");
  const navigate = useNavigate();
  const go = (tab: "apps" | "users") => {
    if (text.trim()) void navigate(`/admin?tab=${tab}&q=${encodeURIComponent(text.trim())}`);
  };
  return (
    <form
      className="filters"
      role="search"
      onSubmit={(e) => {
        e.preventDefault();
        go("apps");
      }}
    >
      <label>
        Find{" "}
        <input value={text} placeholder="An app, a maker, an account ID…" onChange={(e) => setText(e.target.value)} />
      </label>
      <button className="btn small">Apps</button>
      <button type="button" className="btn small ghost" onClick={() => go("users")}>
        Accounts
      </button>
    </form>
  );
}
