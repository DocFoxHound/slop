// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Link } from "react-router";

import { api } from "../api";
import { FeaturedCarousel } from "../components/Featured";
import { StoreFrame } from "../components/StoreNav";
import { CategoryIcon } from "../components/StoreIcons";
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
  const featured = useLoad(() => api.featured(), []);
  // Staff-made shelves, and the automatic ones staff hid (shelves.rs).
  const shelves = useLoad(() => api.shelves(), []);
  const show = (key: string) => !shelves.data?.hidden.includes(key);
  const popular = useLoad(
    () =>
      api
        .apps({ sort: "popular", limit: 12 })
        .then((page) => page.apps.filter((a) => a.stage !== "coming_soon" && !a.is_dead).slice(0, 8)),
    [],
  );
  const notSlop = useLoad(() => api.apps({ sort: "not_slop_now", limit: 8 }).then((p) => p.apps), []);
  const freshSlop = useLoad(() => api.apps({ sort: "fresh_slop", limit: 4 }).then((p) => p.apps), []);
  const barelySeen = useLoad(() => api.apps({ sort: "barely_seen", limit: 4 }).then((p) => p.apps), []);
  const glowUps = useLoad(
    () => api.glowUps().then((list) => list.map((a) => ({ ...a, reason: `${a.before}% → ${a.after}% Slaps` }))),
    [],
  );

  return (
    <StoreFrame>
      {server.data && !server.data.scanning && (
        <p className="notice" role="note">
          <b>Test server.</b> Automatic scanning isn't running here yet, so uploads are not scanned.
        </p>
      )}
      {/* What SLAPorSLOP is: for visitors who haven't signed in, and never in the desktop app. */}
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

      {featured.data && <FeaturedCarousel slides={featured.data} />}

      {me && <ForYouShelf />}

      {shelves.data?.shelves.map((s) => (
        <section key={s.slug} aria-label={s.title}>
          <SectionHeader title={s.title} sub={s.blurb || undefined}>
            <Link to={`/shelf/${s.slug}`} className="sub">
              See all →
            </Link>
          </SectionHeader>
          <Shelf apps={s.apps.slice(0, 8)} empty="" />
        </section>
      ))}

      {show("popular") && (
      <>
      <SectionHeader title="Popular this month" sub="Most added to libraries this month">
        <Link to="/search?sort=popular" className="sub">
          See all →
        </Link>
      </SectionHeader>
      {popular.error ? (
        <ErrorState error={popular.error} onRetry={popular.reload} />
      ) : (
        <Shelf apps={popular.data} empty="Nothing to play yet." />
      )}
      </>
      )}

      {show("upcoming") && (
      <>
      <SectionHeader title="Coming soon & in beta" sub="Projects on their way to release">
        <Link to="/upcoming" className="sub">
          See all →
        </Link>
      </SectionHeader>
      <UpcomingShelf />
      </>
      )}

      {show("categories") && (
      <>
      <SectionHeader title="Browse by category">
        <Link to="/browse" className="sub">
          All tags →
        </Link>
      </SectionHeader>
      <div className="cat-grid">
        {categories.data?.map((c) => (
          <Link className="cat" key={c.slug} to={`/browse/${c.slug}`}>
            <span className="cat-icon">
              <CategoryIcon slug={c.slug} size={26} />
            </span>
            <span className="cat-text">
              <b>{c.name}</b>
              <span>{c.description}</span>
            </span>
          </Link>
        ))}
      </div>
      </>
      )}

      {show("not_slop") && (
      <>
      <SectionHeader title="Most Slaps right now" sub="Best rated by votes from the last 30 days" />
      {notSlop.error ? (
        <ErrorState error={notSlop.error} onRetry={notSlop.reload} />
      ) : (
        <Shelf apps={notSlop.data} empty="Not enough recent votes yet." />
      )}
      </>
      )}

      {show("barely_seen") && barelySeen.data && barelySeen.data.length > 0 && (
        <>
          <SectionHeader title="New and barely seen" sub="Released lately, in few libraries yet. A different pick every day." />
          <Shelf apps={barelySeen.data} empty="" />
        </>
      )}

      {show("glow_ups") && glowUps.data && glowUps.data.length > 0 && (
        <>
          <SectionHeader title="Glow-ups" sub="Updates turned these from Slop toward Slaps in the last 60 days" />
          <Shelf apps={glowUps.data} empty="" />
        </>
      )}

      {show("community") && (
      <>
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
      </>
      )}

      {show("fresh") && (
      <>
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

      </>
      )}

      {show("fresh_slop") && freshSlop.data && freshSlop.data.length > 0 && (
        <>
          <SectionHeader title="Fresh slop" sub="The most Slop votes this month. For fun: be kind in reviews." />
          <Shelf apps={freshSlop.data} empty="" />
        </>
      )}
    </StoreFrame>
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
