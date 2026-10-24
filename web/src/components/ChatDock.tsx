// SPDX-License-Identifier: AGPL-3.0-or-later
// Friends & Chat: a bar in the bottom right of every page for signed-in people, like Steam's.
// It opens a panel docked above it with their chats and friends, while they keep browsing behind
// it; the panel stays open from page to page until it's closed. Chat itself is in the desktop app
// only (pages/Chat.tsx); on the website the panel has friends and says where chat is.
// On phone-sized screens the bar is hidden and Chat is in the bottom tabs (Layout.tsx).

import { useCallback, useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { Link, useLocation } from "react-router";

import { api, ApiError } from "../api";
import { useLoad } from "../hooks";
import { ChatInAppNote, ChatView, ensureChat } from "../pages/Chat";
import { FriendsList } from "../pages/Friends";
import { desktop, isDesktop } from "../platform";
import { useSession } from "../session";

type Tab = "chats" | "friends";

export function ChatDock({ unread }: { unread: number }) {
  const { me } = useSession();
  const location = useLocation();
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<Tab>(isDesktop ? "chats" : "friends");
  const [selected, setSelected] = useState<string>();
  const bar = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const panelId = useId();
  const titleId = useId();
  // The chat and friends pages are the full version of the panel.
  const onChatPage = location.pathname.startsWith("/chat");

  const close = useCallback(() => {
    setOpen(false);
    bar.current?.focus();
  }, []);
  // Going to the full chat page (say, from "Open full chat") takes the chat there.
  useEffect(() => {
    if (onChatPage) setOpen(false);
  }, [onChatPage]);
  // Opening the panel moves focus into it, so keyboard and screen reader users land there.
  useEffect(() => {
    if (open) panel.current?.focus();
  }, [open]);

  if (!me || onChatPage) return null;

  const onKeyDown = (e: KeyboardEvent) => {
    // Escape closes the panel, unless something inside used it (a dialog, cancelling a reply).
    if (e.key !== "Escape" || e.defaultPrevented || document.querySelector("dialog[open]")) return;
    e.preventDefault();
    close();
  };

  const label = unread > 0 ? `Friends & Chat, ${unread} unread` : "Friends & Chat";
  return (
    <div className="chat-dock" onKeyDown={onKeyDown}>
      {open && (
        <div className="chat-dock-panel" id={panelId} role="dialog" aria-labelledby={titleId} ref={panel} tabIndex={-1}>
          <header className="chat-dock-head">
            <h2 id={titleId}>Friends &amp; Chat</h2>
            <Link className="btn small ghost" to={isDesktop ? (tab === "chats" && selected ? `/chat/${selected}` : "/chat") : "/friends"}>
              Full page ↗
            </Link>
            <button type="button" className="btn small ghost" onClick={close} aria-label="Close Friends & Chat">
              ✕
            </button>
          </header>
          {isDesktop && (
            <div className="seg chat-dock-tabs" role="tablist" aria-label="Friends & Chat">
              {(["chats", "friends"] as const).map((t) => (
                <button
                  key={t}
                  type="button"
                  role="tab"
                  id={`${panelId}-${t}`}
                  aria-selected={tab === t}
                  aria-controls={`${panelId}-body`}
                  className={tab === t ? "on" : ""}
                  onClick={() => setTab(t)}
                >
                  {t === "chats" ? "Chats" : "Friends"}
                  {t === "chats" && unread > 0 && (
                    <>
                      <span className="unread-badge tab-badge" aria-hidden="true">
                        {unread > 99 ? "99+" : unread}
                      </span>
                      <span className="sr-only">, {unread} unread</span>
                    </>
                  )}
                </button>
              ))}
            </div>
          )}
          <div
            className="chat-dock-body"
            id={`${panelId}-body`}
            role={isDesktop ? "tabpanel" : undefined}
            aria-labelledby={isDesktop ? `${panelId}-${tab}` : undefined}
          >
            {isDesktop && tab === "chats" ? (
              <ChatView compact selected={selected} onOpen={setSelected} onBack={() => setSelected(undefined)} />
            ) : (
              <DockFriends
                onMessage={
                  isDesktop
                    ? (conversation) => {
                        setSelected(conversation);
                        setTab("chats");
                      }
                    : undefined
                }
              />
            )}
          </div>
        </div>
      )}
      <button
        ref={bar}
        type="button"
        className={`chat-dock-bar ${open ? "on" : ""}`}
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        aria-label={label}
        onClick={() => (open ? close() : setOpen(true))}
      >
        <span aria-hidden="true">💬</span>
        <span>Friends &amp; Chat</span>
        {unread > 0 && (
          <span className="unread-badge" aria-hidden="true">
            {unread > 99 ? "99+" : unread}
          </span>
        )}
        <span className="chat-dock-caret" aria-hidden="true">
          {open ? "▾" : "▴"}
        </span>
      </button>
    </div>
  );
}

/** Friends in the panel: requests to answer and friends to message (the app) or see. */
function DockFriends({ onMessage }: { onMessage?: (conversation: string | undefined) => void }) {
  const friends = useLoad(() => api.friends.list(), []);
  const [note, setNote] = useState<string>();
  const [error, setError] = useState<string>();
  const run = async (f: () => Promise<unknown>, done?: string) => {
    setError(undefined);
    setNote(undefined);
    try {
      await f();
      if (done) setNote(done);
      friends.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  // Messaging a friend opens their chat in the panel, setting chat up first if it needs it.
  const message = onMessage
    ? (user: string) =>
        void run(async () => {
          const status = await ensureChat();
          onMessage(status.state === "active" ? await desktop.chat<string>({ action: "start_direct", user }) : undefined);
        })
    : undefined;
  return (
    <div className="dock-friends">
      {!isDesktop && <ChatInAppNote compact />}
      {note && (
        <p className="muted small" role="status">
          {note}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {friends.error ? (
        <p className="form-error" role="alert">
          {friends.error.message}
        </p>
      ) : (
        <FriendsList compact friends={friends.data} run={(f, done) => void run(f, done)} onMessage={message} />
      )}
      <p className="small dock-foot">
        <Link to="/friends">Add or manage friends</Link>
      </p>
    </div>
  );
}
