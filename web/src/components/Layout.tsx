import { useEffect, useRef, useState, type FormEvent } from "react";
import { Link, NavLink, Outlet, useLocation, useNavigate } from "react-router";

import { api, type Category } from "../api";
import { useLoad } from "../hooks";
import { formatDate } from "../format";
import { desktop, isDesktop } from "../platform";
import { useAutoUpdates } from "./AppUpdates";
import { UpdateNotice } from "./UpdateNotice";
import { WindowControls } from "./WindowControls";
import { useSession } from "../session";

const TABS = [
  {
    to: "/",
    label: "Store",
    icon: "🛒",
    end: true,
    match: (p: string) =>
      !["/publish", "/account", "/library", "/admin", "/people", "/jobs", "/chat", "/friends"].some((prefix) => p.startsWith(prefix)),
  },
  {
    to: "/people",
    label: "People",
    icon: "🎨",
    end: false,
    match: (p: string) => p.startsWith("/people") || p.startsWith("/jobs"),
  },
  { to: "/library", label: "Library", icon: "📚", end: false, match: (p: string) => p.startsWith("/library") },
  {
    to: "/chat",
    label: "Chat",
    icon: "💬",
    end: false,
    match: (p: string) => p.startsWith("/chat") || p.startsWith("/friends"),
  },
  { to: "/publish", label: "Publish", icon: "⬆️", end: false, match: (p: string) => p.startsWith("/publish") },
];

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
  useAutoUpdates();

  // New page, back to the top. The page scrolls inside <main>, not the window.
  useEffect(() => {
    view.current?.scrollTo({ top: 0 });
  }, [location.pathname]);
  // Links to a part of a page (/account#decisions): wait for it to load, then go there.
  useEffect(() => {
    const id = decodeURIComponent(location.hash.slice(1));
    if (!id) return;
    let tries = 0;
    const timer = window.setInterval(() => {
      const el = document.getElementById(id);
      if (el || ++tries > 30) {
        window.clearInterval(timer);
        el?.scrollIntoView({ block: "start" });
      }
    }, 100);
    return () => window.clearInterval(timer);
  }, [location.pathname, location.hash]);

  return (
    <div className="window">
      <a className="skip" href="#main">
        Skip to content
      </a>
      {/* In the desktop app the window has no system title bar: drag it by the bar's empty space. */}
      <nav className={`nav ${isDesktop ? "titlebar" : ""}`} aria-label="Main" data-tauri-drag-region>
        <Link className="logo" to="/" aria-label="Slop home">
          <img src="/logo.svg" alt="" width={28} height={28} />
          <span>
            SL<b>O</b>P
          </span>
        </Link>
        {/* The website offers the desktop app; the app itself doesn't need to. */}
        {!isDesktop && (
          <Link className="btn small download-btn" to="/download">
            ⬇ Download
          </Link>
        )}
        <div className="tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={() => (t.match(location.pathname) ? "on" : "")}>
              {t.label.toUpperCase()}
              {t.to === "/chat" && chatUnread > 0 && (
                <span className="unread-badge tab-badge" aria-label={`${chatUnread} unread`}>
                  {chatUnread > 99 ? "99+" : chatUnread}
                </span>
              )}
            </NavLink>
          ))}
          <AdminTab />
        </div>
        <div className="grow" data-tauri-drag-region />
        <SearchBox />
        <Bell />
        <MeMenu />
        <WindowControls />
      </nav>

      <UpdateNotice />
      <SimulationBanner />
      <AccountNotices />
      <main className="view" id="main" ref={view} tabIndex={-1}>
        <Outlet />
      </main>

      <nav className="bottom-tabs" aria-label="Main">
        {TABS.map((t) => (
          <NavLink key={t.to} to={t.to} end={t.end} className={() => (t.match(location.pathname) ? "on" : "")}>
            <span aria-hidden="true">{t.icon}</span>
            {t.label}
          </NavLink>
        ))}
        <NavLink to="/account" className={({ isActive }) => (isActive ? "on" : "")}>
          <span aria-hidden="true">👤</span>Account
        </NavLink>
      </nav>
    </div>
  );
}

/** Shown only to Slop staff. */
function AdminTab() {
  const { me } = useSession();
  if (!me?.is_admin) return null;
  return (
    <NavLink to="/admin" className={({ isActive }) => (isActive ? "on" : "")}>
      ADMIN
    </NavLink>
  );
}

