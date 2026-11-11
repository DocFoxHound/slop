// SPDX-License-Identifier: AGPL-3.0-or-later
// The kinds the bell's page lists (Notifications.tsx KINDS), what each says shows up there, and
// the line under the list about email.
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import { BELL_ONLY, EmailNote, KINDS } from "./Notifications";

afterEach(cleanup);

describe("the kinds of notification", () => {
  it("has maker news, after Interested, and says it's never emailed", () => {
    // Its promise holds while news is bell-only (crates/slop-api/src/notify.rs BELL_ONLY): moving it
    // out of that list makes this text and the Privacy Policy false (amendment 10.13).
    const ids = KINDS.map((k) => k.id);
    expect(ids.indexOf("news")).toBe(ids.indexOf("interested") + 1);
    const news = KINDS.find((k) => k.id === "news")!;
    expect(news.label).toBe("News");
    expect(news.about).toContain("never emailed");
    expect(new Set(ids).size).toBe(ids.length);
    expect(BELL_ONLY).toEqual(["news"]);
  });
});

describe("the line about email under the list", () => {
  const show = (kind?: Parameters<typeof EmailNote>[0]["kind"]) =>
    render(
      <MemoryRouter>
        <EmailNote kind={kind} />
      </MemoryRouter>,
    );

  it("points a kind that can be emailed to its switch on the account page", () => {
    const { container } = show("jobs");
    expect(container.textContent).toBe("Choose which of these are also emailed to you on your account page.");
    expect(screen.getByRole("link", { name: "your account page" }).getAttribute("href")).toBe("/account/notifications");
  });

  it("for news, says it's never emailed, with no switch to look for", () => {
    const { container } = show("news");
    expect(container.textContent).toBe("News is never emailed.");
    expect(screen.queryByRole("link")).toBeNull();
  });

  it("for every kind at once, leaves news out", () => {
    const { container } = show();
    expect(container.textContent).toBe("Choose which of these, other than news, are also emailed to you on your account page.");
  });
});
