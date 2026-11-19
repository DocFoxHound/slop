// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type AdminAwards } from "../../api";
import { AwardsTab } from "./Awards";

const month: AdminAwards = {
  month: "2026-11",
  current: true,
  can_confirm: false,
  max_votes: 3,
  max_winners: 3,
  my_votes: 2,
  nominations: [
    {
      slug: "tea-shop",
      title: "Tea Shop",
      eligible: true,
      mine: false,
      nominated_by: "Rita Reviewer",
      note: "Players love it.",
      created_at: "2026-11-02T00:00:00Z",
      votes: 2,
      voters: ["Rita Reviewer", "Olive Owner"],
      my_vote: true,
    },
    {
      slug: "moss",
      title: "Moss",
      eligible: true,
      mine: false,
      nominated_by: "Olive Owner",
      note: "",
      created_at: "2026-11-03T00:00:00Z",
      votes: 0,
      voters: [],
      my_vote: false,
    },
    {
      slug: "my-game",
      title: "My Game",
      eligible: true,
      mine: true,
      nominated_by: "Olive Owner",
      note: "",
      created_at: "2026-11-03T00:00:00Z",
      votes: 1,
      voters: ["Olive Owner"],
      my_vote: false,
    },
  ],
  winners: [],
  months: ["2026-11", "2026-10"],
};

const winner = {
  slug: "tea-shop",
  title: "Tea Shop",
  position: 1,
  amount_minor: 2500,
  currency: "usd" as const,
  blurb: "Cozy.",
  confirmed_by: "Olive Owner",
  confirmed_at: "2026-11-20T00:00:00Z",
  prize_paid_at: null,
  accepts_donations: true,
  donation_currency: "usd" as const,
};

const show = () =>
  render(
    <MemoryRouter>
      <AwardsTab />
    </MemoryRouter>,
  );

