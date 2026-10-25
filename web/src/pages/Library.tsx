// SPDX-License-Identifier: AGPL-3.0-or-later
// The library (DESIGN §6), laid out like Steam's: a sidebar listing every app you have (installed,
// not installed, Private Slop, Interested) with a search and a filter, and the selected app on the
// right: a wide banner, one big button (Play, Install, Update…), its activity (release notes and
// your bug reports) and what friends thought of it. The address is /library/<app>. Installing
// happens in the Slop app; the website shows the same library and offers the app.

import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router";

import { api, type AppDetail, type BugReport, type Release } from "../api";
import { AppIcon, appIconProps } from "../components/AppIcon";
import { Avatar } from "../components/Avatar";
import { checkUpdates, LIBRARY_CHANGED } from "../components/AppUpdates";
import { BugReports } from "../components/BugReports";
import { downloading, DownloadsStatus, useDownloads } from "../components/Downloads";
import { GEAR_ICON, INFO_ICON, InstallControls, Popover } from "../components/InstallControls";
import { GameMods } from "../components/ModControls";
import { RatePrompt, RatingBadge } from "../components/RatingPanel";
import { Reviews } from "../components/Reviews";
import { isOn, useSandboxStatus } from "../components/Sandbox";
import { ErrorState, ExternalLink, Loading, StageBadge } from "../components/ui";
import { artFor, formatDate } from "../format";
import { useChosen, useDocumentTitle, useLoad } from "../hooks";
import { renderMarkdown } from "../markdown";
import { desktop, detectOs, isDesktop, type InstalledApp, type SaveCopy } from "../platform";
import { useSession } from "../session";
import {
  buildEntries,
  countLabel,
  filtersFor,
  matchesFilter,
  sections,
  statusOf,
  type Entry,
  type Filter,
  type Group,
  type Live,
} from "./libraryModel";

/** Below this width the sidebar and the app are separate screens (styles.css, "library"). */
const PHONE = "(max-width: 760px)";
const isPhone = () => typeof window !== "undefined" && Boolean(window.matchMedia?.(PHONE).matches);

