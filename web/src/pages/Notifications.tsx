// SPDX-License-Identifier: AGPL-3.0-or-later
// The bell's page (crates/slop-api/src/notify.rs): job steps, Interested apps moving on, and
// moderation. Opening one marks it read.

import { Link, useNavigate } from "react-router";

import { api, type Notification } from "../api";
import { ErrorState, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";

const ICON: Record<Notification["category"], string> = {
  jobs: "🎨",
  interested: "★",
  moderation: "⚑",
  donations: "♥",
  shared: "🔒",
  teams: "👥",
  apps: "📦",
  chat: "💬",
  friends: "🤝",
};

export function Notifications() {
  useDocumentTitle("Notifications");
  const { me, refresh } = useSession();
  const navigate = useNavigate();
  const inbox = useLoad(() => (me ? api.notifications.list() : Promise.resolve(undefined)), [me?.id]);
  if (me === null) {
    return (
      <div className="page narrow">
        <div className="empty-state">
          <Link to="/signin?return_to=/notifications">Sign in</Link> to see your notifications.
        </div>
      </div>
    );
  }
  if (inbox.error) return <ErrorState error={inbox.error} onRetry={inbox.reload} />;
  if (!inbox.data) return <Loading />;
  const open = async (n: Notification) => {
    if (!n.read) await api.notifications.read([n.id]).catch(() => undefined);
    void refresh();
    navigate(n.link);
  };
  const readAll = async () => {
    await api.notifications.read();
    inbox.reload();
    void refresh();
  };
  return (
    <div className="page narrow">
      <div className="title-row spread">
        <h1 className="page-title">Notifications</h1>
        {inbox.data.unread > 0 && (
          <button type="button" className="btn small" onClick={() => void readAll()}>
            Mark all as read
          </button>
        )}
      </div>
      {inbox.data.items.length === 0 ? (
        <div className="empty-state">Nothing yet. Job steps, apps you're Interested in and decisions about your content show up here.</div>
      ) : (
        <ul className="notifications panel">
          {inbox.data.items.map((n) => (
            <li key={n.id} className={n.read ? "" : "unread"}>
              <button type="button" className="notification" onClick={() => void open(n)}>
                <span className="n-icon" aria-hidden="true">
                  {ICON[n.category]}
                </span>
                <span className="n-text">
                  <b>{n.title}</b>
                  {n.count > 1 && <span className="muted small"> ({n.count})</span>}
                  {n.body && <span className="muted small n-body">{n.body}</span>}
                  <span className="faint small">
                    {formatDate(n.created_at)}
                    {!n.read && <span className="sr-only"> · unread</span>}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <p className="muted small">
        Choose which of these are also emailed to you on <Link to="/account#notifications">your account page</Link>.
      </p>
    </div>
  );
}
