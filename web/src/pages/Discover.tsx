// SPDX-License-Identifier: AGPL-3.0-or-later
import { Link, useLocation } from "react-router";

import { api, type DiscoverReel } from "../api";
import { StoreFrame } from "../components/StoreNav";
import { ErrorState, Reel, SectionHeader } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { useSession } from "../session";

/**
 * Discover (the store bar): one request (`GET /v1/discover`, discover.rs) for every reel.
 * Recommended for you first, then reels from the genres the person plays, likes, keeps and
 * supports, with All-time favourites, Up-and-coming and Popular this week and month mixed in.
 * Each reel says what it's based on. Signed out or with recommendations switched off, the
 * page is the general reels only, and says why.
 */
export function Discover() {
  useDocumentTitle("Discover");
  const { me } = useSession();
  // Wait for the session, so the page loads once, for the right person.
  const page = useLoad(() => (me === undefined ? new Promise<never>(() => {}) : api.discover()), [me === undefined ? undefined : (me?.id ?? null)]);
  const off = page.data?.off;
  return (
    <StoreFrame>
      <h1 className="page-title">Discover</h1>
      {off === "signed_out" && <SignInNudge />}
      {off === "switched_off" && (
        <div className="panel panel-pad signin-nudge">
          <p>
            <b>Recommendations based on your activity are off.</b> So this page is the same for everyone today: nothing you do is
            used to pick it. You can turn them on on your account page.
          </p>
          <Link className="btn" to="/account">
            Settings
          </Link>
        </div>
      )}
      {page.error ? (
        <ErrorState error={page.error} onRetry={page.reload} />
      ) : !page.data ? (
        <Reel apps={undefined} empty="" label="Loading" />
      ) : page.data.reels.length === 0 ? (
        <div className="empty-state">Nothing to discover yet.</div>
      ) : (
        page.data.reels.map((r) => <ReelSection key={r.key} reel={r} />)
      )}
    </StoreFrame>
  );
}

function ReelSection({ reel }: { reel: DiscoverReel }) {
  return (
    <section aria-label={reel.title} className="discover-reel">
      <SectionHeader title={reel.title} sub={reel.why}>
        {reel.key === "for_you" && (
          <Link to="/account" className="sub">
            Settings
          </Link>
        )}
        {reel.see_all && (
          <Link to={reel.see_all} className="sub" aria-label={`See all: ${reel.title}`}>
            See all →
          </Link>
        )}
      </SectionHeader>
      <Reel apps={reel.apps} empty="Nothing here yet." label={reel.title} />
    </section>
  );
}

/** For visitors: what signing in adds, and a way to. */
function SignInNudge() {
  const location = useLocation();
  return (
    <div className="panel panel-pad signin-nudge">
      <p>
        <b>Picks of your own.</b> Sign in and SLAPorSLOP recommends apps from what's in your library, what you play, how you vote and
        what you support, with something different mixed in. You can turn it off on your account page.
      </p>
      {isDesktop ? (
        <button className="btn slime" onClick={() => void desktop.showLogin()}>
          Sign in
        </button>
      ) : (
        <Link className="btn slime" to={`/signin?return_to=${encodeURIComponent(location.pathname)}`}>
          Sign in for your picks
        </Link>
      )}
    </div>
  );
}
