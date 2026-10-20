// List pages: search, a category, a tag, and upcoming apps. All share one filterable grid.

import { useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { api, type AppQuery, type AppStage, type AppSummary, type Category } from "../api";
import { detectOs, OS_LABEL } from "../platform";
import { Sidebar } from "../components/Layout";
import { ErrorState, Loading, Shelf } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";

const PAGE = 24;

const DEFAULT_SORTS: Array<NonNullable<AppQuery["sort"]>> = ["recent", "trending", "popular", "rating", "reviewed", "title"];

/** The platform filter's "my platform", where Slop has builds for it. */
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
  const mine = myPlatform();
  const [extra, setExtra] = useState<{ key: string; apps: AppSummary[]; next: number | null }>();

  const filters: AppQuery = { ...base, stage: stage || base.stage, upcoming, platform, min_not_slop, sort };
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
    const page = await api.apps({ ...filters, limit: PAGE, offset: next });
    setExtra({ key, apps: [...(pages?.apps ?? []), ...page.apps], next: page.next_offset });
  };

  const sortLabels: Record<string, string> = {
    relevance: "Best match",
    trending: "Most played this week",
    popular: "Popular this month",
    anticipated: "Most anticipated",
    rating: "Most Not Slop",
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
          Not Slop{" "}
          <select value={min_not_slop ?? ""} onChange={(e) => set("not_slop", e.target.value)}>
            <option value="">Any rating</option>
            <option value="70">70% or more</option>
            <option value="80">80% or more</option>
            <option value="95">95% or more</option>
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
    </>
  );
}

export function Search() {
  const [params] = useSearchParams();
  const q = (params.get("q") ?? "").trim();
  useDocumentTitle(q ? `Search: ${q}` : "Everything");
  return (
    <div className="page">
      <div className="store">
        <Sidebar />
        <div>
          <div className="crumbs">
            <Link to="/">Store</Link> › Search
          </div>
          <h1 className="page-title">{q ? `Results for “${q}”` : "Everything"}</h1>
          <AppGrid base={{ q: q || undefined }} />
        </div>
      </div>
    </div>
  );
}

export function MadeWithPeople() {
  useDocumentTitle("Made with people");
  return (
    <div className="page">
      <div className="store">
        <Sidebar />
        <div>
          <div className="crumbs">
            <Link to="/">Store</Link> › Made with people
          </div>
          <h1 className="page-title">✋ Made with people</h1>
          <p className="muted">
            Apps whose makers hired artists, composers, writers and other creatives through <Link to="/people">People</Link>. Their pages
            credit who made what.
          </p>
          <AppGrid base={{ made_with_people: true }} />
        </div>
      </div>
    </div>
  );
}

export function Community() {
  useDocumentTitle("Community builds");
  return (
    <div className="page">
      <div className="store">
        <Sidebar />
        <div>
          <div className="crumbs">
            <Link to="/">Store</Link> › Community builds
          </div>
          <h1 className="page-title">Community builds</h1>
          <p className="muted">
            Branches of apps, built from forks: mods, fixes, translations and ports. Each is made by its author, not the
            original publisher, and checked the same way as every app on Slop.
          </p>
          <AppGrid base={{ community: true }} sorts={["rating", "recent", "title"]} />
        </div>
      </div>
    </div>
  );
}

export function Upcoming() {
  useDocumentTitle("Coming soon & beta");
  const [params, setParams] = useSearchParams();
  const stage = params.get("stage") === "beta" ? "beta" : "coming_soon";
  return (
    <div className="page">
      <div className="store">
        <Sidebar />
        <div>
          <div className="crumbs">
            <Link to="/">Store</Link> › Upcoming
          </div>
          <h1 className="page-title">Coming soon &amp; beta</h1>
          <p className="muted">
            Coming soon pages have 30 days to reach beta, and betas have a year to fully release. Projects with no code
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
        </div>
      </div>
    </div>
  );
}

export function Browse() {
  useDocumentTitle("Categories");
  const categories = useLoad(() => api.categories(), []);
  return (
    <div className="page">
      <div className="store">
        <Sidebar />
        <div>
          <h1 className="page-title">All categories</h1>
          <p className="muted">Broad categories on top, specific tags underneath. Apps have several tags, so they show up in more than one place.</p>
          {categories.error && <ErrorState error={categories.error} onRetry={categories.reload} />}
          {!categories.data && !categories.error && <Loading />}
          {categories.data?.map((c) => (
            <div className="browse-group" key={c.slug}>
              <h3>
                <Link to={`/browse/${c.slug}`}>{c.name}</Link> <span className="muted">{c.description}</span>
              </h3>
              <TagCloud category={c} />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function TagCloud({ category }: { category: Category }) {
  return (
    <div className="tag-cloud">
      {category.tags.map((t) => (
        <Link key={t.slug} to={`/tag/${t.slug}`}>
          {t.name}
        </Link>
      ))}
    </div>
  );
}

export function CategoryPage() {
  const { slug = "" } = useParams();
  const categories = useLoad(() => api.categories(), []);
  const category = categories.data?.find((c) => c.slug === slug);
  useDocumentTitle(category?.name);
  if (categories.data && !category) return <NotFound />;
  return (
    <div className="page">
      <div className="store">
        <Sidebar active={slug} />
        <div>
          <div className="crumbs">
            <Link to="/">Store</Link> › <Link to="/browse">Categories</Link> › {category?.name}
          </div>
          <h1 className="page-title">{category?.name ?? " "}</h1>
          <div className="muted">{category?.description}</div>
          {category && <TagCloud category={category} />}
          <AppGrid base={{ category: slug }} />
        </div>
      </div>
    </div>
  );
}

export function TagPage() {
  const { slug = "" } = useParams();
  const categories = useLoad(() => api.categories(), []);
  const category = categories.data?.find((c) => c.tags.some((t) => t.slug === slug));
  const tag = category?.tags.find((t) => t.slug === slug);
  useDocumentTitle(tag?.name);
  if (categories.data && !tag) return <NotFound />;
  return (
    <div className="page">
      <div className="store">
        <Sidebar active={category?.slug} />
        <div>
          <div className="crumbs">
            <Link to="/">Store</Link> › <Link to="/browse">Categories</Link> ›{" "}
            {category && <Link to={`/browse/${category.slug}`}>{category.name}</Link>} › {tag?.name}
          </div>
          <h1 className="page-title"># {tag?.name ?? slug}</h1>
          <AppGrid base={{ tag: slug }} />
        </div>
      </div>
    </div>
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
