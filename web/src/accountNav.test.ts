// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { ACCOUNT_SECTIONS, accountPath, accountSections, redirectForOldLink, sectionForPart, sectionFromSlug } from "./accountNav";

const at = (url: string) => {
  const u = new URL(url, "https://slop.example");
  return { pathname: u.pathname, search: u.search, hash: u.hash };
};

describe("account sections", () => {
  it("Profile is /account; the others have their own address", () => {
    expect(accountPath("profile")).toBe("/account");
    expect(accountPath("security")).toBe("/account/security");
    expect(sectionFromSlug(undefined, false)?.id).toBe("profile");
    expect(sectionFromSlug("money", false)?.id).toBe("money");
    expect(sectionFromSlug("nope", false)).toBeUndefined();
  });

  it("This computer is only in the desktop app", () => {
    expect(accountSections(false).map((s) => s.id)).not.toContain("device");
    expect(accountSections(true).map((s) => s.id)).toContain("device");
    expect(sectionFromSlug("device", false)).toBeUndefined();
    expect(sectionFromSlug("device", true)?.id).toBe("device");
  });

  it("every part belongs to one section", () => {
    const parts = ACCOUNT_SECTIONS.flatMap((s) => s.parts);
    expect(new Set(parts).size).toBe(parts.length);
    // "combine" is /account/combine, a page of its own.
    expect(ACCOUNT_SECTIONS.map((s) => s.id)).not.toContain("combine");
  });

  it.each([
    ["/account#picture", undefined],
    ["/account#privacy", undefined],
    ["/account?picture=refresh#picture", undefined],
    ["/account", undefined],
    ["/account#nothing-here", undefined],
    ["/account#sign-ins", "/account/security#sign-ins"],
    ["/account?linked=github#sign-ins", "/account/security?linked=github#sign-ins"],
    ["/account?confirm_link=gitlab#sign-ins", "/account/security?confirm_link=gitlab#sign-ins"],
    ["/account?error=provider#sign-ins", "/account/security?error=provider#sign-ins"],
    ["/account#signing-keys", "/account/agents#signing-keys"],
    ["/account#ai-agents", "/account/agents#ai-agents"],
    ["/account#notifications", "/account/notifications#notifications"],
    ["/account#donations", "/account/money#donations"],
    ["/account#receiving", "/account/money#receiving"],
    ["/account#fee-statements", "/account/money#fee-statements"],
    ["/account#decisions", "/account/moderation#decisions"],
    ["/account#reports", "/account/moderation#reports"],
    ["/account#agreements", "/account/agreements#agreements"],
    ["/account#data", "/account/data#data"],
    ["/account/security#sign-ins", undefined],
  ])("%s goes to %s", (url, to) => {
    expect(redirectForOldLink(at(url), false)).toBe(to);
  });

  it("#this-device opens This computer in the app, and stays on Profile on the website", () => {
    expect(redirectForOldLink(at("/account#this-device"), true)).toBe("/account/device#this-device");
    expect(redirectForOldLink(at("/account#this-device"), false)).toBeUndefined();
    expect(sectionForPart("#this-device", false)).toBeUndefined();
  });
});
