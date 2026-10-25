// SPDX-License-Identifier: AGPL-3.0-or-later
// The account page's sections (pages/Account.tsx): one at a time, each at its own address
// (/account for Profile, /account/<section> for the rest), so they can be linked and reloaded
// and Back works. Links from before the sections, /account#<part> (from notifications, emails
// and the server's redirects), still work: each part belongs to one section.

export type AccountSectionId =
  | "profile"
  | "security"
  | "device"
  | "agents"
  | "notifications"
  | "money"
  | "moderation"
  | "agreements"
  | "data";

export interface AccountSection {
  id: AccountSectionId;
  /** In the section list. */
  label: string;
  /** The section's heading, where it differs from the label. */
  title?: string;
  /** One short line under the heading. */
  intro: string;
  group: "Account" | "Publishing & money" | "Records";
  /** The parts (element ids) in it: /account#<part> opens this section there. */
  parts: string[];
  /** Only in the desktop app. */
  desktopOnly?: boolean;
}

export const ACCOUNT_SECTIONS: AccountSection[] = [
  {
    id: "profile",
    label: "Profile",
    intro: "How you appear on Slop, and what other people can see.",
    group: "Account",
    parts: ["profile", "details", "picture", "people-page", "privacy"],
  },
  {
    id: "security",
    label: "Sign-in & security",
    intro: "The accounts you sign in with, your passkeys, and everywhere you're signed in now.",
    group: "Account",
    parts: ["security", "sign-ins", "passkeys", "sessions"],
  },
  {
    id: "notifications",
    label: "Notifications",
    intro: "What reaches you besides the 🔔: email, and notifications on this phone or browser.",
    group: "Account",
    parts: ["notifications"],
  },
  {
    id: "device",
    label: "This computer",
    intro: "Settings for the Slop app on this computer only.",
    group: "Account",
    parts: ["this-device", "device"],
    desktopOnly: true,
  },
  {
    id: "agents",
    label: "AI agents & keys",
    intro: "Let your own AI coding agent publish for you, and sign the builds you make yourself.",
    group: "Publishing & money",
    parts: ["agents", "ai-agents", "signing-keys"],
  },
  {
    id: "money",
    label: "Money",
    intro: "Donations you give and receive, and Slop's fee statements.",
    group: "Publishing & money",
    parts: ["money", "donations", "receiving", "fee-statements"],
  },
  {
    id: "moderation",
    label: "Moderation",
    intro: "Decisions about your content or account, with how to appeal, and the reports you've sent.",
    group: "Records",
    parts: ["moderation", "decisions", "reports"],
  },
  {
    id: "agreements",
    label: "Agreements",
    title: "Terms and policies",
    intro: "What you've agreed to, and changes coming up.",
    group: "Records",
    parts: ["agreements"],
  },
  {
    id: "data",
    label: "Your data",
    intro: "Download everything linked to your account, or close it.",
    group: "Records",
    parts: ["data"],
  },
];

/** The sections shown here: "This computer" only in the desktop app. */
export function accountSections(desktop: boolean): AccountSection[] {
  return ACCOUNT_SECTIONS.filter((s) => desktop || !s.desktopOnly);
}

/** A section's address: Profile is /account itself. */
export function accountPath(id: AccountSectionId): string {
  return id === "profile" ? "/account" : `/account/${id}`;
}

/** The section at /account/<slug> (none for "/account"): undefined if there's no such section here. */
export function sectionFromSlug(slug: string | undefined, desktop: boolean): AccountSection | undefined {
  const list = accountSections(desktop);
  if (!slug) return list[0];
  return list.find((s) => s.id === slug);
}

/** The section an old /account#<part> link means, or undefined. */
export function sectionForPart(hash: string, desktop: boolean): AccountSection | undefined {
  let part = hash.startsWith("#") ? hash.slice(1) : hash;
  try {
    part = decodeURIComponent(part);
  } catch {
    // Not encoded as expected: used as it is.
  }
  if (!part) return undefined;
  return accountSections(desktop).find((s) => s.parts.includes(part));
}

/**
 * Where an /account address really goes: an old link to a part of the page goes to that part in
 * its section (keeping the query, which some parts read). Undefined: it's fine as it is.
 */
export function redirectForOldLink(
  location: { pathname: string; search: string; hash: string },
  desktop: boolean,
): string | undefined {
  if (location.pathname.replace(/\/+$/, "") !== "/account") return undefined;
  const section = sectionForPart(location.hash, desktop);
  if (!section || section.id === "profile") return undefined;
  return `${accountPath(section.id)}${location.search}${location.hash}`;
}
