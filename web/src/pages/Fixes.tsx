// SPDX-License-Identifier: AGPL-3.0-or-later
// An app's To fix tab (/publish/{slug}/fixes): what SLAPorSLOP's checks found in its newest
// release (crates/slop-api/src/fixes.rs), what must and what should be fixed, each ready to copy
// for an AI coding agent, then suggestions that never hold anything up (no icon yet, say). Its own
// tab since 2026-11-20: at the foot of the Releases page, makers missed it. The tab counts the
// must and should items (MakerFrame.tsx).

import { Link, useParams } from "react-router";

import { api, ApiError, type AppFixes, type Fix } from "../api";
import { MakerFrame } from "../components/MakerFrame";
import { ErrorState, Loading } from "../components/ui";
import { playsInBrowser } from "../format";
import { useDocumentTitle, useInterval, useLoad } from "../hooks";
import { useSession } from "../session";

/** A fix, as a message for an AI coding agent. Exported for tests. */
export function fixPrompt(f: Fix): string {
  return (
    `SLAPorSLOP's checks of my app found this. ${f.problem}\nFix it in the repository: ${f.instruction}` +
    (f.guide_section ? `\n(SLAPorSLOP's publishing guide, section "${f.guide_section}", has the details.)` : "")
  );
}

/** A suggestion, as a message for an AI coding agent. Exported for tests. */
export function suggestionPrompt(f: Fix): string {
  return (
    `SLAPorSLOP suggests this for my app. ${f.problem}\nDo it in the repository: ${f.instruction}` +
    (f.guide_section ? `\n(SLAPorSLOP's publishing guide, section "${f.guide_section}", has the details.)` : "")
  );
}

const copy = (text: string) => void navigator.clipboard.writeText(text);

export function Fixes() {
  const { slug = "" } = useParams();
  const { me } = useSession();
  const app = useLoad(() => (me ? api.app(slug) : Promise.resolve(undefined)), [slug, me?.id]);
  const fixes = useLoad(() => (me ? api.fixes(slug) : Promise.resolve(undefined)), [slug, me?.id]);
  // A release still being checked: look again now and then, so what it finds shows by itself.
  useInterval(fixes.reload, fixes.data?.release?.status === "fetching" ? 15_000 : null);
  useDocumentTitle(app.data ? `${app.data.title}: to fix` : "To fix");

  if (me === undefined || app.loading) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <p>
          <Link to={`/signin?return_to=/publish/${slug}/fixes`}>Sign in</Link> to see what to fix.
        </p>
      </div>
    );
  }
  if (app.error || (app.data && !app.data.is_mine)) {
    return (
      <div className="page">
        <ErrorState error={app.error ?? new ApiError(404, "not_found", "")} />
      </div>
    );
  }
  if (fixes.error) {
    return (
      <div className="page">
        <ErrorState error={fixes.error} onRetry={fixes.reload} />
      </div>
    );
  }
  if (!app.data || !fixes.data) return <Loading />;
  const browser = playsInBrowser(app.data);

  return (
    <MakerFrame app={app.data} tab="fixes" fixes={fixes.data} onStageMoved={app.reload}>
      <div className="maker-cols">
        <div className="maker-main">
          <FixesPanel slug={slug} fixes={fixes.data} browser={browser} />
          <Suggestions list={fixes.data.recommendations ?? []} />
        </div>
        <div className="maker-side">
          <section className="panel panel-pad help" aria-labelledby="fixes-how">
            <h2 id="fixes-how">How it works</h2>
            <p className="small">
              SLAPorSLOP's checks run on each {browser ? "play address" : "release"} you send. <b>Must fix</b> items break
              SLAPorSLOP's rules; <b>should fix</b> items are allowed, but players see them or they cause trouble.
              Suggestions never hold anything up.
            </p>
            <p className="small">
              Fix them in your repository and push, then send a new {browser ? "play address" : "release"} with a new
              version{browser ? "" : " (Slop Build sends one on every push)"}: the checks run again on it.
            </p>
            <p className="small">
              Automated checks aren't perfect and some findings can be false positives. If a <b>must fix</b> item is a
              false alarm, tell SLAPorSLOP staff on the{" "}
              <Link to={`/publish/${slug}/releases`}>Releases page</Link> so a person can review it and allow publishing
              anyway.
            </p>
            <p className="small muted">
              Copy them to your AI coding agent, or <Link to="/account/agents">connect it to SLAPorSLOP</Link> and ask it to
              run <code>get_fixes</code>.
            </p>
          </section>
        </div>
      </div>
    </MakerFrame>
  );
}

