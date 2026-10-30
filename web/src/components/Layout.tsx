// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, NavLink, Outlet, useLocation } from "react-router";

import { api } from "../api";
import { useLoad } from "../hooks";
import { formatDate } from "../format";
import { desktop, isDesktop } from "../platform";
import { useAutoUpdates } from "./AppUpdates";
import { ChatDock } from "./ChatDock";
import { DownloadsStatus } from "./Downloads";
import { Avatar } from "./Avatar";
import { BrowserBar } from "./BrowserBar";
import { ModRequests } from "./ModRequests";
import { CrashPrompt } from "./BugReports";
import { SaveSyncEvents } from "./Sandbox";
import { UpdateNotice } from "./UpdateNotice";
import { WindowControls } from "./WindowControls";
import { useSession } from "../session";
import { fragmentId } from "../browsing";
import { StoreIcon } from "./StoreIcons";
import type { StoreIconName } from "../storeIcons";

const TABS = [
  {
    to: "/",
    label: "Store",
    icon: "store" as StoreIconName,
    end: true,
    match: (p: string) =>
      !["/publish", "/account", "/library", "/admin", "/people", "/jobs", "/chat", "/friends"].some((prefix) => p.startsWith(prefix)),
  },
  {
    to: "/people",
    label: "People",
    icon: "hire" as StoreIconName,
    end: false,
    match: (p: string) => p.startsWith("/people") || p.startsWith("/jobs"),
  },
  { to: "/library", label: "Library", icon: "library" as StoreIconName, end: false, match: (p: string) => p.startsWith("/library") },
  { to: "/publish", label: "Publish", icon: "publish" as StoreIconName, end: false, match: (p: string) => p.startsWith("/publish") },
];

/**
 * On phone-sized screens, chat is a bottom tab; elsewhere it's the Friends & Chat bar in the
 * bottom right (ChatDock.tsx), so it isn't a top tab.
 */
const CHAT_TAB = {
  to: "/chat",
  label: "Chat",
  icon: "chat" as StoreIconName,
  end: false,
  match: (p: string) => p.startsWith("/chat") || p.startsWith("/friends"),
};
const BOTTOM_TABS = [...TABS.slice(0, 3), CHAT_TAB, ...TABS.slice(3)];

/** Unread chat messages, in the desktop app once chat is running (pages/Chat.tsx). */
function useChatUnread(): number {
  const [unread, setUnread] = useState(0);
  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    const count = () => {
      void desktop
        .chat<Array<{ unread: number; muted: boolean }>>({ action: "conversations" })
        .then((list) => setUnread(list.filter((c) => !c.muted).reduce((n, c) => n + c.unread, 0)))
        .catch(() => undefined);
    };
    // Only after chat reports something (or a chat was read): asking earlier would set chat up
    // on devices that never use it.
    void desktop.onChatChanged(count).then((u) => (unlisten = u));
    window.addEventListener("slop-chat-read", count);
    return () => {
      unlisten?.();
      window.removeEventListener("slop-chat-read", count);
    };
  }, []);
  return unread;
}

