// SPDX-License-Identifier: AGPL-3.0-or-later
// The maker's workspace for one app: its name and where it stands, the stage bar (Coming soon →
// Beta → Released, moved on from here) and tabs between Manage, Edit store page, Releases, Builds
// (Slop Build, Slop Check and signing; not for a browser game, which has nothing to build) and To
// fix (what SLAPorSLOP's checks found). The pages lay their panels out in two columns inside it
// (.maker-cols). The To fix tab counts what must or should be fixed, and Builds is marked while
// it's the next step, so neither is missed (owner decision 2026-11-20).

import { useState, type ReactNode } from "react";
import { Link } from "react-router";

import { api, ApiError, type AppDetail, type AppFixes, type AppStage, type ManageOverview } from "../api";
import { daysUntil, formatDate, playsInBrowser, releaseDateText, STAGE_LABEL } from "../format";
import { useLoad } from "../hooks";
import { AppIcon, appIconProps } from "./AppIcon";
import { InviteNotice } from "./Invite";
import { ListInStore } from "./PublishProgress";
import { StoreIcon } from "./StoreIcons";

export type MakerTab = "manage" | "page" | "releases" | "builds" | "fixes";

/**
 * The To fix tab's count: the newest release's `must` and `should` items, red ("bad") when any
 * must be fixed, amber ("warn") when they only should be, and none without either (suggestions
 * such as "no icon yet" never count). `label` is the tab's name with it. Exported for tests.
 */
export function fixesBadge(
  fixes: Pick<AppFixes, "fixes"> | undefined,
): { count: number; tone: "bad" | "warn"; label: string } | null {
  const list = fixes?.fixes ?? [];
  const must = list.filter((f) => f.severity === "must").length;
  const should = list.filter((f) => f.severity === "should").length;
  if (must + should === 0) return null;
  const said = [must > 0 && `${must} must be fixed`, should > 0 && `${should} should be fixed`].filter(Boolean);
  return { count: must + should, tone: must > 0 ? "bad" : "warn", label: `To fix, ${said.join(", ")}` };
}

/** Apps whose Builds page found Slop Build in the repository, this visit: the tabs stop marking
 *  Builds before its first release arrives. */
const slopBuildFound = new Set<string>();

/** What the Builds page found (Builds.tsx). */
export function rememberSlopBuild(slug: string, found: boolean) {
  if (found) slopBuildFound.add(slug);
  else slopBuildFound.delete(slug);
}

/**
 * Whether Builds is the next step, so its tab is marked: a download on GitHub (where Slop Build
 * runs) with no release yet, and Slop Build not found in its repository. A release from anywhere
 * ends it, so the tabs need only the To fix count's call, never GitHub. Exported for tests.
 */
export function buildsNext(
  app: Pick<AppDetail, "slug" | "repo_url"> & { browser: boolean },
  fixes: Pick<AppFixes, "release"> | undefined,
  slopBuildSetUp?: boolean,
): boolean {
  if (app.browser || !app.repo_url.startsWith("https://github.com/") || !fixes) return false;
  return fixes.release === null && !(slopBuildSetUp ?? slopBuildFound.has(app.slug));
}

const STAGES: AppStage[] = ["coming_soon", "beta", "released"];

/** What each stage means for players, in one line. */
export const STAGE_MEANS: Record<AppStage, string> = {
  coming_soon: "A store page, nothing to play yet. Players can mark it Interested.",
  beta: "Playable and still changing. Players can rate it and donate.",
  released: "Finished, and still updated.",
};

/** The button that moves an app on to `stage`. */
export const MOVE_LABEL: Record<AppStage, string> = {
  coming_soon: "",
  beta: "Move to Beta",
  released: "Release it",
};

/** What moving on to `to` does, said before the maker confirms. Exported for tests. */
export function moveEffects(to: AppStage, interested: number): string[] {
  const told =
    interested > 0
      ? interested === 1
        ? "The 1 person who marked it Interested gets a notice."
        : `The ${interested.toLocaleString()} people who marked it Interested get a notice.`
      : "Anyone who marks it Interested from now on is told when it moves on.";
  const lines =
    to === "beta"
      ? [
          "Its page says Beta: players can play it, rate it (marked “Rated in Beta”) and donate.",
          told,
          "It has a year to be released.",
        ]
      : ["Its page shows it as released: no Beta label or release date, and new votes count as released ones.", told];
  return [...lines, "Only forward: to move it back, you'd ask SLAPorSLOP."];
}

/** What a stage move waits for, and the link to it: for a browser game, a published play address. */
export function advanceHint(next: AppStage[], browser: boolean): { text: string; link: string } {
  const moves = next.map((s) => MOVE_LABEL[s]).join(" or ");
  return browser
    ? { text: `${moves} once you've published a play address.`, link: "Add the play address" }
    : { text: `${moves} once you've published a release.`, link: "Submit a release" };
}

/**
 * What the stage bar says about the release date its maker gave: what players see, or that it has
 * passed and players no longer see it. Null without one, or once released. Exported for tests.
 */
