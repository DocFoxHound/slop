import { Link } from "react-router";

import { api } from "../api";
import { Sidebar } from "../components/Layout";
import { ErrorState, SectionHeader, Shelf } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { isDesktop } from "../platform";
import { useSession } from "../session";

/** Upcoming apps that are still alive, soonest deadline first. Dead projects are never promoted. */
async function upcoming(limit: number) {
  const [soon, beta] = await Promise.all([
    api.apps({ stage: "coming_soon", limit }),
    api.apps({ stage: "beta", limit }),
  ]);
  return [...soon.apps, ...beta.apps]
    .filter((a) => !a.is_dead)
    .sort((a, b) => (a.stage_deadline ?? "").localeCompare(b.stage_deadline ?? ""))
    .slice(0, limit);
}

export function Store() {
  useDocumentTitle(undefined);
  const { me } = useSession();
  const fresh = useLoad(() => api.apps({ stage: "released", sort: "recent", limit: 8 }), []);
  const soon = useLoad(() => upcoming(4), []);
  const categories = useLoad(() => api.categories(), []);
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
          {soon.error ? (
            <ErrorState error={soon.error} onRetry={soon.reload} />
          ) : (
            <Shelf apps={soon.data} empty="Nothing upcoming right now." />
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
