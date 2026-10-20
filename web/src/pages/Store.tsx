import { useState } from "react";
import { Link } from "react-router";

import { api } from "../api";
import { Sidebar } from "../components/Layout";
import { ErrorState, SectionHeader, Shelf } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { isDesktop } from "../platform";
import { useSession } from "../session";

/**
 * Coming soon & in beta (DESIGN §5): "For you" matches the tags in your library and Interested
 * list; "Most anticipated" ranks by Interested count. Dead projects are never promoted.
 */
function UpcomingShelf() {
  const { me } = useSession();
  const [view, setView] = useState<"for_you" | "anticipated">("for_you");
  const shelf = useLoad(
    () =>
      view === "for_you" && me
        ? api.interested.forYou().then((apps) => apps.slice(0, 8))
        : api
            .apps({ sort: "anticipated", limit: 16 })
            .then((p) => p.apps.filter((a) => a.stage !== "released" && !a.is_dead).slice(0, 8)),
    [view, me?.id],
  );
  return (
    <>
      {me && (
        <div className="seg small-seg" role="tablist" aria-label="Upcoming apps">
          {(
            [
              ["for_you", "For you"],
              ["anticipated", "Most anticipated"],
            ] as const
          ).map(([key, label]) => (
            <button key={key} role="tab" aria-selected={view === key} className={view === key ? "on" : ""} onClick={() => setView(key)}>
              {label}
            </button>
          ))}
        </div>
      )}
      {shelf.error ? (
        <ErrorState error={shelf.error} onRetry={shelf.reload} />
      ) : (
        <Shelf apps={shelf.data} empty="Nothing upcoming right now." />
      )}
    </>
  );
}

export function Store() {
  useDocumentTitle(undefined);
  const { me } = useSession();
  const fresh = useLoad(() => api.apps({ stage: "released", sort: "recent", limit: 8 }), []);
  const categories = useLoad(() => api.categories(), []);
  const community = useLoad(() => api.apps({ community: true, sort: "rating", limit: 4 }).then((p) => p.apps), []);
  const server = useLoad(() => api.server(), []);
  const trending = useLoad(
    () =>
      api
        .apps({ sort: "trending", limit: 12 })
        .then((page) => page.apps.filter((a) => a.stage !== "coming_soon" && !a.is_dead).slice(0, 8)),
    [],
  );

  return (
    <div className="page">
      <div className="store">
        <Sidebar />
        <div>
          {server.data && !server.data.scanning && (
            <p className="notice" role="note">
              <b>Test server.</b> Automatic scanning isn't running here yet, so uploads are not scanned.
            </p>
          )}
          {/* What Slop is: for visitors who haven't signed in, and never in the desktop app. */}
          {me === null && !isDesktop && (
          <section className="intro panel">
            <h1>
              Apps and games made with AI. <span className="slime">Open source, every one.</span>
            </h1>
            <ul>
              <li>
                <b>Source you can read.</b> Every app links its code, under an open-source licence.
              </li>
              {server.data?.scanning ? (
                <li>
                  <b>Scanned before it's listed.</b> Malware, stolen code and scams are checked on every upload.
                </li>
              ) : (
                <li>
                  <b>Reviewed before it's listed.</b>{" "}
                  {server.data?.maker_signatures_required
                    ? "Every build is signed by its maker and checked by a person."
                    : "Every build is checked by a person."}
                </li>
              )}
              <li>
                <b>Free to download.</b> Makers can take donations. Nothing is ever for sale.
              </li>
            </ul>
          </section>
          )}

          {me && <ForYouShelf />}

          <SectionHeader title="Trending" sub="Most played this week" />
          {trending.error ? (
            <ErrorState error={trending.error} onRetry={trending.reload} />
          ) : (
            <Shelf apps={trending.data} empty="Nothing to play yet." />
          )}

          <SectionHeader title="Coming soon & in beta" sub="Projects on their way to release">
            <Link to="/upcoming" className="sub">
              See all →
            </Link>
          </SectionHeader>
          <UpcomingShelf />

          <SectionHeader title="Community branches worth installing" sub="Mods, fixes, translations and ports from forks">
            <Link to="/community" className="sub">
              See all →
            </Link>
          </SectionHeader>
          {community.error ? (
            <ErrorState error={community.error} onRetry={community.reload} />
          ) : (
            <Shelf apps={community.data} empty="No community builds yet." />
          )}

          <SectionHeader title="Browse by category">
            <Link to="/browse" className="sub">
              All tags →
            </Link>
          </SectionHeader>
          <div className="cat-grid">
            {categories.data?.map((c) => (
              <Link className="cat" key={c.slug} to={`/browse/${c.slug}`}>
                <b>{c.name}</b>
                <span>{c.description}</span>
              </Link>
            ))}
          </div>

          <SectionHeader title="New & updated" />
          {fresh.error ? (
            <ErrorState error={fresh.error} onRetry={fresh.reload} />
          ) : (
            <Shelf
              apps={fresh.data?.apps}
              empty={
                <>
                  Nothing released yet. <Link to="/publish">Be the first to publish</Link>.
                </>
              }
            />
          )}
        </div>
      </div>
    </div>
  );
}

/** Recommendations from the user's own activity, with discovery mixed in (foryou.rs). */
function ForYouShelf() {
  const feed = useLoad(() => api.forYou(), []);
  if (feed.error) return null;
  const f = feed.data;
  const sub = !f
    ? undefined
    : f.personalized
      ? `Based on ${f.based_on.join(", ")}, plus something different every fourth`
      : "Popular, well rated and new. Add apps to your library and play them for picks of your own.";
  return (
    <>
      <SectionHeader title="For you" sub={sub}>
        <Link to="/account" className="sub">
          Settings
        </Link>
      </SectionHeader>
      <Shelf apps={f?.items} empty="Nothing to recommend yet." />
    </>
  );
}
