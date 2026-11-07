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

  if (checks.public_host === true) items.push({ key: "public", label: LABEL.public, state: "done", status: "Yes" });
  else if (checks.public_host === null) {
    items.push({
      key: "public",
      label: LABEL.public,
      state: "skipped",
      status: "Not checked",
      detail: "A test setup of SLAPorSLOP doesn't look up where a site is.",
    });
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

/** A reviewer's question: is this address on a different site from the one Play opens now? */
export function isNewSite(address: string | null | undefined, current: string | null | undefined): boolean {
  if (!address || !current) return false;
  return addressHost(address) !== addressHost(current);
}