function SearchBox() {
  const navigate = useNavigate();
  const location = useLocation();
  const current = location.pathname === "/search" ? (new URLSearchParams(location.search).get("q") ?? "") : "";
  const [value, setValue] = useState(current);
  useEffect(() => setValue(current), [current]);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    navigate(`/search?q=${encodeURIComponent(value.trim())}`);
  };
  return (
    <form className="search" role="search" onSubmit={submit}>
      <input
        type="search"
        placeholder="Search apps, games, tags…"
        aria-label="Search the store"
        value={value}
        maxLength={100}
        onChange={(e) => setValue(e.target.value)}
      />
    </form>
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
      <span className="avatar" aria-hidden="true">
        {me.display_name.charAt(0).toUpperCase()}
      </span>
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
      className={`bell ${location.pathname === "/notifications" ? "on" : ""}`}
      aria-label={n > 0 ? `Notifications, ${n} unread` : "Notifications"}
      title="Notifications"
    >
      <span aria-hidden="true">🔔</span>
      {n > 0 && (
        <span className="bell-count" aria-hidden="true">
          {n > 99 ? "99+" : n}
        </span>
      )}
    </Link>
  );
}

/** Left-hand navigation on store pages. */
export function Sidebar({ active }: { active?: string }) {
  const categories = useLoad(() => api.categories(), []);
  return (
    <aside className="sidebar" aria-label="Browse">
      <h4>Discover</h4>
      <Link to="/upcoming">🔭 Coming soon &amp; beta</Link>
      <Link to="/search">✨ Everything</Link>
      <Link to="/made-with-people">✋ Made with people</Link>
      <Link to="/community">🍴 Community builds</Link>
      <Link to="/teams">👥 Teams</Link>
      <h4>Browse</h4>
      <Link to="/browse">🧭 All categories</Link>
      {categories.data?.map((c: Category) => (
        <Link key={c.slug} to={`/browse/${c.slug}`} className={active === c.slug ? "on" : ""}>
          {c.name}
        </Link>
      ))}
      <h4>Makers</h4>
      <Link to="/publish">⬆️ Publish your app</Link>
      <Link to="/publish/guide">📖 How to publish</Link>
      <Link to="/people">🎨 Hire artists &amp; composers</Link>
      <h4>Slop</h4>
      <Link to="/legal">📜 Terms &amp; policies</Link>
      <Link to="/legal/content">🛡️ Content Policy &amp; reports</Link>
    </aside>
  );
}

/**
 * Things the signed-in user should know, on every page but never in the way: a suspension, new
 * moderation decisions, and agreements or policies that changed (crates/slop-api/src/legal.rs).
 * Announcements of later changes can be dismissed; ones in effect stay until accepted.
 */
function AccountNotices() {
  const { me } = useSession();
  const location = useLocation();
  const notices = useLoad(() => (me ? api.legal.notices() : Promise.resolve([])), [me?.id, location.pathname === "/account"]);
  const [dismissed, setDismissed] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("slop.dismissed-notices") ?? "[]") as string[];
    } catch {
      return [];
    }
  });
  if (!me) return null;
  const dismiss = (key: string) => {
    const next = [...dismissed, key];
    setDismissed(next);
    try {
      localStorage.setItem("slop.dismissed-notices", JSON.stringify(next));
    } catch {
      // Private windows: dismissed for this page view only.
    }
  };
  const legal = (notices.data ?? []).filter((n) => !dismissed.includes(`${n.kind}:${n.version}`) || (n.in_effect && n.action === "accept"));
  if (!me.suspended && me.new_decisions === 0 && legal.length === 0) return null;
  return (
    <div className="account-notices" role="status">
      {me.suspended && (
        <div className="account-notice bad-notice">
          <b>Your account is suspended.</b> You can read Slop, download your data and appeal.{" "}
          <Link to="/account#decisions">See why, and appeal</Link>
        </div>
      )}
      {!me.suspended && me.new_decisions > 0 && (
        <div className="account-notice">
          Slop made {me.new_decisions === 1 ? "a decision" : `${me.new_decisions} decisions`} about your content.{" "}
          <Link to="/account#decisions">See what, why, and how to appeal</Link>
        </div>
      )}
      {legal.map((n) => (
        <div className="account-notice" key={`${n.kind}:${n.version}`}>
          {n.first
            ? `Slop has published its ${n.title}.`
            : n.in_effect
              ? `The ${n.title} changed on ${formatDate(n.effective_at)}.`
              : `The ${n.title} changes on ${formatDate(n.effective_at)}.`}{" "}
          {!n.first && <span className="muted">{n.summary}</span>}{" "}
          <Link to={`/legal/${n.kind}${n.in_effect ? "" : `?version=${n.version}`}`}>
            {n.action === "accept" ? "Review and accept" : "Read it"}
          </Link>
          {!(n.in_effect && n.action === "accept") && (
            <button type="button" className="link-btn small muted" onClick={() => dismiss(`${n.kind}:${n.version}`)}>
              Dismiss
            </button>
          )}
        </div>
      ))}
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
