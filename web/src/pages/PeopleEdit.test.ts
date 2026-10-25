// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import type { MyProfile } from "../api";
import { toInput } from "./PeopleEdit";

describe("saving a People page loaded from the server", () => {
  it("sends only the fields the server takes (it refuses any others)", () => {
    const mine: MyProfile = {
      handle: "vera",
      name: "Vera",
      kind: "individual",
      team_size: null,
      headline: "Pixels",
      about_md: "",
      location: "",
      skills: ["pixel-art"],
      ai_use: "none",
      ai_note: "",
      trader: false,
      taking_work: true,
      turnaround: "",
      services: [],
      accent: "rose",
      sections: ["portfolio", "services", "reviews", "worked_on", "about"],
      links: [],
      portfolio_repo: "https://github.com/vera/portfolio",
      stripe_status: "active",
      live: true,
      hidden_reason: null,
      payments: "stripe",
      status: { state: "live", reason: null, update_waiting: false, waiting_since: null, problem: null, suspended: false },
    };
    const input = toInput(mine);
    // people/mod.rs ProfileInput, less confirm_adult (added when saving).
    expect(Object.keys(input).sort()).toEqual(
      [
        "handle", "name", "kind", "team_size", "headline", "about_md", "location", "skills", "ai_use", "ai_note", "trader",
        "taking_work", "turnaround", "services", "accent", "sections", "links",
      ].sort(),
    );
    expect(input.accent).toBe("rose");
  });
});