export function Layout() {
  const location = useLocation();
  const view = useRef<HTMLElement>(null);
  const chatUnread = useChatUnread();
  const { refresh } = useSession();
  useAutoUpdates();
  // Reload this page (the page bar, F5 in the desktop app): the page is made again, so it loads
  // its data again, without reloading the whole app.
  const [reloads, setReloads] = useState(0);
  const reload = useCallback(() => {
    setReloads((n) => n + 1);
    void refresh();
  }, [refresh]);

  // New page, back to the top. The page scrolls inside <main>, not the window.
  useEffect(() => {
    view.current?.scrollTo({ top: 0 });
  }, [location.pathname]);
  // Links to a part of a page (/account#decisions): wait for it to load, then go there. Parts
  // above it may still be loading and push it down, so it's followed until the page settles
  // (or the person scrolls themselves).
  useEffect(() => {
    const id = fragmentId(location.hash);
    if (!id) return;
    let tries = 0;
    let observer: ResizeObserver | undefined;
    let settled: number | undefined;
    const youScrolled = ["wheel", "touchstart", "keydown", "mousedown"] as const;
    const stop = () => {
      observer?.disconnect();
      observer = undefined;
      window.clearTimeout(settled);
      youScrolled.forEach((e) => window.removeEventListener(e, stop));
    };
    const timer = window.setInterval(() => {
      const el = document.getElementById(id);
      if (el || ++tries > 30) {
        window.clearInterval(timer);
        if (!el) return;
        el.scrollIntoView({ block: "start" });
        const page = view.current?.firstElementChild;
        if (!page || typeof ResizeObserver === "undefined") return;
        observer = new ResizeObserver(() => el.scrollIntoView({ block: "start" }));
        observer.observe(page);
        settled = window.setTimeout(stop, 3000);
        youScrolled.forEach((e) => window.addEventListener(e, stop, { passive: true }));
      }
    }, 100);
    return () => {
      window.clearInterval(timer);
      stop();
    };
  }, [location.pathname, location.hash]);

  return (
    <div className="window">
      <a className="skip" href="#main">
        Skip to content
      </a>
      {/* In the desktop app the window has no system title bar: drag it by the bar's empty space. */}
      <nav className={`nav ${isDesktop ? "titlebar" : ""}`} aria-label="Main" data-tauri-drag-region>
        <Link className="logo" to="/" aria-label="SLAPorSLOP home">
          <img src="/logo.svg" alt="" width={28} height={28} />
          <span>
            SLAP<span className="wm-or">or</span><b>SLOP</b>
          </span>
        </Link>
        {/* The website offers the desktop app; the app itself doesn't need to. */}
        {!isDesktop && (
          <Link className="btn small download-btn" to="/download">
            <StoreIcon name="download" size={14} /> Download
          </Link>
        )}
        <div className="tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={() => (t.match(location.pathname) ? "on" : "")}>
              {t.label.toUpperCase()}
            </NavLink>
          ))}
          <AdminTab />
        </div>
        <div className="grow" data-tauri-drag-region />
        <Bell />
        <MeMenu />
        <WindowControls />
      </nav>
      <BrowserBar onReload={reload} />

      <UpdateNotice />
      <ModRequests />
      <CrashPrompt />
      <SaveSyncEvents />
      <SimulationBanner />
      <AccountNotices />
      <main className="view" id="main" ref={view} tabIndex={-1}>
        <Outlet key={reloads} />
      </main>

      {/* The Library has its own, at the foot of its app list (with names for first installs). */}
      {isDesktop && !location.pathname.startsWith("/library") && (
        <div className="downloads-dock">
          <DownloadsStatus />
        </div>
      )}
      <ChatDock unread={chatUnread} />

      <nav className="bottom-tabs" aria-label="Main">
        {BOTTOM_TABS.map((t) => (
          <NavLink key={t.to} to={t.to} end={t.end} className={() => (t.match(location.pathname) ? "on" : "")}>
            <StoreIcon name={t.icon} size={20} />
            {t.label}
            {t === CHAT_TAB && chatUnread > 0 && (
              <>
                <b className="unread-badge bottom-badge" aria-hidden="true">
                  {chatUnread > 99 ? "99+" : chatUnread}
                </b>
                <span className="sr-only">, {chatUnread} unread</span>
              </>
            )}
          </NavLink>
        ))}
        <NavLink to="/account" className={({ isActive }) => (isActive ? "on" : "")}>
          <StoreIcon name="account" size={20} />
          Account
        </NavLink>
      </nav>
    </div>
  );
}

/** Shown only to SLAPorSLOP staff. */
function AdminTab() {
  const { me } = useSession();
  if (!me?.is_admin) return null;
  return (
    <NavLink to="/admin" className={({ isActive }) => (isActive ? "on" : "")}>
      ADMIN
    </NavLink>
  );
}

