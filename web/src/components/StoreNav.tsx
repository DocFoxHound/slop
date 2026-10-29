// SPDX-License-Identifier: AGPL-3.0-or-later
// The store's own navigation, along the top of store pages (like Steam's store bar): Browse,
// Discover, Games, Kids & Family, Utilities and More, with the store search. The front page under it is all
// apps; categories and the rest live here, in More, and on the list pages' sidebars: Browse's tree of
// every category, a category's own (its sub-genres), and the generic one on the other list pages.

import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Link, useLocation, useNavigate } from "react-router";

import { api, type Category } from "../api";
import { useLoad } from "../hooks";
import type { StoreIconName } from "../storeIcons";
import { CategoryIcon, StoreIcon, TagIcon } from "./StoreIcons";

/** Categories with their own place on the bar; the rest are under More. */
const ON_BAR = ["games", "family", "utilities"];

type Section = "browse" | "discover" | "games" | "family" | "utilities" | "more" | null;

/** Which part of the bar the page is in. */
export function storeSection(pathname: string, search: string, categories: Category[] | undefined): Section {
  if (pathname === "/browse") return "browse";
  if (pathname === "/discover" || pathname === "/recommended") return "discover";
  let category: string | undefined;
  // A category's page, or its full list (/browse/<slug>/all).
  if (pathname.startsWith("/browse/")) category = decodeURIComponent(pathname.slice("/browse/".length).split("/")[0]);
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
  const item = (key: Exclude<Section, null>, to: string, label: string, icon: StoreIconName) => (
    <Link to={to} className={section === key ? "on" : ""} aria-current={section === key ? "page" : undefined}>
      <StoreIcon name={icon} />
      {label}
    </Link>
  );
  return (
    <nav className="store-nav" aria-label="Store">
      <div className="store-nav-links">
        {item("browse", "/browse", "Browse", "browse")}
        {item("discover", "/discover", "Discover", "recommended")}
        {item("games", "/browse/games", "Games", "games")}
        {item("family", "/browse/family", "Kids & Family", "family")}
        {item("utilities", "/browse/utilities", "Utilities", "utilities")}
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
              <CategoryIcon slug={c.slug} />
              {c.name}
            </Link>
          )),
        )}
        {group("Discover", <DiscoverLinks />)}
        {group("Makers", <MakerLinks />)}
      </div>
    </div>
  );
}

/** The rest of the store, in More and on the sidebar. */
const DISCOVER: Array<[string, StoreIconName, string]> = [
  ["/upcoming", "upcoming", "Coming soon & beta"],
  ["/search", "everything", "Everything"],
  ["/made-with-people", "made", "Made with people"],
  ["/community", "community", "Community builds"],
  ["/teams", "teams", "Teams"],
];

/** For makers, in More and on the sidebar. */
const MAKERS: Array<[string, StoreIconName, string]> = [
  ["/publish", "publish", "Publish your app"],
  ["/publish/guide", "guide", "How to publish"],
  ["/people", "hire", "Hire artists & composers"],
  ["/bounties", "bounties", "Fix bounties"],
];

function links(list: Array<[string, StoreIconName, string]>) {
  return list.map(([to, icon, label]) => (
    <Link key={to} to={to}>
      <StoreIcon name={icon} />
      {label}
    </Link>
  ));
}

function DiscoverLinks() {
  return <>{links(DISCOVER)}</>;
}

function MakerLinks() {
  return <>{links(MAKERS)}</>;
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
      <DiscoverLinks />
      <h4>Browse</h4>
      <Link to="/browse">
        <StoreIcon name="browse" />
        All categories
      </Link>
      {categories.data?.map((c: Category) => (
        <Link key={c.slug} to={`/browse/${c.slug}`} className={active === c.slug ? "on" : ""}>
          <CategoryIcon slug={c.slug} />
          {c.name}
        </Link>
      ))}
      <h4>Makers</h4>
      <MakerLinks />
    </aside>
  );
}

/**
 * Browse's sidebar: every category, each opening to its sub-genres (`open` starts open). The
 * category's name goes to its page; the arrow beside it shows its sub-genres here.
 */