describe("the Awards tab", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("lists nominations with their votes, and votes and takes votes back; never for your own app", async () => {
    vi.spyOn(api.admin, "awards").mockResolvedValue(month);
    const voteAward = vi.spyOn(api.admin, "voteAward").mockResolvedValue(undefined);
    show();
    expect(await screen.findByText("You have 1 of 3 votes left this month.")).toBeTruthy();
    expect(screen.getByText(/\(Rita Reviewer, Olive Owner\)/)).toBeTruthy();
    const voted = screen.getByRole("button", { name: "Voted for Tea Shop" });
    expect(voted.getAttribute("aria-pressed")).toBe("true");
    expect((screen.getByRole("button", { name: "Vote for My Game" }) as HTMLButtonElement).disabled).toBe(true);
    // Staff who aren't the owner don't get the confirm form.
    expect(screen.queryByRole("form", { name: /Confirm winners/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Vote for Moss" }));
    await waitFor(() => expect(voteAward).toHaveBeenCalledWith("moss", true));
    fireEvent.click(voted);
    await waitFor(() => expect(voteAward).toHaveBeenCalledWith("tea-shop", false));
  });

  it("stops at three votes", async () => {
    vi.spyOn(api.admin, "awards").mockResolvedValue({ ...month, my_votes: 3 });
    show();
    expect(await screen.findByText("You have 0 of 3 votes left this month.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Vote for Moss" }) as HTMLButtonElement).disabled).toBe(true);
    // Taking one back is still possible.
    expect((screen.getByRole("button", { name: "Voted for Tea Shop" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("nominates an app from its address or a link to it", async () => {
    vi.spyOn(api.admin, "awards").mockResolvedValue(month);
    const nominate = vi.spyOn(api.admin, "nominateAward").mockResolvedValue(month);
    show();
    fireEvent.change(await screen.findByLabelText("App address to nominate"), {
      target: { value: "https://slaporslop.games/app/lanterns?x=1" },
    });
    fireEvent.change(screen.getByLabelText("Why (for staff, optional)"), { target: { value: " Glows. " } });
    fireEvent.click(screen.getByRole("button", { name: "Nominate" }));
    await waitFor(() => expect(nominate).toHaveBeenCalledWith("lanterns", "Glows."));
  });

  it("lets the owner confirm winners with an award each, $25 unless changed", async () => {
    vi.spyOn(api.admin, "awards").mockResolvedValue({ ...month, can_confirm: true });
    const confirm = vi.spyOn(api.admin, "confirmAwards").mockResolvedValue(month);
    show();
    const form = await screen.findByRole("form", { name: "Confirm winners for November 2026" });
    const box = (name: RegExp) => within(form).getByRole("checkbox", { name });
    // Your own app can't be picked.
    expect((box(/^My Game/) as HTMLInputElement).disabled).toBe(true);
    fireEvent.click(box(/^Tea Shop/));
    fireEvent.click(box(/^Moss/));
    const amounts = within(form).getAllByLabelText("Award");
    fireEvent.change(amounts[1], { target: { value: "30" } });
    fireEvent.change(within(form).getAllByLabelText("Currency")[1], { target: { value: "eur" } });
    fireEvent.change(within(form).getAllByLabelText(/Why it won/)[0], { target: { value: "Cozy." } });
    fireEvent.click(within(form).getByRole("button", { name: "Confirm winners" }));
    await waitFor(() =>
      expect(confirm).toHaveBeenCalledWith("2026-11", [
        { slug: "tea-shop", amount_minor: 2500, currency: "usd", blurb: "Cozy." },
        { slug: "moss", amount_minor: 3000, currency: "eur", blurb: "" },
      ]),
    );
  });

  it("shows the winners with a Pay prize link that opens the app's donation with the award filled in, and Mark paid", async () => {
    vi.spyOn(api.admin, "awards").mockResolvedValue({ ...month, can_confirm: true, winners: [winner] });
    const paid = vi.spyOn(api.admin, "markPrizePaid").mockResolvedValue(month);
    show();
    const pay = await screen.findByRole("link", { name: "Pay prize: donate $25 to Tea Shop" });
    expect(pay.getAttribute("href")).toBe("/app/tea-shop?donate=25.00");
    expect(screen.getByText(/It takes donations right now/)).toBeTruthy();
    expect(screen.getByText("NOT PAID YET")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Mark Tea Shop's prize paid" }));
    await waitFor(() => expect(paid).toHaveBeenCalledWith("2026-11", "tea-shop", true));
  });

  it("says when a winner can't take the prize yet, or takes another currency", async () => {
    vi.spyOn(api.admin, "awards").mockResolvedValue({
      ...month,
      can_confirm: true,
      winners: [{ ...winner, accepts_donations: false, donation_currency: "eur" }],
    });
    show();
    expect(await screen.findByText(/doesn't take donations right now/)).toBeTruthy();
    expect(screen.getByText(/Its Donate button takes EUR/)).toBeTruthy();
    // No amount to fill in: it's in another currency.
    expect(screen.getByRole("link", { name: /Pay prize/ }).getAttribute("href")).toBe("/app/tea-shop?donate=");
  });

  it("fixes the winners once a prize is paid; other staff see them without the owner's buttons", async () => {
    const paidWinner = { ...winner, prize_paid_at: "2026-11-21T00:00:00Z" };
    vi.spyOn(api.admin, "awards").mockResolvedValue({ ...month, can_confirm: true, winners: [paidWinner] });
    show();
    expect(await screen.findByText(/winners are fixed/)).toBeTruthy();
    expect(screen.queryByRole("link", { name: /Pay prize/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Mark Tea Shop's prize not paid" })).toBeTruthy();
    cleanup();
    vi.spyOn(api.admin, "awards").mockResolvedValue({ ...month, winners: [paidWinner] });
    show();
    expect(await screen.findByText("Cozy.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /prize/ })).toBeNull();
  });
});
