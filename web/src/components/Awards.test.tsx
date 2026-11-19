// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import type { AppAward } from "../api";
import { awardAmount, AwardBadges, awardText, MakerAwardBadge, monthLabel } from "./Awards";

const award = (month: string, amount_minor = 2500, currency: AppAward["currency"] = "usd"): AppAward => ({
  month,
  position: 1,
  amount_minor,
  currency,
  blurb: "",
});

describe("App of the Month badges", () => {
  afterEach(cleanup);

  it("says the month and the award in words", () => {
    expect(monthLabel("2026-11")).toBe("November 2026");
    expect(monthLabel("2027-01")).toBe("January 2027");
    expect(awardAmount(2500, "usd")).toBe("$25");
    expect(awardAmount(1250, "eur")).toBe("€12.50");
    expect(awardAmount(3000, "gbp")).toBe("£30");
    expect(awardText(award("2026-11"))).toBe("App of the Month · November 2026 · $25 award");
  });

  it("shows one badge a month on the store page, linking to the awards page, and the rest as a count", () => {
    render(
      <MemoryRouter>
        <AwardBadges awards={[award("2026-11"), award("2026-10", 3000, "eur"), award("2026-08"), award("2026-05")]} />
      </MemoryRouter>,
    );
    const links = screen.getAllByRole("link");
    expect(links.map((l) => l.textContent)).toEqual([
      "🏆 App of the Month · November 2026 · $25 award",
      "🏆 App of the Month · October 2026 · €30 award",
      "🏆 App of the Month · August 2026 · $25 award",
      "+1 more month",
    ]);
    expect(links.every((l) => l.getAttribute("href") === "/awards")).toBe(true);
    // The trophy is decoration; the words carry it.
    expect(links[0].querySelector('[aria-hidden="true"]')?.textContent).toBe("🏆");
  });

  it("shows nothing for an app that never won", () => {
    const { container } = render(
      <MemoryRouter>
        <AwardBadges awards={[]} />
        <MakerAwardBadge count={0} />
      </MemoryRouter>,
    );
    expect(container.textContent).toBe("");
  });

  it("counts a maker's awards only when there's more than one", () => {
    const { rerender } = render(
      <MemoryRouter>
        <MakerAwardBadge count={1} />
      </MemoryRouter>,
    );
    expect(screen.getByRole("link").textContent).toBe("🏆 App of the Month");
    rerender(
      <MemoryRouter>
        <MakerAwardBadge count={5} />
      </MemoryRouter>,
    );
    const link = screen.getByRole("link", { name: "App of the Month, 5 times" });
    expect(link.getAttribute("href")).toBe("/awards");
  });
});