export function CategoryTree({ categories, open }: { categories: Category[] | undefined; open?: string }) {
  const [shown, setShown] = useState<ReadonlySet<string>>(() => new Set(open ? [open] : []));
  const id = useId();
  const toggle = (slug: string) =>
    setShown((was) => {
      const next = new Set(was);
      if (next.has(slug)) next.delete(slug);
      else next.add(slug);
      return next;
    });
  return (
    <nav className="sidebar browse-tree" aria-label="Categories">
      <h4>Categories</h4>
      <ul>
        {categories?.map((c) => {
          const expanded = shown.has(c.slug);
          const list = `${id}-${c.slug}`;
          return (
            <li key={c.slug}>
              <div className="tree-row">
                <Link to={`/browse/${c.slug}`}>
                  <CategoryIcon slug={c.slug} />
                  {c.name}
                </Link>
                <button
                  type="button"
                  className="tree-toggle"
                  aria-expanded={expanded}
                  aria-controls={list}
                  aria-label={`${c.name} sub-genres`}
                  onClick={() => toggle(c.slug)}
                >
                  <span aria-hidden="true">▸</span>
                </button>
              </div>
              <ul id={list} hidden={!expanded}>
                {c.tags.map((t) => (
                  <li key={t.slug}>
                    <Link to={`/tag/${t.slug}`}>
                      <TagIcon slug={t.slug} category={c.slug} />
                      {t.name}
                    </Link>
                  </li>
                ))}
              </ul>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}

/**
 * A category's own sidebar, on its page, its full list and its sub-genres' pages: the way back to
 * every category, then the category (its page, and everything in it) and each sub-genre. `on` is the page: "" for the category's,
 * "all" for its full list, or a sub-genre's slug. On a phone it's a row that scrolls sideways.
 */
export function CategorySidebar({ category, on }: { category: Category | undefined; on: string }) {
  // Holds the column while the categories load.
  if (!category) return <div className="sidebar" />;
  const item = (key: string, to: string, children: ReactNode, className = "") => (
    <Link to={to} className={`${className} ${on === key ? "on" : ""}`.trim()} aria-current={on === key ? "page" : undefined}>
      {children}
    </Link>
  );
  return (
    <nav className="sidebar category-side" aria-label={`${category.name} sub-genres`}>
      <Link to="/browse" className="side-back">
        <StoreIcon name="browse" />
        All categories
      </Link>
      {item(
        "",
        `/browse/${category.slug}`,
        <>
          <CategoryIcon slug={category.slug} />
          {category.name}
        </>,
        "side-title",
      )}
      {item(
        "all",
        `/browse/${category.slug}/all`,
        <>
          <StoreIcon name="everything" />
          Everything in {category.name}
        </>,
      )}
      <h4>Sub-genres</h4>
      {category.tags.map((t) =>
        item(
          t.slug,
          `/tag/${t.slug}`,
          <>
            <TagIcon slug={t.slug} category={category.slug} />
            {t.name}
          </>,
        ),
      )}
    </nav>
  );
}

/** SLAPorSLOP itself: the rules, checking a file, the code, and how to reach SLAPorSLOP. */
export function StoreFooter() {
  return (
    <footer className="store-footer">
      <nav aria-label="About SLAPorSLOP">
        <Link to="/legal">
          <StoreIcon name="terms" />
          Terms &amp; policies
        </Link>
        <Link to="/legal/content">
          <StoreIcon name="policy" />
          Content Policy &amp; reports
        </Link>
        <Link to="/verify">
          <StoreIcon name="verify" />
          Check a file
        </Link>
        <Link to="/source">
          <StoreIcon name="source" />
          Source code
        </Link>
        <a href="https://github.com/DocFoxHound/slop/blob/main/docs/CODE-SIGNING.md">
          <StoreIcon name="signing" />
          Code signing policy
        </a>
        <Link to="/contact">
          <StoreIcon name="contact" />
          Contact
        </Link>
      </nav>
    </footer>
  );
}

/**
 * A store page: the store bar on top, SLAPorSLOP's links at the bottom, and, on list pages, a
 * sidebar for browsing: `true` for the generic one, or a sidebar of the page's own (Browse's tree,
 * a category's). The front page has no sidebar: it's all apps.
 */
export function StoreFrame({ children, sidebar = false, active }: { children: ReactNode; sidebar?: boolean | ReactNode; active?: string }) {
  return (
    <div className="page">
      <StoreNav />
      {sidebar ? (
        <div className="store">
          {sidebar === true ? <Sidebar active={active} /> : sidebar}
          <div>{children}</div>
        </div>
      ) : (
        children
      )}
      <StoreFooter />
    </div>
  );
}
