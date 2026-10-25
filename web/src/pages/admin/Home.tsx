// SPDX-License-Identifier: AGPL-3.0-or-later
// The admin panel's first page: everything waiting for staff, with a count and a link to its
// category in the review queue (Inbox.tsx), and one box to find an app or an account.

import { useState } from "react";
import { Link, useNavigate } from "react-router";

import { api } from "../../api";
import { ErrorState, Loading } from "../../components/ui";
import { useLoad } from "../../hooks";
import { BOXES, inboxUrl } from "./queue";
import type { Waiting } from "./Inbox";

export function AdminHome({ waiting }: { waiting: Waiting }) {
  const ops = useLoad(() => api.admin.ops(), []);
  if (waiting.queue.error) return <ErrorState error={waiting.queue.error} onRetry={waiting.reload} />;
  if (!waiting.queue.data) return <Loading />;
  const todo = BOXES.map((b) => [waiting.counts[b.key] ?? 0, b.label, inboxUrl(b.key)] as const).filter(([n]) => n > 0);
  const alerts = ops.data?.length ?? 0;
  const paused = waiting.holds.data?.length ?? 0;
  return (
    <>
      <Find />
      <section>
        <h2 className="section-title">
          Needs you{" "}
          {waiting.total !== undefined && waiting.total > 0 && <span className="muted">({waiting.total} waiting)</span>}
        </h2>
        {todo.length === 0 && alerts === 0 ? (
          <div className="empty-state">Nothing waiting. 🎉</div>
        ) : (
          <ul className="admin-todo">
            {alerts > 0 && (
              <li>
                <Link to={inboxUrl()} className="panel panel-pad">
                  <span className="admin-count">{alerts}</span> Server alerts
                </Link>
              </li>
            )}
            {todo.map(([n, label, to]) => (
              <li key={label}>
                <Link to={to} className="panel panel-pad">
                  <span className="admin-count">{n}</span> {label}
                </Link>
              </li>
            ))}
          </ul>
        )}
        {paused > 0 && (
          <p className="muted small">
            {paused} apps have donations paused. <Link to="/admin?tab=money">See them under Money</Link>.
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
