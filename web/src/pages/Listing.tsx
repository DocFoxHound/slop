// SPDX-License-Identifier: AGPL-3.0-or-later
// List pages: search, upcoming apps, a tag, and everything in a category share one filterable grid.
// Browse and a category's page are reels instead: the top of each category, or of each sub-genre.

import { useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { ACCESSIBILITY, api, ApiError, type AppQuery, type AppStage, type AppSummary, type Category } from "../api";
import { CONTENT_LABEL } from "../format";
import { detectOs, OS_LABEL } from "../platform";
import { CategorySidebar, CategoryTree, StoreFrame } from "../components/StoreNav";
import { CategoryIcon, StoreIcon, TagIcon } from "../components/StoreIcons";
import { ErrorState, Loading, Reel, Shelf } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";

const PAGE = 24;

const DEFAULT_SORTS: Array<NonNullable<AppQuery["sort"]>> = [
  "recent",
  "trending",
  "popular",
  "rating",
  "not_slop_now",
  "reviewed",
  "fresh_slop",
  "title",
];

/** The platform filter's "my platform", where SLAPorSLOP has builds for it. */
function myPlatform(): "windows" | "linux" | undefined {
  const os = detectOs();
  return os === "windows" || os === "linux" ? os : undefined;
}

function AppGrid({ base, sorts = DEFAULT_SORTS }: { base: AppQuery; sorts?: Array<NonNullable<AppQuery["sort"]>> }) {
  const [params, setParams] = useSearchParams();
  const show = params.get("stage") ?? "";
  const stage = (["released", "beta", "coming_soon"].includes(show) ? show : "") as AppStage | "";
  const upcoming = show === "upcoming" ? true : undefined;
  const sort = (params.get("sort") ?? (base.q ? "relevance" : sorts[0])) as NonNullable<AppQuery["sort"]>;
  const platformParam = params.get("runs_on");
  const platform = platformParam === "windows" || platformParam === "linux" ? platformParam : undefined;
  const minParam = Number(params.get("not_slop"));
  const min_not_slop = minParam > 0 && minParam <= 100 ? minParam : undefined;
  const accessibility = params.get("accessibility") ?? undefined;
  const mine = myPlatform();
  const [extra, setExtra] = useState<{ key: string; apps: AppSummary[]; next: number | null }>();
  const [moreError, setMoreError] = useState<{ key: string; message: string }>();

  const filters: AppQuery = { ...base, stage: stage || base.stage, upcoming, platform, min_not_slop, accessibility, sort };
  const key = JSON.stringify(filters);
  const first = useLoad(() => api.apps({ ...filters, limit: PAGE }), [key]);
  const pages = extra?.key === key ? extra : undefined;
  const apps = first.data ? [...first.data.apps, ...(pages?.apps ?? [])] : undefined;
  const next = pages ? pages.next : first.data?.next_offset;

  const set = (name: string, value: string) => {
    const p = new URLSearchParams(params);
    if (value) p.set(name, value);
    else p.delete(name);
    setParams(p, { replace: true });
  };

  const more = async () => {
    if (next == null) return;
    setMoreError(undefined);
    try {
      const page = await api.apps({ ...filters, limit: PAGE, offset: next });
      setExtra({ key, apps: [...(pages?.apps ?? []), ...page.apps], next: page.next_offset });
    } catch (err) {
      setMoreError({ key, message: err instanceof ApiError ? err.message : String(err) });
    }
  };

  const sortLabels: Record<string, string> = {
    relevance: "Best match",
    trending: "Most played this week",
    popular: "Popular this month",
    anticipated: "Most anticipated",
    rating: "Most Slaps",
    not_slop_now: "Most Slaps right now",
    fresh_slop: "Fresh slop (most 👎 this month)",
    reviewed: "Most reviewed",
    recent: "Recently updated",
    title: "A–Z",
  };
  const sortOptions = base.q ? ["relevance", ...sorts] : sorts;

  return (
    <>
      <div className="filters">
        <label>
          Sort{" "}
          <select value={sort} onChange={(e) => set("sort", e.target.value)}>
            {sortOptions.map((s) => (
              <option key={s} value={s}>
                {sortLabels[s]}
              </option>
            ))}
          </select>
        </label>
        {!base.stage && (
          <label>
            Show{" "}
            <select value={show} onChange={(e) => set("stage", e.target.value)}>
              <option value="">Everything</option>
              <option value="released">Released</option>
              <option value="upcoming">Upcoming (coming soon and beta)</option>
              <option value="beta">Beta</option>
              <option value="coming_soon">Coming soon</option>
            </select>
          </label>
        )}
        <label>
          Runs on{" "}
          <select value={platform ?? ""} onChange={(e) => set("runs_on", e.target.value)}>
            <option value="">Any platform</option>
            {(["windows", "linux"] as const).map((p) => (
              <option key={p} value={p}>
                {OS_LABEL[p]}
                {p === mine ? " (this computer)" : ""}
              </option>
            ))}
          </select>
        </label>
        <label>
          Slaps{" "}
          <select value={min_not_slop ?? ""} onChange={(e) => set("not_slop", e.target.value)}>
            <option value="">Any rating</option>
            <option value="70">70% or more</option>
            <option value="80">80% or more</option>
            <option value="95">95% or more</option>
          </select>
        </label>
        <label>
          Accessibility{" "}
          <select value={accessibility ?? ""} onChange={(e) => set("accessibility", e.target.value)}>
            <option value="">Any</option>
            {ACCESSIBILITY.map(([key, name]) => (
              <option key={key} value={key}>
                {name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {platform && <p className="muted small">Includes apps that run on {OS_LABEL[platform]} through a community build.</p>}
      {first.error ? (
        <ErrorState error={first.error} onRetry={first.reload} />
      ) : (
        <Shelf apps={apps} empty="Nothing matches. Try another search or filter." />
      )}
      {next != null && (
        <div className="more">
          <button className="btn" onClick={() => void more()}>
            Show more
          </button>
        </div>
      )}
      {moreError?.key === key && (
        <p className="form-error" role="alert">
          {moreError.message}
        </p>
      )}
    </>
  );
}

export function Search() {
  const [params] = useSearchParams();
  const q = (params.get("q") ?? "").trim();
  useDocumentTitle(q ? `Search: ${q}` : "Everything");
  return (
    <StoreFrame sidebar>
      <div className="crumbs">
        <Link to="/">Store</Link> › Search
      </div>
      <h1 className="page-title">{q ? `Results for “${q}”` : "Everything"}</h1>
      <AppGrid base={{ q: q || undefined }} />
    </StoreFrame>
  );
}

export function MadeWithPeople() {
  useDocumentTitle("Made with people");
  return (
    <StoreFrame sidebar>
      <div className="crumbs">
        <Link to="/">Store</Link> › Made with people
      </div>
      <h1 className="page-title with-icon">
        <StoreIcon name="made" size={28} />
        Made with people
      </h1>
      <p className="muted">
        Apps whose makers hired artists, composers, writers and other creatives through <Link to="/people">People</Link>. Their pages
        credit who made what.
      </p>
      <AppGrid base={{ made_with_people: true }} />
    </StoreFrame>
  );
}

export function Community() {
  useDocumentTitle("Community builds");
  return (
    <StoreFrame sidebar>
      <div className="crumbs">
        <Link to="/">Store</Link> › Community builds
      </div>
      <h1 className="page-title">Community builds</h1>
      <p className="muted">
        Branches of apps, built from forks: mods, fixes, translations and ports. Each is made by its author, not the
        original publisher, and checked the same way as every app on SLAPorSLOP.
      </p>
      <AppGrid base={{ community: true }} sorts={["rating", "recent", "title"]} />
    </StoreFrame>
  );
}

export function Upcoming() {
  useDocumentTitle("Coming soon & beta");
  const [params, setParams] = useSearchParams();
  const stage = params.get("stage") === "beta" ? "beta" : "coming_soon";
  return (
    <StoreFrame sidebar>
      <div className="crumbs">
        <Link to="/">Store</Link> › Upcoming
      </div>
      <h1 className="page-title">Coming soon &amp; beta</h1>
      <p className="muted">
        Coming soon pages have a year to reach beta, and betas have a year to fully release. Projects with no code
        changes for 90 days are marked dead.
      </p>
      <div className="seg" role="tablist">
        {(["coming_soon", "beta"] as const).map((s) => (
          <button
            key={s}
            role="tab"
            aria-selected={stage === s}
            className={stage === s ? "on" : ""}
            onClick={() => setParams({ stage: s }, { replace: true })}
          >
            {s === "beta" ? "Beta" : "Coming soon"}
          </button>
        ))}
      </div>
      <AppGrid base={{ stage }} sorts={["anticipated", "recent", "title"]} />
    </StoreFrame>
  );
}

/** What every Browse reel is, under the page's title. */
const REEL_NOTE = "The most played this week, topped up in a quiet week with what's being added to libraries and rated best.";

/** Browse: the categories in a tree on the left, and the top of each, one reel apiece. */
export function Browse() {
  useDocumentTitle("Browse");
  const categories = useLoad(() => api.categories(), []);
  const top = useLoad(() => api.browse(), []);
  return (
    <StoreFrame sidebar={<CategoryTree categories={categories.data} />}>
      <h1 className="page-title">All categories</h1>
      <p className="muted">{REEL_NOTE} Open a category for its sub-genres.</p>
      {categories.error && <ErrorState error={categories.error} onRetry={categories.reload} />}
      {top.error && <ErrorState error={top.error} onRetry={top.reload} />}
      {!categories.data && !categories.error && <Loading />}
      {!top.error &&
        categories.data?.map((c) => (
          <section className="browse-section" key={c.slug} aria-labelledby={`browse-${c.slug}`}>
            <div className="section-h browse-head">
              <h2 id={`browse-${c.slug}`}>
                <Link to={`/browse/${c.slug}`}>
                  <CategoryIcon slug={c.slug} size={22} />
                  {c.name}
                </Link>
              </h2>
              <span className="sub">{c.description}</span>
              <Link to={`/browse/${c.slug}`} className="sub">
                Open<span className="sr-only"> {c.name}</span> →
              </Link>
            </div>
            <Reel
              apps={top.data?.reels.find((r) => r.slug === c.slug)?.apps}
              empty="Nothing here yet."
              label={`Top of ${c.name} this week`}
            />
          </section>
        ))}
    </StoreFrame>
  );
}

/** What a category with an age limit (Kids & Family) leaves out, so a missing app isn't a mystery. */
function AgeLimit({ category }: { category: Category }) {
  if (category.max_age == null) return null;
  const labels = (category.excluded_labels ?? []).map((l) => CONTENT_LABEL[l].toLowerCase());
  const without = labels.length > 1 ? `${labels.slice(0, -1).join(", ")} or ${labels[labels.length - 1]}` : labels[0];
  return (
    <p className="muted small age-limit">
      Only apps rated {category.max_age === 0 ? "for everyone" : `${category.max_age}+ or under`}
      {without ? `, without ${without}` : ""}. Makers rate and label their own apps; you can report one that's wrong
      from its page.
    </p>
  );
}

/**
 * A category (Games, Kids & Family…: what the store bar opens): its own sidebar of sub-genres, and
 * the top of each sub-genre, one reel apiece. Sub-genres with nothing in them yet are listed after
 * the reels. Everything in the category, with sort and filters, is its own page (`CategoryAll`),
 * so this one stays one request and the filters never seem to apply to the reels.
 */
export function CategoryPage() {
  const { slug = "" } = useParams();
  const categories = useLoad(() => api.categories(), []);
  const category = categories.data?.find((c) => c.slug === slug);
  const top = useLoad(() => api.browse(slug), [slug]);
  useDocumentTitle(category?.name);
  if (categories.data && !category) return <NotFound />;
  // Not the last category's reels while the next one's load (the page stays mounted).
  const reels = top.loading ? undefined : top.data?.reels;
  const full = reels?.filter((r) => r.apps.length > 0);
  const quiet = reels?.filter((r) => r.apps.length === 0);
  return (
    <StoreFrame sidebar={<CategorySidebar category={category} on="" />}>
      <div className="crumbs">
        <Link to="/">Store</Link> › <Link to="/browse">Categories</Link> › {category?.name}
      </div>
      <h1 className="page-title with-icon">
        {category && <CategoryIcon slug={category.slug} size={28} />}
        {category?.name ?? " "}
      </h1>
      <div className="muted">{category?.description}</div>
      {category && <AgeLimit category={category} />}
      {category && (
        <p className="browse-all">
          <Link to={`/browse/${category.slug}/all`} className="btn">
            <StoreIcon name="everything" />
            Everything in {category.name} →
          </Link>
          <span className="muted small">Every app in it, with sort and filters.</span>
        </p>
      )}
      <p className="muted small">{REEL_NOTE}</p>
      {top.error ? (
        <ErrorState error={top.error} onRetry={top.reload} />
      ) : !full || !category ? (
        <Loading />
      ) : full.length === 0 ? (
        <div className="empty-state">Nothing in {category.name} yet.</div>
      ) : (
        full.map((r) => (
          <section className="browse-section" key={r.slug} aria-labelledby={`browse-${r.slug}`}>
            <div className="section-h browse-head">
              <h2 id={`browse-${r.slug}`}>
                <Link to={`/tag/${r.slug}`}>
                  <TagIcon slug={r.slug} category={category.slug} size={20} />
                  {r.name}
                </Link>
              </h2>
              <Link to={`/tag/${r.slug}`} className="sub">
                See all<span className="sr-only"> {r.name}</span> →
              </Link>
            </div>
            <Reel apps={r.apps} empty="Nothing here yet." label={`Top of ${r.name} this week`} />
          </section>
        ))
      )}
      {category && quiet && quiet.length > 0 && (
        <div className="browse-quiet">
          <h2 className="section-title">Nothing here yet</h2>
          <div className="chips">
            {quiet.map((r) => (
              <Link key={r.slug} to={`/tag/${r.slug}`} className="chip">
                <TagIcon slug={r.slug} category={category.slug} size={14} />
                {r.name}
              </Link>
            ))}
          </div>
        </div>
      )}
    </StoreFrame>
  );
}

/** Everything in a category, with the sort and filters of every list page. */
export function CategoryAll() {
  const { slug = "" } = useParams();
  const categories = useLoad(() => api.categories(), []);
  const category = categories.data?.find((c) => c.slug === slug);
  useDocumentTitle(category && `Everything in ${category.name}`);
  if (categories.data && !category) return <NotFound />;
  return (
    <StoreFrame sidebar={<CategorySidebar category={category} on="all" />}>
      <div className="crumbs">
        <Link to="/">Store</Link> › <Link to="/browse">Categories</Link> ›{" "}
        {category && <Link to={`/browse/${category.slug}`}>{category.name}</Link>} › Everything
      </div>
      <h1 className="page-title with-icon">
        {category && <CategoryIcon slug={category.slug} size={28} />}
        {category ? `Everything in ${category.name}` : " "}
      </h1>
      {category && <AgeLimit category={category} />}
      <AppGrid base={{ category: slug }} />
    </StoreFrame>
  );
}

/** A sub-genre: its category's sidebar with it marked, and all of it, with sort and filters. */
export function TagPage() {
  const { slug = "" } = useParams();
  const categories = useLoad(() => api.categories(), []);
  const category = categories.data?.find((c) => c.tags.some((t) => t.slug === slug));
  const tag = category?.tags.find((t) => t.slug === slug);
  useDocumentTitle(tag?.name);
  if (categories.data && !tag) return <NotFound />;
  return (
    <StoreFrame sidebar={<CategorySidebar category={category} on={slug} />}>
      <div className="crumbs">
        <Link to="/">Store</Link> › <Link to="/browse">Categories</Link> ›{" "}
        {category && <Link to={`/browse/${category.slug}`}>{category.name}</Link>} › {tag?.name}
      </div>
      <h1 className="page-title with-icon">
        {category && <TagIcon slug={slug} category={category.slug} size={28} />}
        {tag?.name ?? " "}
      </h1>
      {category && <AgeLimit category={category} />}
      <AppGrid base={{ tag: slug }} />
    </StoreFrame>
  );
}

export function NotFound() {
  useDocumentTitle("Not found");
  return (
    <div className="page">
      <div className="empty-state">
        Page not found. <Link to="/">Back to the store</Link>
      </div>
    </div>
  );
}
