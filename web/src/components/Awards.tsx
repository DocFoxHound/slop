// SPDX-License-Identifier: AGPL-3.0-or-later
// App of the Month badges (crates/slop-api/src/awards.rs): on an app's store page, one per month
// it won with the published award, and on its maker's profile with a count. Each links to the
// awards page. The trophy is decoration: the words say what it is.

import { Link } from "react-router";

import type { AppAward, AwardCurrency } from "../api";

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/** "2026-11" → "November 2026". */
export function monthLabel(month: string): string {
  const [year, m] = month.split("-");
  const name = MONTHS[Number(m) - 1];
  return name ? `${name} ${year}` : month;
}

/** An award as published: "$25", or "$12.50" when it has cents. */
export function awardAmount(minor: number, currency: AwardCurrency | string): string {
  const symbol = currency === "eur" ? "€" : currency === "gbp" ? "£" : "$";
  return minor % 100 === 0 ? `${symbol}${minor / 100}` : `${symbol}${(minor / 100).toFixed(2)}`;
}

/** "App of the Month · November 2026 · $25 award". */
export function awardText(a: Pick<AppAward, "month" | "amount_minor" | "currency">): string {
  return `App of the Month · ${monthLabel(a.month)} · ${awardAmount(a.amount_minor, a.currency)} award`;
}

/** Shown on the store page at most; the rest are on the awards page. */
const SHOWN = 3;

/** The store page's badges, one per month it won (newest first). */
export function AwardBadges({ awards }: { awards: AppAward[] | undefined }) {
  if (!awards || awards.length === 0) return null;
  const more = awards.length - SHOWN;
  return (
    <>
      {awards.slice(0, SHOWN).map((a) => (
        <Link key={a.month} to="/awards" className="badge award" title={a.blurb || undefined}>
          <span aria-hidden="true">🏆</span> {awardText(a)}
        </Link>
      ))}
      {more > 0 && (
        <Link to="/awards" className="badge award">
          +{more} more {more === 1 ? "month" : "months"}
        </Link>
      )}
    </>
  );
}

/** A maker's profile badge: "App of the Month", with ×N when they've won more than once. */
export function MakerAwardBadge({ count }: { count: number }) {
  if (count < 1) return null;
  return (
    <Link to="/awards" className="badge award">
      <span aria-hidden="true">🏆</span> App of the Month
      {count > 1 && (
        <>
          <span aria-hidden="true"> ×{count}</span>
          <span className="sr-only">, {count} times</span>
        </>
      )}
    </Link>
  );
}
