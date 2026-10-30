// SPDX-License-Identifier: AGPL-3.0-or-later
// The maker's workspace for one app: its name and where it stands, the stage bar (Coming soon →
// Beta → Released, moved on from here) and tabs between Manage, Edit store page and Releases. The
// three pages lay their panels out in two columns inside it (.maker-cols).

import { useState, type ReactNode } from "react";
import { Link } from "react-router";

import { api, ApiError, type AppDetail, type AppStage, type ManageOverview } from "../api";
import { daysUntil, formatDate, STAGE_LABEL } from "../format";
import { useLoad } from "../hooks";
import { AppIcon, appIconProps } from "./AppIcon";

export type MakerTab = "manage" | "page" | "releases";

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
      : ["Its page shows it as released: no Beta label, and new votes count as released ones.", told];
  return [...lines, "Only forward: to move it back, you'd ask SLAPorSLOP."];
}

export function MakerFrame({
  app,
  tab,
  overview,
  onStageMoved,
  children,
}: {
  app: AppDetail;
  tab: MakerTab;
  /** The Manage overview when the page has it already; loaded here otherwise. */
  overview?: ManageOverview;
  onStageMoved?: () => void;
  children: ReactNode;
}) {
  const own = useLoad(() => (overview ? Promise.resolve(undefined) : api.manage(app.slug)), [app.slug, Boolean(overview)]);
  const can = overview?.can_advance ?? own.data?.can_advance;
  const base = `/publish/${app.slug}`;
  const tabs: Array<[MakerTab, string, string]> = [
    ["manage", "Manage", `${base}/manage`],
    ["page", "Edit store page", base],
    ["releases", "Releases", `${base}/releases`],
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
            <span className="muted">slop/app/{app.slug}</span>
          </div>
        </div>
        <Link className="btn small" to={`/app/${app.slug}`}>
          View store page
        </Link>
      </header>
      <StageBar
        app={app}
        canAdvance={can}
        onReleasesPage={tab === "releases"}
        onMoved={() => {
          own.reload();
          onStageMoved?.();
        }}
      />
      <nav className="maker-tabs" aria-label={`${app.title}: maker pages`}>
        {tabs.map(([key, label, to]) => (
          <Link key={key} to={to} className={tab === key ? "on" : undefined} aria-current={tab === key ? "page" : undefined}>
            {label}
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
  app: Pick<AppDetail, "slug" | "stage" | "stage_entered_at" | "stage_deadline" | "interested">;
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
        {next.length === 0 ? (
          <p className="small muted">Released: the last stage.</p>
        ) : canAdvance === false ? (
          <p className="small">
            {next.map((s) => MOVE_LABEL[s]).join(" or ")} once a release is approved.{" "}
            {!onReleasesPage && <Link to={`/publish/${app.slug}/releases`}>Submit a release</Link>}
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
