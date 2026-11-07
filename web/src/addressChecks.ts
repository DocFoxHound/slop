// SPDX-License-Identifier: AGPL-3.0-or-later
// A browser game's play address, as SLAPorSLOP's automatic checks found it (crates/slop-api/src/
// releases.rs `check_play_address`), in words: one line per check for the maker's progress and
// Releases history and for the reviewer's card. Each line says its result in text, so it never
// rests on an icon or a colour. Nothing here says anything about what the site serves: the checks
// are about the address only. Kept free of React so it can be tested on its own.

import type { AddressChecks, Release } from "./api";
import { addressHost } from "./format";

export type CheckState = "done" | "active" | "problem" | "skipped";

export interface AddressCheckItem {
  key: "https" | "public" | "bad_links" | "page";
  /** What was checked: "https", "A public site"… */
  label: string;
  state: CheckState;
  /** What it found, in words. */
  status: string;
  detail?: string;
}

const LABEL: Record<AddressCheckItem["key"], string> = {
  https: "https",
  public: "A public site",
  bad_links: "Bad-link lists",
  page: "The page loads",
};

/** Said wherever this server has no bad-link list to compare with: never "not on any list". */
export const NO_LISTS = "No bad-link lists are loaded on this server, so it wasn't compared with any.";

const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

/**
 * One line per check. While the address is still being checked (`fetching`, or no result yet),
 * every line is under way. A check that failed takes the problem the server named; one that
 * passes only on a test setup (plain http, a local page) says so instead of claiming it passed.
 */
export function addressCheckItems(checks: AddressChecks | null | undefined, status?: Release["status"]): AddressCheckItem[] {
  const keys: AddressCheckItem["key"][] = ["https", "public", "bad_links", "page"];
  if (!checks || status === "fetching") {
    return keys.map((key) => ({ key, label: LABEL[key], state: "active", status: "Checking" }));
  }
  const items: AddressCheckItem[] = [];

  if (checks.https) items.push({ key: "https", label: LABEL.https, state: "done", status: "Yes" });
  else if (checks.ok) {
    items.push({
      key: "https",
      label: LABEL.https,
      state: "skipped",
      status: "No: plain http",
      detail: "Only a test setup of SLAPorSLOP allows that.",
    });
  } else items.push({ key: "https", label: LABEL.https, state: "problem", status: "No" });

  // Null says nothing by itself: the server says why (`public_host_skipped`). Older checks don't;
  // one that passed then was a test setup's, since a name that can't be looked up fails.
  const skipped = checks.public_host_skipped ?? (checks.ok ? "test_setup" : null);
  if (checks.public_host === true) items.push({ key: "public", label: LABEL.public, state: "done", status: "Yes" });
  else if (checks.public_host === null && skipped === "lookup_failed") {
    items.push({ key: "public", label: LABEL.public, state: "problem", status: "No: the site's name couldn't be looked up" });
  } else if (checks.public_host === null && skipped === "test_setup") {
    items.push({
      key: "public",
      label: LABEL.public,
      state: "skipped",
      status: "Not checked",
      detail: "A test setup of SLAPorSLOP doesn't look up where a site is.",
    });
  } else if (checks.public_host === null) {
    items.push({ key: "public", label: LABEL.public, state: "skipped", status: "Not known" });
  } else items.push({ key: "public", label: LABEL.public, state: "problem", status: "No: it's on a private network" });

  const lists = checks.bad_links;
  if (lists.listed_by) {
    items.push({ key: "bad_links", label: LABEL.bad_links, state: "problem", status: `On ${lists.listed_by}` });
  } else if (!lists.lists_loaded) {
    items.push({ key: "bad_links", label: LABEL.bad_links, state: "skipped", status: "Not checked", detail: NO_LISTS });
  } else items.push({ key: "bad_links", label: LABEL.bad_links, state: "done", status: "Not on them" });

  const loaded = checks.status !== null && checks.status >= 200 && checks.status < 300;
  const where = [
    checks.redirects > 0 && checks.final_address ? `after ${plural(checks.redirects, "redirect")}, at ${checks.final_address}` : "",
    checks.content_type ? `a ${checks.content_type.split(";")[0].trim()} page` : "",
  ]
    .filter(Boolean)
    .join("; ");
  // A failure the lines above don't show (the wrong kind of page, a redirect to SLAPorSLOP's own
  // site) goes on this line, so the reason the server gave is always there.
  const anyProblem = items.some((i) => i.state === "problem");
  if (checks.ok || (loaded && anyProblem)) {
    items.push({
      key: "page",
      label: LABEL.page,
      state: "done",
      status: `Yes (HTTP ${checks.status})`,
      detail: where ? where.charAt(0).toUpperCase() + where.slice(1) : undefined,
    });
  } else {
    items.push({
      key: "page",
      label: LABEL.page,
      state: "problem",
      status: checks.status === null ? "No answer" : loaded ? "No" : `No (HTTP ${checks.status})`,
      detail: checks.problem ?? undefined,
    });
  }
  return items;
}

/** "✓ https: Yes · – A public site: Not checked …": the checks in one line of plain text. */
export function addressChecksLine(items: AddressCheckItem[]): string {
  return items.map((i) => `${CHECK_MARK[i.state]} ${i.label}: ${i.status}`).join(" · ");
}

/** Marks beside each line (always with its words: the mark alone means nothing). */
export const CHECK_MARK: Record<CheckState, string> = { done: "✓", active: "…", problem: "!", skipped: "–" };

/**
 * How an address compares with another (where Play goes now, or the one SLAPorSLOP withdrew), for
 * a reviewer: the same address; another address on the same host, which on a host many people
 * share (itch.zone, a CDN, a page builder) can be someone else's game, so it's never called the
 * same site; or a new site. Null when there's nothing to compare. Hosts compare as the server
 * stores them, with any dot at the end dropped; addresses compare whole, query and fragment too
 * (releases.rs `same_address`).
 */
export type AddressComparison = "same_address" | "same_host" | "new_site";

export function compareAddress(address: string | null | undefined, other: string | null | undefined): AddressComparison | null {
  const a = parsed(address);
  const b = parsed(other);
  if (!a || !b) return null;
  if (a.host !== b.host) return "new_site";
  return a.href === b.href ? "same_address" : "same_host";
}

function parsed(address: string | null | undefined): { host: string; href: string } | null {
  if (!address) return null;
  try {
    const url = new URL(address);
    url.hostname = url.hostname.replace(/\.+$/, "");
    return { host: url.host || addressHost(address), href: url.href };
  } catch {
    return null;
  }
}
