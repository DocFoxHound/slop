// SPDX-License-Identifier: AGPL-3.0-or-later
// How ratings work, for games that play in the browser too: who can vote, and how their votes are
// weighted (by the days played, as votes.rs `vote_weight` has it).
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import { HowRatings } from "./HowRatings";

afterEach(cleanup);

describe("How ratings work", () => {
  it("says browser games' votes count by the days played, and that pressing Play is playing", () => {
    render(
      <MemoryRouter>
        <HowRatings />
      </MemoryRouter>,
    );
    const page = document.body.textContent ?? "";
    expect(page).toContain("or pressed Play on SLAPorSLOP for a game that plays in your browser");
    expect(page).toContain("Games that play in your browser always count this way: each day you pressed Play on SLAPorSLOP is a day played.");
    expect(screen.getByRole("table", { name: "Vote weight by days launched" })).toBeTruthy();
  });
});
