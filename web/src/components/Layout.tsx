import { useEffect, useRef, useState, type FormEvent } from "react";
import { Link, NavLink, Outlet, useLocation, useNavigate } from "react-router";

import { api, type Category } from "../api";
import { useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { UpdateNotice } from "./UpdateNotice";
import { WindowControls } from "./WindowControls";
import { useSession } from "../session";

const TABS = [
  {
    to: "/",
    label: "Store",
    icon: "🛒",
    end: true,
    match: (p: string) => !["/publish", "/account", "/library", "/admin", "/people", "/jobs"].some((prefix) => p.startsWith(prefix)),
  },
  {
    to: "/people",
    label: "People",
    icon: "🎨",
    end: false,
    match: (p: string) => p.startsWith("/people") || p.startsWith("/jobs"),
  },
  { to: "/library", label: "Library", icon: "📚", end: false, match: (p: string) => p.startsWith("/library") },
  { to: "/publish", label: "Publish", icon: "⬆️", end: false, match: (p: string) => p.startsWith("/publish") },
];

export function Layout() {
  const location = useLocation();
  const view = useRef<HTMLElement>(null);

  // New page, back to the top. The page scrolls inside <main>, not the window.
  useEffect(() => {
    view.current?.scrollTo({ top: 0 });
  }, [location.pathname]);

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
            </NavLink>
          ))}
          <AdminTab />
        </div>
        <div className="grow" data-tauri-drag-region />
        <SearchBox />
        <MeMenu />
        <WindowControls />
      </nav>

      <UpdateNotice />
      <SimulationBanner />
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

/** Left-hand navigation on store pages. */
export function Sidebar({ active }: { active?: string }) {
  const categories = useLoad(() => api.categories(), []);
  return (
    <aside className="sidebar" aria-label="Browse">
      <h4>Discover</h4>
      <Link to="/upcoming">🔭 Coming soon &amp; beta</Link>
      <Link to="/search">✨ Everything</Link>
      <Link to="/made-with-people">✋ Made with people</Link>
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
    </aside>
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
