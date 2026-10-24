// SPDX-License-Identifier: AGPL-3.0-or-later
// The store's own navigation, along the top of store pages (like Steam's store bar): Browse,
// Recommended, Games, Utilities and More, with the store search. The front page under it is all
// apps; categories and the rest live here, in More, and on the list pages' sidebar.

import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Link, useLocation, useNavigate } from "react-router";

import { api, type Category } from "../api";
import { useLoad } from "../hooks";

/** Categories with their own place on the bar; the rest are under More. */
const ON_BAR = ["games", "utilities"];

type Section = "browse" | "recommended" | "games" | "utilities" | "more" | null;

/** Which part of the bar the page is in. */
export function storeSection(pathname: string, search: string, categories: Category[] | undefined): Section {
  if (pathname === "/browse") return "browse";
  if (pathname === "/recommended") return "recommended";
  let category: string | undefined;
  if (pathname.startsWith("/browse/")) category = decodeURIComponent(pathname.slice("/browse/".length));
  if (pathname.startsWith("/tag/")) {
    const tag = decodeURIComponent(pathname.slice("/tag/".length));
    category = categories?.find((c) => c.tags.some((t) => t.slug === tag))?.slug;
    // A tag of a category not known yet: nothing until the categories load.
    if (!category) return null;
  }
  if (category) return ON_BAR.includes(category) ? (category as Section) : "more";
  // "Everything" is search without words; a search with words is just a search.
  if (pathname === "/search") return new URLSearchParams(search).get("q") ? null : "more";
  if (["/upcoming", "/made-with-people", "/community", "/teams"].includes(pathname)) return "more";
  return null;
}

export function StoreNav() {
  const location = useLocation();
  const categories = useLoad(() => api.categories(), []);
  const section = storeSection(location.pathname, location.search, categories.data);
  const item = (key: Exclude<Section, null>, to: string, label: string) => (
    <Link to={to} className={section === key ? "on" : ""} aria-current={section === key ? "page" : undefined}>
      {label}
    </Link>
  );
  return (
    <nav className="store-nav" aria-label="Store">
      <div className="store-nav-links">
        {item("browse", "/browse", "Browse")}
        {item("recommended", "/recommended", "Recommended")}
        {item("games", "/browse/games", "Games")}
        {item("utilities", "/browse/utilities", "Utilities")}
      </div>
      <MoreMenu on={section === "more"} categories={categories.data?.filter((c) => !ON_BAR.includes(c.slug))} />
      <StoreSearch />
    </nav>
  );
}

/** The rest of the store, in a menu that opens below its button. */
function MoreMenu({ on, categories }: { on: boolean; categories: Category[] | undefined }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const location = useLocation();
  // Following a link closes it.
  useEffect(() => setOpen(false), [location.pathname, location.search]);
  // A click or focus anywhere else closes it.
  useEffect(() => {
    if (!open) return;
    const away = (e: Event) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    document.addEventListener("focusin", away);
    return () => {
      document.removeEventListener("mousedown", away);
      document.removeEventListener("focusin", away);
    };
  }, [open]);
  const group = (title: string, links: ReactNode) => (
    <div className="more-group" role="group" aria-label={title}>
      <h4 aria-hidden="true">{title}</h4>
      {links}
    </div>
  );
  return (
    <div
      className="store-more"
      ref={root}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.preventDefault();
          setOpen(false);
          button.current?.focus();
        }
      }}
    >
      <button
        ref={button}
        type="button"
        className={`store-more-btn ${on ? "on" : ""}`}
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
      >
        More <span aria-hidden="true">▾</span>
      </button>
      <div className="store-more-menu" id={id} hidden={!open}>
        {group(
          "Categories",
          categories?.map((c) => (
            <Link key={c.slug} to={`/browse/${c.slug}`}>
              {c.name}
            </Link>
          )),
        )}
        {group(
          "Discover",
          <>
            <Link to="/upcoming">🔭 Coming soon &amp; beta</Link>
            <Link to="/search">✨ Everything</Link>
            <Link to="/made-with-people">✋ Made with people</Link>
            <Link to="/community">🍴 Community builds</Link>
            <Link to="/teams">👥 Teams</Link>
          </>,
        )}
        {group(
          "Makers",
          <>
            <Link to="/publish">⬆️ Publish your app</Link>
            <Link to="/publish/guide">📖 How to publish</Link>
            <Link to="/people">🎨 Hire artists &amp; composers</Link>
            <Link to="/bounties">💰 Fix bounties</Link>
          </>,
        )}
      </div>
    </div>
  );
}

/** Searches the store (/search?q=…); on the search page it shows what was searched. */
function StoreSearch() {
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
    <form className="search store-search" role="search" onSubmit={submit}>
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

/** Left-hand navigation on the store's list pages (not the front page: that's all apps). */
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
      <Link to="/bounties">💰 Fix bounties</Link>
    </aside>
  );
}

/** Slop itself: the rules, checking a file, the code, and how to reach Slop. */
export function StoreFooter() {
  return (
    <footer className="store-footer">
      <nav aria-label="About Slop">
        <Link to="/legal">📜 Terms &amp; policies</Link>
        <Link to="/legal/content">🛡️ Content Policy &amp; reports</Link>
        <Link to="/verify">🔎 Check a file</Link>
        <Link to="/source">🧾 Source code</Link>
        <Link to="/contact">✉️ Contact</Link>
      </nav>
    </footer>
  );
}

/**
 * A store page: the store bar on top, Slop's links at the bottom, and, on list pages, the
 * sidebar for browsing. The front page has no sidebar: it's all apps.
 */
export function StoreFrame({ children, sidebar = false, active }: { children: ReactNode; sidebar?: boolean; active?: string }) {
  return (
    <div className="page">
      <StoreNav />
      {sidebar ? (
        <div className="store">
          <Sidebar active={active} />
          <div>{children}</div>
        </div>
      ) : (
        children
      )}
      <StoreFooter />
    </div>
  );
}
