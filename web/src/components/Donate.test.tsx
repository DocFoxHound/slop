// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type DonateInfo, type DonationSettings } from "../api";
import { DonateBox } from "./Donate";

vi.mock("../session", () => ({ useSession: () => ({ me: null, refresh: async () => {} }) }));
vi.mock("./Terms", () => ({ useTermsGate: () => ({ prompt: null, guard: <T,>(f: () => Promise<T>) => f() }) }));

const info: DonateInfo = {
  on: false,
  off_reason: "The maker hasn't turned on donations.",
  currency: "usd",
  presets: [{ donation: 500, total: 546, slop_fee: 25, processing: 46, maker_receives: 475, donor_covers_fees: true }],
  min_minor: 100,
  max_minor: 100000,
  donor_covers_fees: true,
  servers: null,
  maker: "Mia Maker",
  cant: "Sign in to donate.",
  simulated: true,
  shared: null,
};

const settings: DonationSettings = {
  donations_on: false,
  donation_suggested_minor: 500,
  donation_currency: "usd",
  runs_servers: false,
  servers_statement: null,
  servers_shut_down_at: null,
  account_status: "active",
  active_server_donations: 0,
  payments: "simulated",
};

const show = (isMine = false) =>
  render(
    <MemoryRouter>
      <DonateBox slug="herds" title="Angry Herds" isMine={isMine} />
    </MemoryRouter>,
  );

describe("DonateBox", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("never hides: when donations are off it says why", async () => {
    vi.spyOn(api.donations, "info").mockResolvedValue(info);
    show();
    expect(await screen.findByText("Donations are off.")).toBeTruthy();
    expect(screen.getByText(/The maker hasn't turned on donations\./)).toBeTruthy();
    expect(screen.getByRole("region", { name: "Donate to Angry Herds" })).toBeTruthy();
  });

  it("gives signed-out visitors a Sign in to donate button", async () => {
    vi.spyOn(api.donations, "info").mockResolvedValue({ ...info, on: true, off_reason: null });
    show();
    const link = await screen.findByRole("link", { name: "Sign in to donate" });
    expect(link.getAttribute("href")).toBe("/signin?return_to=/app/herds");
    expect(link.className).toContain("btn");
  });

  it("lets the maker turn donations on right there once Stripe is ready", async () => {
    const load = vi.spyOn(api.donations, "info").mockResolvedValue({ ...info, cant: "It's your own app." });
    vi.spyOn(api.donations, "settings").mockResolvedValue(settings);
    vi.spyOn(api.donations, "hold").mockResolvedValue({ open: null, can_ask_at: null });
    const save = vi.spyOn(api.donations, "saveSettings").mockResolvedValue(undefined as never);
    show(true);
    fireEvent.click(await screen.findByRole("button", { name: "Turn on donations" }));
    await waitFor(() => expect(save).toHaveBeenCalledWith("herds", expect.objectContaining({ donations_on: true, donation_suggested_minor: 500 })));
    await waitFor(() => expect(load).toHaveBeenCalledTimes(2));
  });

  it("starts the maker's Stripe sign-up right there, coming back to the store page", async () => {
    vi.spyOn(api.donations, "info").mockResolvedValue({
      ...info,
      off_reason: "The maker's Stripe account isn't ready to receive donations yet.",
      cant: "It's your own app.",
    });
    vi.spyOn(api.donations, "settings").mockResolvedValue({ ...settings, account_status: "none" });
    vi.spyOn(api.donations, "hold").mockResolvedValue({ open: null, can_ask_at: null });
    const onboard = vi.spyOn(api.donations, "onboard").mockResolvedValue({ url: null, status: "active" });
    show(true);
    const go = await screen.findByRole("button", { name: "Enable donations with Stripe" });
    expect((go as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByLabelText(/donations may be taxable income/));
    fireEvent.click(go);
    await waitFor(() =>
      expect(onboard).toHaveBeenCalledWith({ tax_notice_ack: true, confirm_adult: false, return_to: "/app/herds" }),
    );
  });

  it("shows nothing for the maker to fix when payments are off on the server", async () => {
    vi.spyOn(api.donations, "info").mockResolvedValue({ ...info, off_reason: "Donations aren't switched on on this server yet." });
    const read = vi.spyOn(api.donations, "settings").mockResolvedValue({ ...settings, payments: "off" });
    show(true);
    expect(await screen.findByText(/aren't switched on on this server yet/)).toBeTruthy();
    await waitFor(() => expect(read).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByRole("button")).toBeNull();
  });
});