export function releaseDateNote(
  app: Pick<AppDetail, "stage" | "release_date" | "release_date_precision" | "release_date_passed">,
  now: number = Date.now(),
): { text: string; passed: boolean } | null {
  if (app.stage === "released" || !app.release_date) return null;
  if (app.release_date_passed) {
    return { text: "The release date you gave has passed, so players no longer see it: give a new one, or clear it.", passed: true };
  }
  return { text: `Players see “${releaseDateText(app, now)}”.`, passed: false };
}

/** Whether the maker can put a draft in the store from here: a Coming soon page, or once a release
 *  is published (`canAdvance`). Private apps go public through SLAPorSLOP staff. Exported for tests. */
export function canPutInStore(
  app: Pick<AppDetail, "status" | "private" | "stage">,
  published: boolean | undefined,
): boolean {
  return app.status === "draft" && !app.private && (app.stage === "coming_soon" || published === true);
}

export function MakerFrame({
  app,
  tab,
  overview,
  fixes,
  fixesKey,
  slopBuildSetUp,
  onStageMoved,
  children,
}: {
  app: AppDetail;
  tab: MakerTab;
  /** The Manage overview when the page has it already; loaded here otherwise. */
  overview?: ManageOverview;
  /** The fixes when the page has them already (To fix); loaded here otherwise, once. */
  fixes?: AppFixes;
  /** Changes when the newest release may have new fixes: they're loaded again. */
  fixesKey?: string;
  /** Whether Slop Build is in the repository, when the page knows (Builds). */
  slopBuildSetUp?: boolean;
  onStageMoved?: () => void;
  children: ReactNode;
}) {
  const own = useLoad(() => (overview ? Promise.resolve(undefined) : api.manage(app.slug)), [app.slug, Boolean(overview)]);
  const ownFixes = useLoad(() => (fixes ? Promise.resolve(undefined) : api.fixes(app.slug)), [app.slug, Boolean(fixes), fixesKey]);
  const can = overview?.can_advance ?? own.data?.can_advance;
  const found = fixes ?? ownFixes.data;
  const browser = playsInBrowser(app);
  const badge = fixesBadge(found);
  const next = buildsNext({ ...app, browser }, found, slopBuildSetUp);
  const base = `/publish/${app.slug}`;
  // [tab, its name, address, its name with its count or mark, for screen readers]
  const tabs: Array<[MakerTab, string, string, string?]> = [
    ["manage", "Manage", `${base}/manage`],
    ["page", "Edit store page", base],
    ["releases", "Releases", `${base}/releases`],
    ...(browser ? [] : [["builds", "Builds", `${base}/builds`, next ? "Builds, the next step" : undefined] as [MakerTab, string, string, string?]]),
    ["fixes", "To fix", `${base}/fixes`, badge?.label],
  ];
  return (
    <div className="page maker">
      <div className="crumbs">
        <Link to="/publish">Your apps</Link> › {app.title}
      </div>
      <header className="maker-head">
        <AppIcon {...appIconProps(app)} size={56} />
        <div className="maker-name">
          <h1 className="page-title">{app.title}</h1>
          <div className="title-row small">
            <span className={`badge ${app.stage === "beta" ? "beta" : app.stage === "coming_soon" ? "soon" : "official"}`}>
              {STAGE_LABEL[app.stage].toUpperCase()}
            </span>
            {app.private ? (
              <span className="badge neutral">PRIVATE</span>
            ) : app.status && app.status !== "listed" ? (
              <span className="badge neutral">{app.status === "draft" ? "DRAFT: NOT IN THE STORE YET" : "OUT OF THE STORE"}</span>
            ) : (
              <span className="badge official">IN THE STORE</span>
            )}
            {app.plays_in_browser && (
              <span className="badge neutral">
                <StoreIcon name="browser" size={12} />
                PLAYS IN THE BROWSER
              </span>
            )}
            <span className="muted">slop/app/{app.slug}</span>
          </div>
        </div>
        <Link className="btn small" to={`/app/${app.slug}`}>
          View store page
        </Link>
      </header>
      {/* While SLAPorSLOP is in beta, going public may need an invite code: said before Publish. */}
      <InviteNotice />
      <StageBar
        app={app}
        canAdvance={can}
        onReleasesPage={tab === "releases"}
        onMoved={() => {
          own.reload();
          onStageMoved?.();
        }}
      />
      {/* The first listing is the maker's (the Releases page has it in its progress). */}
      {tab === "manage" && canPutInStore(app, can) && (
        <section className="panel panel-pad list-panel" aria-labelledby="list-title">
          <h2 id="list-title">Not in the store yet</h2>
          <p className="small">
            Only you can see this draft. Put it in the store when you're ready for players to find it
            {app.stage === "coming_soon" ? ": a Coming soon page needs no release." : "."}
          </p>
          <ListInStore
            slug={app.slug}
            onListed={() => {
              own.reload();
              onStageMoved?.();
            }}
          />
        </section>
      )}
      <nav className="maker-tabs" aria-label={`${app.title}: maker pages`}>
        {tabs.map(([key, label, to, named]) => (
          <Link
            key={key}
            to={to}
            className={tab === key ? "on" : undefined}
            aria-current={tab === key ? "page" : undefined}
            aria-label={named}
          >
            {label}
            {key === "fixes" && badge && (
              <span className={`badge ${badge.tone} tab-count`} aria-hidden="true">
                {/* Not colour alone: "!" where something must be fixed. */}
                {badge.tone === "bad" && "!"}
                {badge.count}
              </span>
            )}
            {key === "builds" && next && <span className="tab-mark" aria-hidden="true" title="The next step" />}
          </Link>
        ))}
      </nav>
      {children}
    </div>
  );
}

