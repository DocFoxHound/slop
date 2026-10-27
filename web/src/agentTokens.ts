// SPDX-License-Identifier: AGPL-3.0-or-later
// AI agent tokens' lifetimes and expiry (crates/slop-api/src/maker_agents.rs): the choices on
// Account → AI agents, the countdown each token shows, which ones need replacing, and the
// account section's badge. The server warns the owner a week and a day before, and on expiry.
// Staff tokens (Admin → AI reviewers, review.rs) use the same choices, countdown and flags.

import type { AgentToken } from "./api";

const DAY = 86_400_000;

/** What expiry needs to know about a token: a maker's (`AgentToken`) or a staff member's. */
export type ExpiringToken = Pick<AgentToken, "id" | "name" | "created_at" | "expires_at">;

/** How long a token can last (the server allows 1 to 365 days). */
export const LIFETIMES: { days: number; label: string }[] = [
  { days: 1, label: "1 day" },
  { days: 7, label: "7 days" },
  { days: 30, label: "30 days" },
  { days: 90, label: "3 months" },
  { days: 180, label: "6 months" },
  { days: 365, label: "12 months" },
];

/** A token is highlighted on the page this many days before it expires. */
export const SOON_DAYS = 14;
/** The account section's badge shows this many days before. */
export const BADGE_DAYS = 7;

/** Whole days left, rounded up; 0 once it has expired. */
export function daysLeft(expiresAt: string, now: number = Date.now()): number {
  const ms = new Date(expiresAt).getTime() - now;
  return ms <= 0 ? 0 : Math.ceil(ms / DAY);
}

export function isExpired(t: Pick<AgentToken, "expires_at">, now: number = Date.now()): boolean {
  return new Date(t.expires_at).getTime() <= now;
}

/** "expires in 12 days", "expires within a day", "expired". */
export function expiryText(expiresAt: string, now: number = Date.now()): string {
  const n = daysLeft(expiresAt, now);
  if (n === 0) return "expired";
  if (n === 1) return "expires within a day";
  return `expires in ${n} days`;
}

/**
 * The token that replaces this one: the same name, made after it, lasting longer, still working.
 * (The server doesn't warn about a replaced token either.)
 */
export function replacementOf<T extends ExpiringToken>(t: T, all: T[], now: number = Date.now()): T | undefined {
  return all.find(
    (o) =>
      o.id !== t.id &&
      o.name === t.name &&
      new Date(o.created_at).getTime() > new Date(t.created_at).getTime() &&
      new Date(o.expires_at).getTime() > new Date(t.expires_at).getTime() &&
      !isExpired(o, now),
  );
}

/**
 * Made for a week or less: meant to be short, so it isn't flagged before it expires (and SLAPorSLOP
 * sends no warnings for it).
 */
export function isShortLived(t: Pick<AgentToken, "created_at" | "expires_at">): boolean {
  return new Date(t.expires_at).getTime() - new Date(t.created_at).getTime() <= 8 * DAY;
}

/** Expired, or (unless short-lived) expiring within `days`, and not replaced yet. */
export function needsReplacing<T extends ExpiringToken>(t: T, all: T[], days: number, now: number = Date.now()): boolean {
  if (replacementOf(t, all, now)) return false;
  if (isExpired(t, now)) return true;
  return !isShortLived(t) && daysLeft(t.expires_at, now) <= days;
}

/** The lifetime choice nearest to how long a token was made for (to make its replacement). */
export function lifetimeOf(t: Pick<AgentToken, "created_at" | "expires_at">): number {
  const days = (new Date(t.expires_at).getTime() - new Date(t.created_at).getTime()) / DAY;
  return LIFETIMES.reduce((best, l) => (Math.abs(l.days - days) < Math.abs(best - days) ? l.days : best), LIFETIMES[0].days);
}

export interface TokenBadge {
  text: string;
  label: string;
  tone: "warn" | "bad";
}

/**
 * The account section's badge: a token expired, or expiring within a week, with no replacement.
 * Short-lived tokens never count.
 */
export function tokenBadge(tokens: AgentToken[], now: number = Date.now()): TokenBadge | undefined {
  const due = tokens.filter((t) => !isShortLived(t) && needsReplacing(t, tokens, BADGE_DAYS, now));
  if (due.length === 0) return undefined;
  const expired = due.filter((t) => isExpired(t, now)).length;
  if (expired > 0) {
    return { text: "!", label: expired === 1 ? "an AI agent token has expired" : `${expired} AI agent tokens have expired`, tone: "bad" };
  }
  return { text: "!", label: due.length === 1 ? "an AI agent token expires soon" : `${due.length} AI agent tokens expire soon`, tone: "warn" };
}
