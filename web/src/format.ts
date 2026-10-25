// SPDX-License-Identifier: AGPL-3.0-or-later
// Display helpers shared by the pages.

import type { AppStage, AppSummary, ContentLabel, OnlineKind } from "./api";

export const STAGE_LABEL: Record<AppStage, string> = {
  coming_soon: "Coming soon",
  beta: "Beta",
  released: "Released",
};

const DAY = 86_400_000;

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

/** The full date and time, for a tooltip beside a relative time. */
export function formatDateTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, { dateStyle: "full", timeStyle: "short" });
}

/** "just now", "5 min ago", "3 h ago", "2 days ago", then the date ("Oct 3", with the year when it isn't this year's). */
export function timeAgo(iso: string, now: number = Date.now()): string {
  const then = new Date(iso);
  const minutes = Math.floor((now - then.getTime()) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days} day${days === 1 ? "" : "s"} ago`;
  const sameYear = then.getFullYear() === new Date(now).getFullYear();
  return then.toLocaleDateString(undefined, sameYear ? { month: "short", day: "numeric" } : { month: "short", day: "numeric", year: "numeric" });
}

export type DayGroup = "Today" | "Yesterday" | "This week" | "Earlier";
export const DAY_GROUPS: DayGroup[] = ["Today", "Yesterday", "This week", "Earlier"];

/** Which of the inbox's day groups a time falls in, by the viewer's own calendar days. */
export function dayGroup(iso: string, now: number = Date.now()): DayGroup {
  const midnight = (t: number) => {
    const d = new Date(t);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  };
  // Calendar days, so a day with a clock change still counts as one.
  const days = Math.round((midnight(now) - midnight(new Date(iso).getTime())) / DAY);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return "This week";
  return "Earlier";
}

/**
 * An account's ID from what staff paste: the ID itself, or a link to the profile (…/u/<id>).
 * Undefined when it's neither, so the form can say so instead of sending it.
 */
export function accountId(text: string): string | undefined {
  const id = text
    .trim()
    .replace(/^.*\/u\//, "")
    .replace(/[/?#].*$/, "");
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id) ? id.toLowerCase() : undefined;
}

export function daysUntil(iso: string, now: number = Date.now()): number {
  return Math.ceil((new Date(iso).getTime() - now) / DAY);
}

export function daysSince(iso: string, now: number = Date.now()): number {
  return Math.floor((now - new Date(iso).getTime()) / DAY);
}

/** "Beta due in 12 days", "Full release due by Mar 3, 2027", or null once released. */
export function deadlineText(app: Pick<AppSummary, "stage" | "stage_deadline">, now: number = Date.now()): string | null {
  if (!app.stage_deadline) return null;
  if (app.stage === "coming_soon") {
    const days = daysUntil(app.stage_deadline, now);
    if (days < 0) return "Missed its beta deadline";
    return `Beta due in ${days} day${days === 1 ? "" : "s"}`;
  }
  if (daysUntil(app.stage_deadline, now) < 0) return "Missed its release deadline";
  return `Full release due by ${formatDate(app.stage_deadline)}`;
}

/** A stable two-colour gradient and initial for apps that have no artwork yet. */
export function artFor(slug: string): { from: string; to: string } {
  let hash = 0;
  for (const ch of slug) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  const hue = hash % 360;
  return { from: `hsl(${hue} 55% 32%)`, to: `hsl(${(hue + 50) % 360} 60% 18%)` };
}

export function ageLabel(minAge: number): string | null {
  return minAge > 0 ? `${minAge}+` : null;
}

const SYMBOL: Record<string, string> = { usd: "$", eur: "€", gbp: "£" };

/** "$120", "$120.01", "£12,345" — the same as the server's job texts. */
export function money(minor: number, currency: string): string {
  const whole = Math.trunc(minor / 100).toLocaleString("en-US");
  const cents = Math.abs(minor % 100);
  return `${SYMBOL[currency] ?? "$"}${whole}${cents ? `.${String(cents).padStart(2, "0")}` : ""}`;
}

/** "120.5" → 12050; null when it isn't an amount. */
export function toMinor(text: string): number | null {
  const t = text.trim().replace(/[$€£,\s]/g, "");
  if (!/^\d+(\.\d{1,2})?$/.test(t)) return null;
  const [whole, frac = ""] = t.split(".");
  return Number(whole) * 100 + Number(frac.padEnd(2, "0"));
}

/** What an app connects to online, as players read it. */
export const ONLINE_LABEL: Record<OnlineKind, string> = {
  own_servers: "Connects to the maker's servers or database",
  player_servers: "Players can host their own servers",
  peer_to_peer: "Players connect directly to each other",
  other_services: "Uses other online services (cloud databases, AI, APIs)",
};

/** Content labels (DESIGN §5), as players read them. */
export const CONTENT_LABEL: Record<ContentLabel, string> = {
  romance: "Romance",
  suggestive: "Suggestive themes",
  violence: "Violence",
  horror: "Horror",
  strong_language: "Strong language",
  gambling_themes: "Gambling themes",
};