/** Coming soon → Beta → Released: where the app is, its deadline, and the move to the next one. */
export function StageBar({
  app,
  canAdvance,
  onReleasesPage,
  onMoved,
}: {
  app: Pick<
    AppDetail,
    "slug" | "stage" | "stage_entered_at" | "stage_deadline" | "interested" | "release_date" | "release_date_precision" | "release_date_passed"
  > & { plays_in_browser?: boolean };
  /** Undefined while it's being loaded. */
  canAdvance: boolean | undefined;
  onReleasesPage?: boolean;
  onMoved: () => void;
}) {
  const at = STAGES.indexOf(app.stage);
  const [confirm, setConfirm] = useState<AppStage>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const next = STAGES.slice(at + 1);
  const due = app.stage_deadline;
  const late = due ? daysUntil(due) < 0 : false;
  const hint = advanceHint(next, Boolean(app.plays_in_browser));
  const releaseNote = releaseDateNote(app);

  const move = async (to: AppStage) => {
    setBusy(true);
    setError(undefined);
    try {
      await api.moveStage(app.slug, to);
      setConfirm(undefined);
      onMoved();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel stage-bar" aria-labelledby="stage-title">
      <h2 id="stage-title" className="sr-only">
        Stage
      </h2>
      <ol className="stage-steps">
        {STAGES.map((s, i) => {
          const state = i < at ? "done" : i === at ? "now" : "next";
          return (
            <li key={s} className={`stage-step ${state}`} aria-current={state === "now" ? "step" : undefined}>
              <span className="stage-dot" aria-hidden="true">
                {state === "done" ? "✓" : i + 1}
              </span>
              <div>
                <b>{STAGE_LABEL[s]}</b>
                {state === "now" && <span className="muted small"> · now, since {formatDate(app.stage_entered_at)}</span>}
                {state === "done" && <span className="sr-only"> (done)</span>}
                <span className="stage-means muted small">{STAGE_MEANS[s]}</span>
              </div>
            </li>
          );
        })}
      </ol>
      <div className="stage-action">
        {due && (
          <p className={`small ${late ? "bad" : "muted"}`}>
            {late
              ? `Missed its deadline: out of the store until it reaches ${app.stage === "coming_soon" ? "Beta" : "Released"}.`
              : `${app.stage === "coming_soon" ? "Must reach Beta" : "Must be released"} by ${formatDate(due)}.`}
          </p>
        )}
        {releaseNote && (
          <p className={`small ${releaseNote.passed ? "bad" : "muted"}`} role={releaseNote.passed ? "alert" : undefined}>
            {releaseNote.text} <Link to={`/publish/${app.slug}`}>{releaseNote.passed ? "Update the release date" : "Change it"}</Link>
          </p>
        )}
        {next.length === 0 ? (
          <p className="small muted">Released: the last stage.</p>
        ) : canAdvance === false ? (
          <p className="small">
            {hint.text} {!onReleasesPage && <Link to={`/publish/${app.slug}/releases`}>{hint.link}</Link>}
          </p>
        ) : (
          !confirm && (
            <div className="row-actions">
              {next.map((s, i) => (
                <button
                  key={s}
                  type="button"
                  className={`btn small ${i === 0 ? "slime" : ""}`}
                  disabled={canAdvance === undefined}
                  onClick={() => setConfirm(s)}
                >
                  {MOVE_LABEL[s]}…
                </button>
              ))}
            </div>
          )
        )}
      </div>
      {confirm && (
        <div className="stage-confirm" role="group" aria-label={MOVE_LABEL[confirm]}>
          <b>{confirm === "beta" ? "Move it to Beta?" : "Release it?"}</b>
          <ul className="small">
            {moveEffects(confirm, app.interested).map((l) => (
              <li key={l}>{l}</li>
            ))}
          </ul>
          <div className="row-actions">
            <button type="button" className="btn small slime" disabled={busy} onClick={() => void move(confirm)}>
              {busy ? "Moving…" : MOVE_LABEL[confirm]}
            </button>
            <button type="button" className="btn small ghost" disabled={busy} onClick={() => setConfirm(undefined)}>
              Not yet
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