/** What must and should be fixed in the newest release, each with a fix to hand to an AI agent. */
function FixesPanel({ slug, fixes, browser }: { slug: string; fixes: AppFixes; browser: boolean }) {
  const list = fixes.fixes;
  const release = fixes.release;
  const all = list.map(fixPrompt).join("\n\n") + `\n\n${fixes.next}`;
  return (
    <section className="panel panel-pad" id="fixes" aria-labelledby="fixes-title">
      <h2 id="fixes-title">{release ? `To fix in ${release.version}` : "To fix"}</h2>
      {!release ? (
        <p className="muted">
          Nothing yet: SLAPorSLOP's checks run on each {browser ? "play address" : "release"} you send, and what they
          find to fix shows here. <Link to={`/publish/${slug}/releases`}>{browser ? "Add the play address" : "Submit a release"}</Link>
        </p>
      ) : list.length === 0 ? (
        release.status === "fetching" ? (
          <p className="muted" role="status">
            SLAPorSLOP is still checking {release.version}: what it finds shows here.
          </p>
        ) : (
          <p className="muted">Nothing to fix in {release.version}.</p>
        )
      ) : (
        <>
          <p className="notice small" role="note">
            <b>Automated checks can have false positives.</b> Review each item before changing your code. If a{" "}
            <b>MUST FIX</b> finding is a false alarm, ask SLAPorSLOP staff on the{" "}
            <Link to={`/publish/${slug}/releases`}>Releases page</Link> to review it and allow publishing anyway.
          </p>
          <p className="muted small">
            Copy these to your AI coding agent, or connect it to SLAPorSLOP (Account → AI agents) and ask it to run get_fixes.
          </p>
          <ul className="fix-list">
            {list.map((f, i) => (
              <li key={`${f.kind}-${i}`}>
                <span className={`badge ${f.severity === "must" ? "bad" : "warn"}`}>
                  {f.severity === "must" ? "MUST FIX" : "SHOULD FIX"}
                </span>{" "}
                {f.problem}
                <p className="small">{f.instruction}</p>
                <button className="btn small ghost" type="button" onClick={() => copy(fixPrompt(f))}>
                  Copy for your AI
                </button>
              </li>
            ))}
          </ul>
          <p className="muted small">{fixes.next}</p>
          <button className="btn small" type="button" onClick={() => copy(all)}>
            Copy all for your AI
          </button>
        </>
      )}
    </section>
  );
}

/** Worth doing, never blocking (fixes.rs `recommendations`): after the fixes, and quieter. */
function Suggestions({ list }: { list: Fix[] }) {
  if (list.length === 0) return null;
  return (
    <section className="panel panel-pad" aria-labelledby="suggestions-title">
      <h2 id="suggestions-title">Worth doing</h2>
      <p className="muted small">Suggestions: none of them holds anything up.</p>
      <ul className="fix-list">
        {list.map((f, i) => (
          <li key={`${f.kind}-${i}`}>
            <span className="badge neutral">SUGGESTED</span> {f.problem}
            <p className="small">{f.instruction}</p>
            <button className="btn small ghost" type="button" onClick={() => copy(suggestionPrompt(f))}>
              Copy for your AI
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