function MeMenu() {
  const { me } = useSession();
  const location = useLocation();
  if (me === undefined) return <div className="me" />;
  if (!me) {
    const returnTo = location.pathname + location.search;
    if (isDesktop) {
      return (
        <div className="me">
          <button className="btn slime" onClick={() => void desktop.showLogin()}>
            Sign in
          </button>
        </div>
      );
    }
    return (
      <div className="me">
        <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(returnTo)}`}>
          Sign in
        </Link>
      </div>
    );
  }
  return (
    <Link className="me" to="/account" title="Your account">
      <Avatar name={me.display_name} url={me.avatar_url} />
      <span className="who">
        <b>{me.display_name}</b>
      </span>
    </Link>
  );
}

/** Unread notifications, refreshed as the user moves around (and every minute). */
function Bell() {
  const { me, refresh } = useSession();
  const location = useLocation();
  useEffect(() => {
    if (!me) return;
    const timer = window.setInterval(() => void refresh(), 60_000);
    return () => window.clearInterval(timer);
  }, [me?.id, refresh]);
  if (!me) return null;
  const n = me.unread_notifications;
  return (
    <Link
      to="/notifications"
      className={`bell ${location.pathname === "/notifications" || location.pathname.startsWith("/notifications/") ? "on" : ""}`}
      aria-label={n > 0 ? `Notifications, ${n} unread` : "Notifications"}
      title="Notifications"
    >
      <StoreIcon name="bell" size={20} />
      {n > 0 && (
        <span className="bell-count" aria-hidden="true">
          {n > 99 ? "99+" : n}
        </span>
      )}
    </Link>
  );
}

/**
 * Things the signed-in user has to act on, on every page but never in the way: a suspension, new
 * moderation decisions, and agreements whose new version is in effect and waits to be accepted
 * (crates/slop-api/src/legal.rs). Announced changes and policies go to the bell instead, once.
 */
function AccountNotices() {
  const { me } = useSession();
  const location = useLocation();
  const notices = useLoad(() => (me ? api.legal.notices() : Promise.resolve([])), [me?.id, location.pathname.startsWith("/account")]);
  if (!me) return null;
  const toAccept = (notices.data ?? []).filter((n) => n.in_effect && n.action === "accept");
  if (!me.suspended && me.new_decisions === 0 && toAccept.length === 0) return null;
  return (
    <div className="account-notices" role="status">
      {me.suspended && (
        <div className="account-notice bad-notice">
          <b>Your account is suspended.</b> You can read SLAPorSLOP, download your data and appeal.{" "}
          <Link to="/account/moderation#decisions">See why, and appeal</Link>
        </div>
      )}
      {!me.suspended && me.new_decisions > 0 && (
        <div className="account-notice">
          SLAPorSLOP made {me.new_decisions === 1 ? "a decision" : `${me.new_decisions} decisions`} about your content.{" "}
          <Link to="/account/moderation#decisions">See what, why, and how to appeal</Link>
        </div>
      )}
      {toAccept.length === 1 ? (
        <div className="account-notice">
          {toAccept[0].first
            ? `SLAPorSLOP has published its ${toAccept[0].title}.`
            : `The ${toAccept[0].title} changed on ${formatDate(toAccept[0].effective_at)}.`}{" "}
          {!toAccept[0].first && <span className="muted">{toAccept[0].summary}</span>}{" "}
          <Link to={`/legal/${toAccept[0].kind}`}>Review and accept</Link>
        </div>
      ) : (
        toAccept.length > 1 && (
          <div className="account-notice">
            New versions to review and accept:{" "}
            {toAccept.map((n, i) => (
              <span key={n.kind}>
                {i > 0 && " · "}
                <Link to={`/legal/${n.kind}`}>{n.title}</Link>
              </span>
            ))}
          </div>
        )
      )}
    </div>
  );
}

/** On the simulation sandbox, says so everywhere, with the way to view as a simulated user. */
function SimulationBanner() {
  const server = useLoad(() => api.server(), []);
  if (!server.data?.simulation) return null;
  return (
    <div className="sim-banner" role="note">
      <b>Simulation sandbox.</b> Every app, player and vote here is made up, to test recommendations and ratings.{" "}
      <Link to="/sim">View as a simulated user</Link>
    </div>
  );
}
