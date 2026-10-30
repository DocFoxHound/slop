// SPDX-License-Identifier: AGPL-3.0-or-later
// Friends (crates/slop-api/src/friends.rs): requests, the friends list, blocked people, and who may
// message you. Works on the website too; chatting itself is in the app (pages/Chat.tsx).

import { useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router";

import { api, ApiError, type Friends } from "../api";
import { ErrorState, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { useSession } from "../session";
import { ensureChat } from "./Chat";
import { InlineIcon } from "../components/StoreIcons";

export function FriendsPage() {
  useDocumentTitle("Friends");
  const { me } = useSession();
  const navigate = useNavigate();
  const friends = useLoad(() => (me ? api.friends.list() : Promise.resolve(undefined)), [me?.id]);
  const blocks = useLoad(() => (me ? api.friends.blocks() : Promise.resolve(undefined)), [me?.id]);
  const settings = useLoad(() => (me ? api.friends.chatSettings() : Promise.resolve(undefined)), [me?.id]);
  const [link, setLink] = useState("");
  const [note, setNote] = useState<string>();
  const [error, setError] = useState<string>();

  if (me === undefined) return <Loading />;
  if (me === null) {
    return (
      <div className="page narrow">
        <div className="empty-state">
          <Link to="/signin?return_to=/friends">Sign in</Link> to see your friends.
        </div>
      </div>
    );
  }
  if (friends.error) return <ErrorState error={friends.error} onRetry={friends.reload} />;

  const run = async (f: () => Promise<unknown>, done?: string) => {
    setError(undefined);
    setNote(undefined);
    try {
      await f();
      if (done) setNote(done);
      friends.reload();
      blocks.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const add = (e: FormEvent) => {
    e.preventDefault();
    void run(async () => {
      const res = await api.friends.request(link.trim());
      setLink("");
      setNote(res.status === "friends" ? "You're friends now." : "Request sent. They'll see it in their notifications.");
    });
  };
  const message = async (id: string) => {
    try {
      navigate(await startDirectChat(id));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };
  const f = friends.data;
  return (
    <div className="page narrow">
      <h1 className="page-title">
        <InlineIcon name="friends" /> Friends
      </h1>
      <p className="muted">
        Friends can message each other in the SLAPorSLOP app, along with teammates and the people you work with on People.{" "}
        {!isDesktop && <Link to="/chat">About chat</Link>}
      </p>
      <form className="panel panel-pad form inline-form wide" onSubmit={add}>
        <label>
          Add a friend: paste their profile link
          <input value={link} onChange={(e) => setLink(e.target.value)} placeholder="https://…/u/…" />
        </label>
        <button className="btn slime" disabled={!link.trim()}>
          Send request
        </button>
        {note && <p className="muted small">{note}</p>}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </form>
      <FriendsList friends={f} run={run} onMessage={isDesktop ? (id) => void message(id) : undefined} />
      <h2 className="section-title" id="settings">
        Who can message you
      </h2>
      <div className="panel panel-pad">
        {settings.data?.locked ? (
          <p className="muted">Friends, teammates and People job partners. In the under-18 view nobody else can message you.</p>
        ) : (
          <label className="check">
            <input
              type="checkbox"
              checked={settings.data?.from_anyone ?? false}
              onChange={(e) => void run(() => api.friends.setChatSettings(e.target.checked)).then(settings.reload)}
            />
            <span>Anyone on SLAPorSLOP (otherwise only friends, teammates and People job partners)</span>
          </label>
        )}
      </div>
      {blocks.data && blocks.data.length > 0 && (
        <>
          <h2 className="section-title">Blocked</h2>
          <p className="muted small">Blocked people can't message you or send friend requests, and their chats leave your list.</p>
          <ul className="panel report-list">
            {blocks.data.map((p) => (
              <li key={p.id}>
                <span>{p.name}</span>
                <button className="btn small ghost" onClick={() => void run(() => api.friends.unblock(p.id))}>
                  Unblock
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

/**
 * Requests, friends and sent requests. The Friends page shows everything; the Friends & Chat
 * panel (components/ChatDock.tsx) is compact: requests to answer and friends to message.
 */
export function FriendsList({
  friends: f,
  run,
  compact = false,
  onMessage,
}: {
  friends: Friends | undefined;
  run: (f: () => Promise<unknown>, done?: string) => void;
  compact?: boolean;
  /** Message a friend (the app only). */
  onMessage?: (id: string) => void;
}) {
  if (!f) return <Loading />;
  const Heading = compact ? "h3" : "h2";
  return (
    <>
      {f.incoming.length > 0 && (
        <section id={compact ? undefined : "requests"}>
          <Heading className="section-title">Requests ({f.incoming.length})</Heading>
          <ul className="panel report-list">
            {f.incoming.map((p) => (
              <li key={p.id} id={compact ? undefined : p.id}>
                <span>
                  <Link to={`/u/${p.id}`}>
                    <b>{p.name}</b>
                  </Link>{" "}
                  {!compact && <span className="muted small">asked {formatDate(p.since)}</span>}
                </span>
                <span className="row-actions">
                  <button className="btn small slime" onClick={() => run(() => api.friends.accept(p.id), `You and ${p.name} are friends.`)}>
                    Accept
                  </button>
                  <button className="btn small ghost" onClick={() => run(() => api.friends.remove(p.id))}>
                    Decline
                  </button>
                  {!compact && (
                    <button className="btn small ghost" onClick={() => run(() => api.friends.block(p.id), `${p.name} is blocked.`)}>
                      Block
                    </button>
                  )}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <Heading className="section-title">Your friends ({f.friends.length})</Heading>
      {f.friends.length === 0 ? (
        <div className="empty-state">No friends yet. Add someone with their profile link, or from their profile.</div>
      ) : (
        <ul className="panel report-list">
          {f.friends.map((p) => (
            <li key={p.id}>
              <span>
                <Link to={`/u/${p.id}`}>
                  <b>{p.name}</b>
                </Link>{" "}
                {!compact && <span className="muted small">since {formatDate(p.since)}</span>}
              </span>
              <span className="row-actions">
                {onMessage && (
                  <button className="btn small" onClick={() => onMessage(p.id)} aria-label={`Message ${p.name}`}>
                    <InlineIcon name="chat" /> Message
                  </button>
                )}
                {!compact && (
                  <button className="btn small ghost" onClick={() => run(() => api.friends.remove(p.id))}>
                    Remove
                  </button>
                )}
              </span>
            </li>
          ))}
        </ul>
      )}
      {!compact && f.outgoing.length > 0 && (
        <>
          <h2 className="section-title">Waiting for an answer</h2>
          <ul className="panel report-list">
            {f.outgoing.map((p) => (
              <li key={p.id}>
                <Link to={`/u/${p.id}`}>{p.name}</Link>
                <button className="btn small ghost" onClick={() => run(() => api.friends.remove(p.id))}>
                  Cancel
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </>
  );
}

/** Starts (or opens) a direct chat in the app, setting chat up first if needed; where to go. */
async function startDirectChat(user: string): Promise<string> {
  const status = await ensureChat();
  // Not ready on this computer yet (waiting for approval, or removed): the chat page says why.
  if (status.state !== "active") return "/chat";
  return `/chat/${await desktop.chat<string>({ action: "start_direct", user })}`;
}

/** Friend and message buttons on someone's profile. */
export function ConnectionButtons({ userId, name }: { userId: string; name: string }) {
  const { me } = useSession();
  const navigate = useNavigate();
  const conn = useLoad(() => (me && me.id !== userId ? api.friends.connection(userId) : Promise.resolve(undefined)), [me?.id, userId]);
  const [error, setError] = useState<string>();
  if (!me || me.id === userId || !conn.data) return null;
  const c = conn.data;
  const run = async (f: () => Promise<unknown>) => {
    setError(undefined);
    try {
      await f();
      conn.reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };
  return (
    <div className="row-actions">
      {c.blocked ? (
        <button className="btn small" onClick={() => void run(() => api.friends.unblock(userId))}>
          Unblock {name}
        </button>
      ) : (
        <>
          {c.friend === "none" && (
            <button className="btn small slime" onClick={() => void run(() => api.friends.request(userId))}>
              ＋ Add friend
            </button>
          )}
          {c.friend === "requested" && <span className="muted small">Friend request sent</span>}
          {c.friend === "incoming" && (
            <button className="btn small slime" onClick={() => void run(() => api.friends.accept(userId))}>
              Accept friend request
            </button>
          )}
          {c.friend === "friends" && <span className="muted small">✓ Friends</span>}
          {c.can_message &&
            (isDesktop ? (
              <button
                className="btn small"
                onClick={() => void run(async () => navigate(await startDirectChat(userId)))}
              >
                <InlineIcon name="chat" /> Message
              </button>
            ) : (
              <Link className="btn small" to="/chat">
                <InlineIcon name="chat" /> Message in the app
              </Link>
            ))}
          <button className="btn small ghost" onClick={() => void run(() => api.friends.block(userId))}>
            Block
          </button>
        </>
      )}
      {error && (
        <span className="form-error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
