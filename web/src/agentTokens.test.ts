// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { AgentToken } from "./api";
import { LIFETIMES, daysLeft, expiryText, lifetimeOf, needsReplacing, replacementOf, tokenBadge } from "./agentTokens";

const NOW = Date.parse("2026-10-25T12:00:00Z");
const DAY = 86_400_000;
const at = (days: number) => new Date(NOW + days * DAY).toISOString();

function token(name: string, madeDaysAgo: number, expiresInDays: number, id = name): AgentToken {
  return {
    id,
    name,
    scope: "publish",
    apps: [],
    people: false,
    created_at: at(-madeDaysAgo),
    expires_at: at(expiresInDays),
    last_used_at: at(-1),
    expired: expiresInDays <= 0,
  };
}

describe("AI agent token expiry", () => {
  it("offers a day up to 12 months", () => {
    expect(LIFETIMES.map((l) => l.days)).toEqual([1, 7, 30, 90, 180, 365]);
    expect(LIFETIMES.at(-1)?.label).toBe("12 months");
  });

  it("counts whole days left, rounding up", () => {
    expect(daysLeft(at(12), NOW)).toBe(12);
    expect(daysLeft(at(11.2), NOW)).toBe(12);
    expect(daysLeft(at(0.1), NOW)).toBe(1);
    expect(daysLeft(at(0), NOW)).toBe(0);
    expect(daysLeft(at(-3), NOW)).toBe(0);
  });

  it("says when it expires", () => {
    expect(expiryText(at(12), NOW)).toBe("expires in 12 days");
    expect(expiryText(at(1.5), NOW)).toBe("expires in 2 days");
    expect(expiryText(at(0.5), NOW)).toBe("expires within a day");
    expect(expiryText(at(-0.5), NOW)).toBe("expired");
  });

  it("finds the lifetime a token was made with, for its replacement", () => {
    expect(lifetimeOf(token("a", 300, 65))).toBe(365);
    expect(lifetimeOf(token("a", 80, 10))).toBe(90);
    expect(lifetimeOf(token("a", 29, 1))).toBe(30);
    expect(lifetimeOf(token("a", 0, 7))).toBe(7);
    expect(lifetimeOf(token("a", 1, 0))).toBe(1);
  });

  it("a newer, longer-lasting token with the same name replaces it", () => {
    const old = token("Laptop", 85, 5, "old");
    const fresh = token("Laptop", 0, 90, "new");
    const other = token("Desktop", 0, 90, "other");
    expect(replacementOf(old, [old, fresh, other], NOW)?.id).toBe("new");
    expect(replacementOf(fresh, [old, fresh, other], NOW)).toBeUndefined();
    expect(replacementOf(old, [old, other], NOW)).toBeUndefined();
    // An expired "replacement" doesn't count.
    const dead = token("Laptop", 1, -0.1, "dead");
    expect(replacementOf(old, [old, { ...dead, expires_at: at(-0.1), created_at: at(-0.5) }], NOW)).toBeUndefined();
    expect(needsReplacing(old, [old], 14, NOW)).toBe(true);
    expect(needsReplacing(old, [old, fresh], 14, NOW)).toBe(false);
    expect(needsReplacing(token("Later", 10, 20), [], 14, NOW)).toBe(false);
    // Made for a week or less: only once it has expired.
    expect(needsReplacing(token("Week", 1, 6), [], 14, NOW)).toBe(false);
    expect(needsReplacing(token("Week", 8, -1), [], 14, NOW)).toBe(true);
  });

  it("the account badge shows a token expiring within a week, or expired", () => {
    expect(tokenBadge([], NOW)).toBeUndefined();
    expect(tokenBadge([token("a", 300, 8)], NOW)).toBeUndefined();
    expect(tokenBadge([token("a", 300, 7)], NOW)).toEqual({ text: "!", label: "an AI agent token expires soon", tone: "warn" });
    expect(tokenBadge([token("a", 300, 3), token("b", 300, 1)], NOW)?.label).toBe("2 AI agent tokens expire soon");
    expect(tokenBadge([token("a", 300, 3), token("b", 300, -1)], NOW)).toEqual({
      text: "!",
      label: "an AI agent token has expired",
      tone: "bad",
    });
    // Short-lived tokens never count, even expired.
    expect(tokenBadge([token("day", 0.5, 0.5), token("week", 8, -1)], NOW)).toBeUndefined();
    // Replaced: nothing to do.
    expect(tokenBadge([token("a", 300, 3, "old"), token("a", 0, 90, "new")], NOW)).toBeUndefined();
  });
});
