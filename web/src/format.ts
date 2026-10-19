// Display helpers shared by the pages.

import type { AppStage, AppSummary } from "./api";

export const STAGE_LABEL: Record<AppStage, string> = {
  coming_soon: "Coming soon",
  beta: "Beta",
  released: "Released",
};

const DAY = 86_400_000;

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
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
