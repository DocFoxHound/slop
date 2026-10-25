// SPDX-License-Identifier: AGPL-3.0-or-later
// Admin → Apps in words: what each flag the API sets means for staff (crates/slop-api/src/admin_web.rs,
// FLAGS), the quiet line a healthy app gets, why an app can't go in the store yet, and the list's
// filters as they are in the address. Pure, so it's tested on its own (appFacts.test.ts).

import type { AdminApp, AdminAppFlag, AdminAppsQuery, AppStage, AppStatus } from "../../api";
import { daysSince, daysUntil, timeAgo } from "../../format";

/** One fact on an app's line: a problem (bad), something to keep an eye on (warn), or neither. */
export interface Fact {
  text: string;
  tone: "bad" | "warn" | "quiet";
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** "3 days", "5 months", "2 years": how long, for "no commits for …". */
export function span(days: number): string {
  if (days < 60) return plural(days, "day");
  if (days < 730) return plural(Math.floor(days / 30), "month");
  return plural(Math.floor(days / 365), "year");
}

export const STAGE_LABEL: Record<AppStage, string> = {
  coming_soon: "Coming soon",
  beta: "Beta",
  released: "Released",
};

/** A flag in words, with what's behind it. */
export function flagFact(app: AdminApp, flag: AdminAppFlag, now: number = Date.now()): Fact {
  switch (flag) {
    case "rescan_flagged":
      return { tone: "bad", text: `Re-scan found something in ${app.rescan_flags === 1 ? "a live file" : `${app.rescan_flags} live files`}` };
    case "download_lost":
      return {
        tone: "bad",
        text: `${app.downloads_lost === 1 ? "Its download" : `${app.downloads_lost} downloads`} changed or can't be reached`,
      };
    case "frozen":
      return { tone: "bad", text: `Frozen${app.frozen_reason ? `: ${app.frozen_reason}` : ""}` };
    case "reports":
      return { tone: "bad", text: `${plural(app.open_reports, "open report")}` };
    case "release_failed": {
      const r = app.newest_release;
      return { tone: "bad", text: `${r?.version ?? "Newest release"} failed${r?.status_reason ? `: ${r.status_reason}` : ""}` };
    }
    case "release_waiting":
      return {
        tone: "warn",
        text: `${app.pending_releases === 1 ? "A release" : `${app.pending_releases} releases`} waiting for review`,
      };
    case "past_deadline": {
      const days = app.stage_deadline ? -daysUntil(app.stage_deadline, now) : 0;
      return {
        tone: "bad",
        text: `Hidden from the store: its ${STAGE_LABEL[app.stage]} time ran out${days > 0 ? ` ${span(days)} ago` : ""}`,
      };
    }
    case "deadline_soon": {
      const days = app.stage_deadline ? Math.max(0, daysUntil(app.stage_deadline, now)) : 0;
      return { tone: "warn", text: `${STAGE_LABEL[app.stage]} time runs out in ${plural(days, "day")}` };
    }
    case "donations_paused":
      return { tone: "warn", text: "Donations paused" };
    case "no_commits":
      return {
        tone: "warn",
        text: app.last_commit_at ? `Dead? No commits for ${span(daysSince(app.last_commit_at, now))}` : "Dead? No commits",
      };
    case "no_players":
      return {
        tone: "warn",
        text: app.last_played_at
          ? `Dead? Nobody has played it for ${span(daysSince(app.last_played_at, now))}`
          : "Dead? Nobody has played it yet",
      };
    case "low_rating":
      return { tone: "warn", text: `Rated ${app.not_slop_percent ?? 0}% Not Slop (${plural(app.vote_count, "vote")})` };
  }
}

/** Why it can't go in the store yet, or null when it can (or already is). */
export function cantListReason(app: AdminApp): string | null {
  if (app.status === "listed" || app.stage === "coming_soon" || app.releases.length > 0) return null;
  return app.pending_releases > 0
    ? "Can't list yet: approve its release under Review queue → New releases first."
    : "Can't list yet: the maker hasn't submitted a release. Set the stage to Coming soon to list the page without one.";
}

/** The line a healthy app gets: what players get and how it's doing. */
export function quietLine(app: AdminApp, now: number = Date.now()): string {
  const parts: string[] = [];
  if (app.serving_version) parts.push(`Players get ${app.serving_version}`);
  else if (app.stage === "coming_soon") parts.push("No release yet");
  else parts.push(app.newest_release ? `${app.newest_release.version} ${app.newest_release.status.replace(/_/g, " ")}` : "No release yet");
  if (app.players > 0) parts.push(plural(app.players, "player"));
  if (app.last_played_at) parts.push(`played ${timeAgo(app.last_played_at, now)}`);
  if (app.not_slop_percent !== null) parts.push(`${app.not_slop_percent}% Not Slop`);
  return parts.join(" · ");
}

/** The facts an app's line shows: its first two flags, or the quiet line; and how many more. */
export function rowFacts(app: AdminApp, now: number = Date.now()): { shown: Fact[]; more: Fact[] } {
  if (app.flags.length === 0) return { shown: [{ tone: "quiet", text: quietLine(app, now) }], more: [] };
  const facts = app.flags.map((f) => flagFact(app, f, now));
  return { shown: facts.slice(0, 2), more: facts.slice(2) };
}

// ---------------------------------------------------------------------------------------------
// The filters, as they are in the address (/admin?tab=apps&show=attention&sort=title&q=…)

export type Show = "" | "attention" | AppStatus;
export type Sort = "updated" | "urgent" | "title";

export const SHOWS: Array<[Show, string]> = [
  ["", "All apps"],
  ["attention", "Needs attention"],
  ["draft", "Draft"],
  ["listed", "Listed"],
  ["delisted", "Delisted"],
];

export const SORTS: Array<[Sort, string]> = [
  ["updated", "Recently changed"],
  ["urgent", "Most urgent first"],
  ["title", "Title"],
];

export function readShow(value: string | null): Show {
  return SHOWS.some(([s]) => s === value) ? (value as Show) : "";
}

/** The chosen order; without one, most urgent first when only apps needing attention are shown. */
export function readSort(value: string | null, show: Show): Sort {
  if (SORTS.some(([s]) => s === value)) return value as Sort;
  return show === "attention" ? "urgent" : "updated";
}

/** How many apps one page of the list holds. */
export const PAGE = 50;

/** The API query for page `page` (from 0) of the list. */
export function pageQuery(show: Show, sort: Sort, q: string, page: number): AdminAppsQuery {
  return {
    status: show === "" || show === "attention" ? undefined : show,
    attention: show === "attention",
    sort,
    q: q || undefined,
    limit: PAGE,
    offset: page * PAGE,
  };
}