export function Library() {
  const { me } = useSession();
  const { slug } = useParams();
  const location = useLocation();
  const navigate = useNavigate();
  const library = useLoad(() => (me ? api.library() : Promise.resolve([])), [me?.id]);
  const installed = useLoad(() => (isDesktop ? checkUpdates() : Promise.resolve([])), []);
  const reloadInstalled = installed.reload;
  useEffect(() => {
    window.addEventListener(LIBRARY_CHANGED, reloadInstalled);
    return () => window.removeEventListener(LIBRARY_CHANGED, reloadInstalled);
  }, [reloadInstalled]);
  const interested = useLoad(() => (me ? api.interested.mine() : Promise.resolve([])), [me?.id]);
  const privateSlop = useLoad(
    () => (me ? api.private.mine().catch(() => ({ mine: [], shared: [] })) : Promise.resolve({ mine: [], shared: [] })),
    [me?.id],
  );
  const autoUpdate = useLoad(() => (isDesktop ? desktop.autoUpdateSetting() : Promise.resolve(false)), []);
  const running = useRunning();
  const downloads = useDownloads();
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<Group[]>([]);
  const list = useRef<HTMLDivElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const lastSlug = useRef<string | undefined>(undefined);

  const entries = useMemo(
    () =>
      buildEntries({
        library: library.data ?? [],
        installed: installed.data ?? [],
        interested: interested.data ?? [],
        privateMine: privateSlop.data?.mine ?? [],
        privateShared: privateSlop.data?.shared ?? [],
      }),
    [library.data, installed.data, interested.data, privateSlop.data],
  );
  const live: Live = { downloading: downloading(downloads), running, autoUpdate: autoUpdate.data ?? false };
  // On a wide screen the library opens on the first app; on a phone, on the list.
  const selected = slug ? entries.find((e) => e.slug === slug) : entries[0];
  useDocumentTitle(slug && selected ? `${selected.title} · Library` : "Library");

  // Links to the old library's parts (/library#<app>) open that app.
  useEffect(() => {
    const id = decodeURIComponent(location.hash.slice(1));
    if (!slug && id && entries.some((e) => e.slug === id)) navigate(`/library/${encodeURIComponent(id)}`, { replace: true });
  }, [slug, location.hash, entries, navigate]);

  // On a phone, opening an app moves to its page; going back returns to its row.
  useEffect(() => {
    if (isPhone()) {
      if (slug) heading.current?.focus();
      else if (lastSlug.current)
        list.current?.querySelector<HTMLElement>(`[data-slug="${CSS.escape(lastSlug.current)}"]`)?.focus();
    }
    lastSlug.current = slug;
  }, [slug]);

  const loading =
    me === undefined ||
    (library.loading && !library.data) ||
    (installed.loading && !installed.data) ||
    (interested.loading && !interested.data && !interested.error) ||
    (privateSlop.loading && !privateSlop.data);
  if (loading) return <Loading />;

  const signInNote = !me && (
    <p className="notice small lib-signin">
      <Link to="/signin?return_to=/library">Sign in</Link> to keep a library that follows you between devices.
      {isDesktop && " Apps installed on this computer are shown below either way."}
    </p>
  );

  if (entries.length === 0) {
    return (
      <div className="page">
        <h1 className="page-title">Library</h1>
        {signInNote}
        {library.error && <ErrorState error={library.error} onRetry={library.reload} />}
        <div className="empty-state lib-empty">
          <p>
            <b>Nothing here yet.</b> Apps you add from the store show up here,
            {isDesktop ? " ready to install and play." : " ready to install from the Slop app."}
          </p>
          <Link className="btn slime" to="/">
            Find something in the store
          </Link>
        </div>
      </div>
    );
  }

  const shown = sections(entries, filter, query, isDesktop);
  const filters = filtersFor(isDesktop).filter(
    (f) => f.id === "all" || f.id === filter || entries.some((e) => matchesFilter(e, f.id)),
  );

  // Arrow keys move through the list, like Steam's; the app follows on a wide screen.
  const onListKey = (e: KeyboardEvent) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const rows = Array.from(list.current?.querySelectorAll<HTMLAnchorElement>("a.lib-row") ?? []);
    if (rows.length === 0) return;
    e.preventDefault();
    const at = rows.indexOf(document.activeElement as HTMLAnchorElement);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? rows.length - 1
          : e.key === "ArrowDown"
            ? Math.min(rows.length - 1, at + 1)
            : Math.max(0, at - 1);
    rows[next].focus();
    const to = rows[next].dataset.slug;
    if (to && !isPhone()) navigate(`/library/${encodeURIComponent(to)}`, { replace: true });
  };
  const firstShown = shown[0]?.entries[0];
  const selectedShown = shown.some((s) => s.entries.includes(selected!));

  return (
    <div className={`lib${slug ? " has-slug" : ""}`}>
      <nav className="lib-side" aria-label="Your apps">
        <div className="lib-side-head">
          <h1>Library</h1>
          {isDesktop && <LibrarySettings onChanged={autoUpdate.reload} />}
        </div>
        {signInNote}
        {library.error && <ErrorState error={library.error} onRetry={library.reload} />}
        <div className="lib-filters">
          <select aria-label="Show" value={filter} onChange={(e) => setFilter(e.target.value as Filter)}>
            {filters.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
          <input
            type="search"
            className="lib-search"
            placeholder="Search"
            aria-label="Search your library"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <div className="lib-list" ref={list} onKeyDown={onListKey}>
          {shown.length === 0 && <p className="muted small lib-none">No apps match.</p>}
          {shown.map((s) => {
            const closed = collapsed.includes(s.group) && !query;
            const headId = `lib-group-${s.group}`;
            return (
              <div key={s.group} role="group" aria-labelledby={headId}>
                <h2 className="lib-group" id={headId}>
                  <button
                    type="button"
                    aria-expanded={!closed}
                    onClick={() =>
                      setCollapsed((c) => (c.includes(s.group) ? c.filter((g) => g !== s.group) : [...c, s.group]))
                    }
                  >
                    <span aria-hidden="true">{closed ? "+" : "–"}</span> {s.label}{" "}
                    <span className="lib-count">{countLabel(s)}</span>
                  </button>
                </h2>
                {!closed && (
                  <ul className="lib-rows">
                    {s.entries.map((e) => {
                      const status = statusOf(e, live);
                      const on = e.slug === selected?.slug;
                      // One row is reachable with Tab; the arrow keys move between them.
                      const tabbable = on || (!selectedShown && e === firstShown);
                      return (
                        <li key={e.slug}>
                          <Link
                            to={`/library/${encodeURIComponent(e.slug)}`}
                            className={`lib-row${on ? " on" : ""}${e.group === "installed" ? "" : " dim"}${e.buildOf ? " nested" : ""}`}
                            aria-current={on ? "page" : undefined}
                            tabIndex={tabbable ? 0 : -1}
                            data-slug={e.slug}
                          >
                            <AppIcon {...appIconProps(e.app ?? { title: e.title, slug: e.slug })} size={20} className="lib-app-icon" />
                            <span className="lib-row-name">{e.title}</span>
                            {status && <span className="lib-row-status"> – {status}</span>}
                          </Link>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>
            );
          })}
        </div>
        {isDesktop && (
          <DownloadsStatus
            apps={installed.data}
            titles={Object.fromEntries(entries.map((e) => [e.slug, e.title]))}
            onChanged={reloadInstalled}
          />
        )}
      </nav>

      {selected ? (
        <Details
          key={selected.slug}
          entry={selected}
          live={live}
          signedIn={Boolean(me)}
          headingRef={heading}
          onInterestRemoved={() => {
            interested.reload();
            navigate("/library", { replace: true });
          }}
        />
      ) : (
        <section className="lib-main" aria-labelledby="lib-title">
          <Link className="btn small ghost lib-back" to="/library">
            ← Library
          </Link>
          <div className="lib-body">
            <h2 id="lib-title" ref={heading} tabIndex={-1}>
              That isn't in your library
            </h2>
            <p className="muted">
              <Link to={`/app/${encodeURIComponent(slug ?? "")}`}>Open its store page</Link> to add it.
            </p>
          </div>
        </section>
      )}
    </div>
  );
}

/** Apps Slop started that are still running, checked every few seconds (the Slop app). */
function useRunning(): string[] {
  const [running, setRunning] = useState<string[]>([]);
  useEffect(() => {
    if (!isDesktop) return;
    let alive = true;
    const tick = () =>
      void desktop.running().then(
        (r) => {
          if (alive) setRunning(r ?? []);
        },
        () => undefined,
      );
    tick();
    const every = window.setInterval(tick, 5000);
    return () => {
      alive = false;
      window.clearInterval(every);
    };
  }, []);
  return running;
}

type Tab = "activity" | "community" | "saves" | "bugs" | "reviews";

/** The selected app: banner, action bar, a rating prompt after playing, tabs and their content. */
function Details({
  entry,
  live,
  signedIn,
  headingRef,
  onInterestRemoved,
}: {
  entry: Entry;
  live: Live;
  signedIn: boolean;
  headingRef: RefObject<HTMLHeadingElement | null>;
  onInterestRemoved: () => void;
}) {
  const detail = useLoad(() => api.app(entry.slug).catch(() => null), [entry.slug]);
  const upcoming = entry.app?.stage === "coming_soon";
  const releases = useLoad(
    () => (upcoming ? Promise.resolve([] as Release[]) : api.releases(entry.slug).catch(() => [] as Release[])),
    [entry.slug],
  );
  const bugs = useLoad(
    () => (signedIn && !upcoming ? api.bugReports.list(entry.slug, "all").catch(() => null) : Promise.resolve(null)),
    [entry.slug, signedIn],
  );
  const sandbox = useLoad(() => (isDesktop ? desktop.sandboxStatus().catch(() => null) : Promise.resolve(null)), []);
  const [tab, setTab] = useState<Tab>("activity");
  const tabsId = useId();
  const d = detail.data ?? undefined;
  const installedHere = isDesktop && Boolean(entry.installed);
  const isPrivate = Boolean(entry.owner) || Boolean(d?.private);
  const hasCommunity =
    (d?.community.length ?? 0) > 0 ||
    (installedHere && ((entry.update?.builds.length ?? 0) > 1 || (entry.update?.mods?.length ?? 0) > 0));

  const tabs: Array<{ id: Tab; label: string }> = [
    { id: "activity", label: "Activity" },
    ...(hasCommunity ? [{ id: "community" as const, label: "Community builds & mods" }] : []),
    ...(installedHere && entry.installed?.kind !== "installer" ? [{ id: "saves" as const, label: "Saves" }] : []),
    ...(signedIn && !upcoming && d?.bug_reports !== false ? [{ id: "bugs" as const, label: "Bug reports" }] : []),
    ...(!isPrivate && !upcoming ? [{ id: "reviews" as const, label: "Reviews" }] : []),
  ];
  const current = tabs.some((t) => t.id === tab) ? tab : "activity";
  const onTabKey = (e: KeyboardEvent) => {
    const at = tabs.findIndex((t) => t.id === current);
    const keys: Record<string, number> = {
      ArrowRight: (at + 1) % tabs.length,
      ArrowLeft: (at - 1 + tabs.length) % tabs.length,
      Home: 0,
      End: tabs.length - 1,
    };
    const next = keys[e.key];
    if (next === undefined) return;
    e.preventDefault();
    setTab(tabs[next].id);
    document.getElementById(`${tabsId}-${tabs[next].id}`)?.focus();
  };

  const image = d?.gallery.find((g) => g.url)?.url ?? entry.app?.thumbnail_url ?? d?.thumbnail_url ?? null;
  const { from, to } = artFor(entry.slug);
  const maker = d?.maker.display_name ?? entry.installed?.maker;
  const latest = releases.data?.find((r) => r.status === "approved");
  const waiting = entry.interested && !entry.installed && !entry.inLibrary && entry.app?.stage !== "released";
  const info = <AppInfo entry={entry} detail={d} latest={latest} />;

  return (
    <section className="lib-main" aria-labelledby="lib-title">
      <Link className="btn small ghost lib-back" to="/library">
        ← Library
      </Link>
      <div
        className={`lib-hero${image ? " has-image" : ""}`}
        style={image ? undefined : { background: `linear-gradient(135deg, ${from}, ${to})` }}
      >
        {image && <img className="lib-hero-img" src={image} alt="" />}
        <div className="lib-hero-text">
          <AppIcon
            {...appIconProps({
              title: entry.title,
              slug: entry.slug,
              icon_url: entry.app?.icon_url ?? d?.icon_url,
              thumbnail_url: entry.app?.thumbnail_url ?? d?.thumbnail_url,
            })}
            size={48}
            className="lib-app-icon"
          />
          <div>
            <h2 id="lib-title" ref={headingRef} tabIndex={-1}>
              {entry.title}
            </h2>
            <div className="lib-hero-sub">
              {entry.app && <StageBadge app={entry.app} />}
              {entry.buildOf && <span className="badge neutral">COMMUNITY</span>}
              {isPrivate && <span className="badge neutral">PRIVATE</span>}
              {installedHere && <span className="badge official">INSTALLED</span>}
              {installedHere && sandbox.data?.available && isOn(sandbox.data, entry.slug) && (
                <span className="badge neutral">SANDBOX</span>
              )}
              {!isPrivate && !upcoming && <RatingBadge slug={entry.slug} signedIn={signedIn} />}
              <span>
                {entry.buildOf ? `Community build of ${entry.buildOf} · ` : ""}
                {entry.owner ? `by ${entry.owner}` : maker ? `by ${maker}` : ""}
              </span>
            </div>
          </div>
        </div>
      </div>

      <div className="lib-body">
        {entry.interested && !entry.inLibrary && !entry.installed && entry.app?.stage === "released" && (
          <p className="notice small">Released: add it to your library to install it.</p>
        )}
        {waiting ? (
          <InterestedBar entry={entry} info={info} onRemoved={onInterestRemoved} />
        ) : (
          <InstallControls
            key={entry.slug}
            slug={entry.slug}
            title={entry.title}
            inLibrary={entry.inLibrary}
            sandbox={
              entry.update
                ? {
                    suggested: Boolean(entry.update.sandbox_suggested),
                    offline: !entry.update.sandbox_suggested,
                    onlineMods: (entry.update.mods ?? []).filter((m) => m.online).map((m) => m.slug),
                  }
                : undefined
            }
            bar={{
              available: entry.available,
              latest: entry.update?.latest?.version,
              pending: entry.update?.pending,
              autoUpdate: live.autoUpdate,
              running: live.running.includes(entry.slug),
              extra: info,
              onShowSaves: () => setTab("saves"),
            }}
          />
        )}

        {signedIn && !isPrivate && !upcoming && <RatePrompt slug={entry.slug} title={entry.title} />}

        <div className="lib-tabbar">
          <div className="lib-tabs" role="tablist" aria-label={`${entry.title} in your library`} onKeyDown={onTabKey}>
            {tabs.map((t) => (
              <button
                key={t.id}
                type="button"
                role="tab"
                id={`${tabsId}-${t.id}`}
                aria-selected={current === t.id}
                aria-controls={`${tabsId}-panel`}
                tabIndex={current === t.id ? 0 : -1}
                className={current === t.id ? "on" : ""}
                onClick={() => setTab(t.id)}
              >
                {t.label}
              </button>
            ))}
          </div>
          <div className="lib-tablinks">
            <Link to={`/app/${encodeURIComponent(entry.slug)}`}>Store page</Link>
            {d?.repo_url && <ExternalLink href={d.repo_url}>Source ↗</ExternalLink>}
          </div>
        </div>

        <div className="lib-panel" role="tabpanel" id={`${tabsId}-panel`} aria-labelledby={`${tabsId}-${current}`}>
          {current === "activity" && (
            <div className="lib-columns">
              <Activity entry={entry} releases={releases.data ?? []} bugs={bugs.data?.reports ?? []} upcoming={upcoming} />
              <div className="lib-aside">
                <div className="lib-box">
                  <h3 className="lib-col-h">About</h3>
                  <p className="small">{entry.app?.summary ?? d?.summary ?? `${entry.title} is installed on this computer.`}</p>
                  <Link className="small" to={`/app/${encodeURIComponent(entry.slug)}`}>
                    Store page →
                  </Link>
                </div>
                {signedIn && !isPrivate && <FriendsWhoRated slug={entry.slug} />}
              </div>
            </div>
          )}
          {current === "community" && <CommunityTab entry={entry} detail={d} />}
          {current === "saves" && <Saves slug={entry.slug} />}
          {current === "bugs" && (
            <>
              {bugs.data && !bugs.data.can_report && !bugs.data.is_maker && (
                <p className="muted small">You can report bugs once you've played it from the Slop app.</p>
              )}
              <BugReports slug={entry.slug} version={entry.installed?.version} />
            </>
          )}
          {current === "reviews" && (
            <>
              <p className="muted small">
                What players wrote with their vote.{" "}
                <Link to={`/app/${encodeURIComponent(entry.slug)}#rating`}>See the ratings</Link>.
              </p>
              <Reviews slug={entry.slug} />
            </>
          )}
        </div>
      </div>
    </section>
  );
}

/** An upcoming app you're waiting for: nothing to install yet. */
function InterestedBar({ entry, info, onRemoved }: { entry: Entry; info: ReactNode; onRemoved: () => void }) {
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  return (
    <>
      <div className="lib-bar">
        <button type="button" className="btn big lib-primary" disabled>
          {entry.app?.stage === "beta" ? "In beta" : "Coming soon"}
        </button>
        <div className="lib-bar-status">
          <span className="lib-bar-label">Interested</span>
          <span className="muted small">We tell you when it reaches beta or is released.</span>
        </div>
        <div className="lib-bar-icons">
          <button
            type="button"
            className="btn ghost"
            disabled={busy}
            onClick={() => {
              setError(undefined);
              setBusy(true);
              void api.interested.remove(entry.slug).then(onRemoved, (err: unknown) => {
                setBusy(false);
                setError(err instanceof Error ? err.message : String(err));
              });
            }}
          >
            Not interested
          </button>
          {info}
        </div>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}

/** The info button: what the app is, its versions, size, licence and source. */
function AppInfo({ entry, detail, latest }: { entry: Entry; detail?: AppDetail; latest?: Release }) {
  const platform = useLoad(() => (isDesktop ? desktop.platformInfo().catch(() => null) : Promise.resolve(null)), []);
  const os = platform.data?.platform ?? detectOs();
  const file = latest?.files.find((f) => f.platform === os) ?? latest?.files.find((f) => f.platform === "any");
  const maker = detail?.maker.display_name ?? entry.installed?.maker;
  const newest = entry.update?.latest?.version ?? latest?.version;
  return (
    <Popover label={`About ${entry.title}`} icon={INFO_ICON}>
      <dl className="kv lib-info">
        <dt>What it does</dt>
        <dd>{entry.app?.summary ?? detail?.summary ?? "—"}</dd>
        {maker && (
          <>
            <dt>Made by</dt>
            <dd>{maker}</dd>
          </>
        )}
        {isDesktop && entry.installed && (
          <>
            <dt>Installed</dt>
            <dd>Version {entry.installed.version}</dd>
          </>
        )}
        {newest && (
          <>
            <dt>Latest</dt>
            <dd>Version {newest}</dd>
          </>
        )}
        {file?.size_bytes != null && (
          <>
            <dt>Download size</dt>
            <dd>{formatSize(file.size_bytes)}</dd>
          </>
        )}
        {detail?.license_spdx && (
          <>
            <dt>Licence</dt>
            <dd>
              <code>{detail.license_spdx}</code>
            </dd>
          </>
        )}
        {detail?.repo_url && (
          <>
            <dt>Source</dt>
            <dd>
              <ExternalLink href={detail.repo_url}>{detail.repo_url.replace(/^https:\/\//, "")}</ExternalLink>
            </dd>
          </>
        )}
        {entry.app?.players_now != null && entry.app.players_now > 0 && (
          <>
            <dt>Playing now</dt>
            <dd>{entry.app.players_now.toLocaleString()}</dd>
          </>
        )}
      </dl>
      <Link className="small" to={`/app/${encodeURIComponent(entry.slug)}`}>
        Store page →
      </Link>
    </Popover>
  );
}

function formatSize(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1e3))} KB`;
}

const BUG_STATUS: Record<BugReport["status"], string> = {
  open: "OPEN",
  fixed: "FIXED",
  wont_fix: "WON'T FIX",
  duplicate: "DUPLICATE",
};

/** Release notes, newest first, with the player's own bug reports and what's being checked. */
function Activity({
  entry,
  releases,
  bugs,
  upcoming,
}: {
  entry: Entry;
  releases: Release[];
  bugs: BugReport[];
  upcoming: boolean;
}) {
  type Item = { at: string; key: string; body: ReactNode };
  const items: Item[] = [
    ...releases
      .filter((r) => r.status === "approved")
      .map((r) => ({
        at: r.created_at,
        key: `r-${r.id}`,
        body: (
          <>
            <div className="lib-act-title">
              <b>Version {r.version}</b>
              {isDesktop && entry.installed?.version === r.version && <span className="badge official">INSTALLED</span>}
            </div>
            {r.notes?.trim() ? (
              // Rendered with raw HTML disabled; see markdown.ts.
              <div className="prose small" dangerouslySetInnerHTML={{ __html: renderMarkdown(r.notes) }} />
            ) : (
              <p className="muted small">No notes for this version.</p>
            )}
          </>
        ),
      })),
    ...bugs
      .filter((b) => b.mine)
      .map((b) => ({
        at: b.created_at,
        key: `b-${b.id}`,
        body: (
          <>
            <div className="lib-act-title">
              <span>
                Your bug report: <b>{b.title}</b>
              </span>
              <span className="badge neutral">{BUG_STATUS[b.status]}</span>
            </div>
            {b.maker_note && <p className="small">The maker: {b.maker_note}</p>}
          </>
        ),
      })),
  ].sort((a, b) => b.at.localeCompare(a.at));
  const days: Array<{ day: string; items: Item[] }> = [];
  for (const item of items) {
    const day = formatDate(item.at);
    const last = days[days.length - 1];
    if (last?.day === day) last.items.push(item);
    else days.push({ day, items: [item] });
  }
  const pending = entry.update?.pending;
  return (
    <div className="lib-activity">
      <h3 className="lib-col-h">Activity</h3>
      {pending && (
        <div className="lib-act-item">
          <div className="lib-act-title">
            <b>Version {pending.version}</b> <span className="badge soon">BEING CHECKED</span>
          </div>
          <p className="muted small">
            {pending.status === "fetching" ? "Slop is downloading and scanning it." : "Scanned, waiting for a person to review it."}{" "}
            It installs once it's approved.
          </p>
        </div>
      )}
      {days.length === 0 && !pending && (
        <p className="muted small">
          {upcoming ? "Nothing yet: we tell you when it reaches beta or is released." : "No release notes yet."}
        </p>
      )}
      {days.map((d) => (
        <div key={d.day} className="lib-day">
          <h4>{d.day}</h4>
          {d.items.map((i) => (
            <div key={i.key} className="lib-act-item">
              {i.body}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

interface FriendVote {
  id: string;
  name: string;
  avatar_url: string | null;
  vote: "not_slop" | "slop";
}

interface FriendVotes {
  friends: number;
  votes: Record<string, FriendVote[]>;
}

let friendCache: { user: string; at: number; data: Promise<FriendVotes> } | undefined;

/**
 * What friends voted, from their public profiles: only what they chose to show everyone. Slop
 * doesn't share friends' libraries or playtime, so this is all a friend sees. Kept a few minutes.
 */
function friendVotes(user: string): Promise<FriendVotes> {
  if (friendCache && friendCache.user === user && Date.now() - friendCache.at < 5 * 60_000) return friendCache.data;
  const data = api.friends.list().then(async (f) => {
    const profiles = await Promise.all(f.friends.slice(0, 30).map((p) => api.profile(p.id).catch(() => null)));
    const votes: Record<string, FriendVote[]> = {};
    for (const p of profiles) {
      if (!p) continue;
      for (const v of p.votes) (votes[v.slug] ??= []).push({ id: p.id, name: p.name, avatar_url: p.avatar_url, vote: v.my_vote });
    }
    return { friends: f.friends.length, votes };
  });
  data.catch(() => (friendCache = undefined));
  friendCache = { user, at: Date.now(), data };
  return data;
}

function FriendsWhoRated({ slug }: { slug: string }) {
  const { me } = useSession();
  const loaded = useLoad(() => (me ? friendVotes(me.id).catch(() => null) : Promise.resolve(null)), [me?.id]);
  if (!loaded.data) return null;
  const { friends, votes } = loaded.data;
  const list = votes[slug] ?? [];
  return (
    <div className="lib-box">
      <h3 className="lib-col-h">Friends who rated it</h3>
      {friends === 0 ? (
        <p className="muted small">
          <Link to="/friends">Add friends</Link> to see what they think of it.
        </p>
      ) : list.length === 0 ? (
        <p className="muted small">None of your friends has rated it on their profile yet.</p>
      ) : (
        <>
          <p className="small">{list.length === 1 ? "1 friend rated it" : `${list.length} friends rated it`}</p>
          <ul className="plain lib-friends">
            {list.map((f) => (
              <li key={f.id}>
                <Avatar name={f.name} url={f.avatar_url} />
                <Link to={`/u/${f.id}`}>{f.name}</Link>
                <span className="muted small">{f.vote === "not_slop" ? "👍 Not Slop" : "👎 Slop"}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

/** Community builds and mods (DESIGN §9): install another build beside this one, add mods. */
function CommunityTab({ entry, detail }: { entry: Entry; detail?: AppDetail }) {
  const builds = entry.update?.builds ?? [];
  const mods = entry.update?.mods ?? [];
  const here = isDesktop && entry.installed;
  const installedAll = useLoad(() => (here ? desktop.installed() : Promise.resolve([] as InstalledApp[])), [entry.slug]);
  const others = builds.filter((b) => b.slug !== entry.slug && !(installedAll.data ?? []).some((a) => a.slug === b.slug));
  const listed = detail?.community ?? [];
  return (
    <div className="lib-community">
      {here && entry.installed && (
        <>
          {others.length > 0 && (
            <OtherBuilds current={entry.installed} builds={builds} others={others} onInstalled={installedAll.reload} />
          )}
          <GameMods game={entry.slug} available={mods} />
        </>
      )}
      {listed.length > 0 && (
        <>
          <h3 className="lib-col-h">On its store page</h3>
          <ul className="plain lib-community-list">
            {listed.map((c) => (
              <li key={c.slug}>
                <Link to={`/app/${encodeURIComponent(c.slug)}`}>{c.title}</Link>{" "}
                <span className="muted small">
                  {c.platforms.includes("any") ? "mod" : "community build"} by {c.author}
                  {c.adds.length > 0 && ` · adds ${c.adds.join(", ")}`}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      {!isDesktop && <p className="muted small">Community builds and mods install in the Slop app, beside the original.</p>}
    </div>
  );
}

/** "Update apps automatically", sandbox mode for every app, and save sync (the Slop app). */
function LibrarySettings({ onChanged }: { onChanged: () => void }) {
  const auto = useLoad(() => desktop.autoUpdateSetting(), []);
  const sandbox = useSandboxStatus();
  const saveSync = useLoad(() => desktop.saveSyncStatus().catch(() => null), []);
  const [autoOn, chooseAuto] = useChosen(auto.data ?? true);
  const [sandboxAll, chooseSandboxAll] = useChosen(sandbox.data?.settings.all ?? false);
  const [syncOn, chooseSync] = useChosen(saveSync.data ? saveSync.data.available && saveSync.data.on : false);
  return (
    <Popover label="Library settings" icon={GEAR_ICON}>
      <div className="lib-menu lib-settings">
        <label className="check">
          <input
            type="checkbox"
            checked={autoOn}
            onChange={(e) => {
              chooseAuto(e.target.checked);
              void desktop.autoUpdateSetting(e.target.checked).then(
                () => {
                  auto.reload();
                  onChanged();
                },
                () => chooseAuto(undefined),
              );
            }}
          />{" "}
          Update apps automatically <span className="muted small">(never while an app is running)</span>
        </label>
        {sandbox.data?.available && (
          <label className="check">
            <input
              type="checkbox"
              checked={sandboxAll}
              onChange={(e) => {
                chooseSandboxAll(e.target.checked);
                void desktop.setSandbox(null, e.target.checked).then(sandbox.reload, () => chooseSandboxAll(undefined));
              }}
            />{" "}
            Run every app in sandbox mode <span className="muted small">(unless you turned it off for one)</span>
          </label>
        )}
        {sandbox.data?.available && sandbox.data.limits && <p className="muted small">{sandbox.data.limits}</p>}
        {saveSync.data && (
          <label className="check">
            <input
              type="checkbox"
              checked={syncOn}
              disabled={!saveSync.data.available}
              onChange={(e) => {
                chooseSync(e.target.checked);
                void desktop.setSaveSync(e.target.checked).then(saveSync.reload, () => chooseSync(undefined));
              }}
            />{" "}
            Sync saves between my computers{" "}
            <span className="muted small">
              {saveSync.data.available ? (
                "(end-to-end encrypted: Slop can't read them; up to 20 MB per app)"
              ) : (
                <>
                  (uses your chat's encryption: <Link to="/chat">set up chat</Link> on this computer first)
                </>
              )}
            </span>
          </label>
        )}
      </div>
    </Popover>
  );
}

type Builds = NonNullable<Entry["update"]>["builds"];

/** Install the official app or a community build beside this one (DESIGN §6, §9). Community
 * builds of an installed original show in its Play button's list. */
function OtherBuilds({
  current,
  builds,
  others,
  onInstalled,
}: {
  current: InstalledApp;
  builds: Builds;
  others: Builds;
  onInstalled: () => void;
}) {
  const platform = useLoad(() => desktop.platformInfo(), []);
  const [target, setTarget] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [bringSaves, setBringSaves] = useState(true);
  const [playIt, setPlayIt] = useState(true);
  const os = platform.data?.platform;
  const options = others.filter((b) => os && b.platforms.includes(os));
  if (options.length === 0) return null;
  const chosen = options.find((b) => b.slug === target);
  const original = builds.find((b) => b.official);
  const install = async () => {
    if (!chosen) return;
    setBusy(true);
    setError(undefined);
    try {
      await desktop.install(chosen.slug);
      if (bringSaves) await desktop.copySaves(current.slug, chosen.slug);
      if (playIt && original) await desktop.setPlayChoice(original.slug, chosen.official ? null : chosen.slug);
      setTarget(undefined);
      window.dispatchEvent(new Event(LIBRARY_CHANGED));
      onInstalled();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="small build-switch">
      <label>
        Other builds{" "}
        <select value={target ?? ""} onChange={(e) => setTarget(e.target.value || undefined)}>
          <option value="">Choose one to install…</option>
          {options.map((b) => (
            <option key={b.slug} value={b.slug}>
              {b.official ? `${b.title} (official)` : `${b.title} (community)`}
            </option>
          ))}
        </select>
      </label>
      {chosen && (
        <div className="notice">
          <p>
            Install <b>{chosen.title}</b> beside <b>{current.title}</b>?{" "}
            {!chosen.official &&
              "Community builds are made by other people from the original's code, and checked and reviewed like any app."}{" "}
            Each build has its own saves.
          </p>
          <label className="check">
            <input type="checkbox" checked={bringSaves} onChange={(e) => setBringSaves(e.target.checked)} /> Copy my saves to{" "}
            {chosen.title} <span className="muted">(a fork may store saves differently)</span>
          </label>
          {original && (
            <label className="check">
              <input type="checkbox" checked={playIt} onChange={(e) => setPlayIt(e.target.checked)} /> Make it what{" "}
              {original.title}'s Play button starts <span className="muted">(change it any time from Play's list)</span>
            </label>
          )}
          <div className="actions">
            <button className="btn small ghost" onClick={() => setTarget(undefined)} disabled={busy}>
              Cancel
            </button>
            <button className="btn small slime" onClick={() => void install()} disabled={busy}>
              {busy ? "Installing…" : "Install"}
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/** An installed app's save folder: copies taken before sandboxed sessions, to go back to. */
function Saves({ slug }: { slug: string }) {
  const copies = useLoad(() => desktop.saveCopies(slug).catch(() => [] as SaveCopy[]), [slug]);
  const [done, setDone] = useState<string>();
  const [error, setError] = useState<string>();
  return (
    <div className="lib-saves">
      <h3 className="lib-col-h">Saves</h3>
      <p className="muted small">
        Kept in Slop's save folder for this app, the same in normal and sandbox mode, and kept when you uninstall. Slop copies
        them before each sandboxed session. Save sync is in the library's settings (the gear beside "Library").
      </p>
      {copies.data && copies.data.length === 0 && <p className="muted small">No copies yet.</p>}
      <ul className="plain">
        {copies.data?.map((c) => (
          <li key={c.id}>
            {new Date(c.taken_at).toLocaleString()} · {Math.max(1, Math.round(c.bytes / 1024))} KB{" "}
            <button
              className="btn small ghost"
              onClick={() => {
                setError(undefined);
                void desktop.restoreSaves(slug, c.id).then(
                  () => {
                    setDone(c.id);
                    copies.reload();
                  },
                  (err: unknown) => setError(err instanceof Error ? err.message : String(err)),
                );
              }}
            >
              Go back to this
            </button>
            {done === c.id && <span className="muted"> Restored. Your saves before that were copied too.</span>}
          </li>
        ))}
      </ul>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
