// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type DonateInfo, type DonationSettings } from "../api";
import { DonateButton } from "./Donate";

const session = vi.hoisted(() => ({ me: null as null | { id: string; adult_confirmed: boolean } }));
vi.mock("../session", () => ({ useSession: () => ({ me: session.me, refresh: async () => {} }) }));
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
const on: DonateInfo = { ...info, on: true, off_reason: null };
const signedIn: DonateInfo = { ...on, cant: null, mine: { last: null, monthly: null } };
const servers = { statement: "Pays for two servers.", example_consent: "terms" };
const monthly = { id: "sd1", total_minor: 546, currency: "usd", started_at: "2026-10-01T00:00:00Z", next_charge_at: "2026-10-31T00:00:00Z" };

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
      <DonateButton slug="herds" title="Angry Herds" isMine={isMine} />
    </MemoryRouter>,
  );
const donateButton = (name: string | RegExp = /^Donate$/) => screen.findByRole("button", { name });

describe("DonateButton", () => {
  beforeEach(() => {
    session.me = null;
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("is green like Play while you can donate, and opens the form right there", async () => {
    session.me = { id: "u1", adult_confirmed: true };
    vi.spyOn(api.donations, "info").mockResolvedValue(signedIn);
    show();
    const button = await donateButton();
    expect(button.className).toContain("slime");
    expect(button.className).not.toContain("big");
    expect(button.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(button);
    expect(button.getAttribute("aria-expanded")).toBe("true");
    const panel = screen.getByRole("dialog", { name: "Donate to Angry Herds" });
    expect(button.getAttribute("aria-controls")).toBe(panel.id);
    await waitFor(() => expect(document.activeElement).toBe(panel));
    expect(screen.getByRole("form", { name: "Donate to Angry Herds" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Choose an amount" })).toBeTruthy();
    // Escape closes it and puts focus back on the button.
    fireEvent.keyDown(panel, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  it("when donations are off it's quiet, says so beside it, and says why on hover, focus and click", async () => {
    vi.spyOn(api.donations, "info").mockResolvedValue(info);
    show();
    const button = await donateButton();
    expect(button.className).not.toContain("slime");
    expect(button.className).toContain("quiet");
    expect(screen.getByText("Donations off")).toBeTruthy();
    const tip = screen.getByRole("tooltip", { hidden: true });
    expect(button.getAttribute("aria-describedby")).toBe(tip.id);
    expect(tip.textContent).toBe("Donations are off. The maker hasn't turned on donations.");
    fireEvent.click(button);
    expect(screen.getByText("Donations are off.")).toBeTruthy();
    expect(screen.getByText(/never unlock anything/)).toBeTruthy();
  });

  it("gives signed-out visitors the amounts and a Sign in to donate button", async () => {
    vi.spyOn(api.donations, "info").mockResolvedValue(on);
    show();
    const button = await donateButton();
    expect(button.className).toContain("slime");
    fireEvent.click(button);
    expect(screen.getByRole("radiogroup", { name: "Amount" })).toBeTruthy();
    const link = screen.getByRole("link", { name: "Sign in to donate" });
    expect(link.getAttribute("href")).toBe("/signin?return_to=/app/herds");
  });

  it("goes grey once you've donated, says what and when, and offers Donate again", async () => {
    session.me = { id: "u1", adult_confirmed: true };
    vi.spyOn(api.donations, "info").mockResolvedValue({
      ...signedIn,
      mine: { last: { total_minor: 340, currency: "usd", paid_at: "2026-10-03T12:00:00Z" }, monthly: null },
    });
    show();
    const button = await donateButton(/^Donated$/);
    expect(button.className).not.toContain("slime");
    expect(button.className).toContain("given");
    fireEvent.click(button);
    const panel = screen.getByRole("dialog", { name: "Donate to Angry Herds" });
    expect(panel.textContent).toContain("You donated $3.40 on");
    expect(screen.queryByRole("form")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Donate again" }));
    expect(screen.getByRole("form", { name: "Donate to Angry Herds" })).toBeTruthy();
    // Focus stays in the panel rather than falling to the page.
    await waitFor(() => expect(document.activeElement).toBe(panel));
  });

  it("shows a monthly server donation grey with Cancel, which asks first and ends it at once", async () => {
    session.me = { id: "u1", adult_confirmed: true };
    const load = vi.spyOn(api.donations, "info").mockResolvedValue({ ...signedIn, servers, mine: { last: null, monthly } });
    const cancel = vi.spyOn(api.donations, "cancel").mockResolvedValue(undefined);
    show();
    const button = await donateButton("$5.46/month server donation");
    expect(button.className).toContain("given");
    // The panel says what it is; one at a time, so Donate again offers only once.
    fireEvent.click(button);
    expect(screen.getByRole("dialog").textContent).toContain("You give $5.46 a month for Angry Herds's servers");
    fireEvent.click(screen.getByRole("button", { name: "Donate again" }));
    expect(screen.queryByRole("tablist", { name: "How often" })).toBeNull();

    // Keep it backs out, and focus goes back to Cancel.
    fireEvent.click(screen.getByRole("button", { name: "Cancel server donation" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    const confirm = screen.getByRole("group", { name: "Cancel your server donation" });
    expect(confirm.textContent).toContain("Cancel your server donation of $5.46 a month to Angry Herds? It ends now");
    expect(confirm.textContent).toContain("including the one due");
    await waitFor(() => expect(document.activeElement).toBe(confirm));
    fireEvent.click(screen.getByRole("button", { name: "Keep it" }));
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel server donation" })));
    expect(cancel).not.toHaveBeenCalled();

    // Yes cancels it straight away (no other step), and the button reloads.
    fireEvent.click(screen.getByRole("button", { name: "Cancel server donation" }));
    load.mockResolvedValue({ ...signedIn, servers });
    fireEvent.click(screen.getByRole("button", { name: "Yes, cancel it" }));
    await waitFor(() => expect(cancel).toHaveBeenCalledWith("sd1"));
    expect((await screen.findByRole("status")).textContent).toBe(
      "Your server donation to Angry Herds is cancelled. No more payments will be taken.",
    );
    expect((await donateButton()).className).toContain("slime");
    expect(screen.queryByRole("button", { name: "Cancel server donation" })).toBeNull();
  });

  it("says when cancelling fails, and keeps the donation", async () => {
    session.me = { id: "u1", adult_confirmed: true };
    vi.spyOn(api.donations, "info").mockResolvedValue({
      ...signedIn,
      servers,
      mine: { last: null, monthly: { ...monthly, next_charge_at: null } },
    });
    vi.spyOn(api.donations, "cancel").mockRejectedValue(new ApiError(503, "unavailable", "Stripe is down"));
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Cancel server donation" }));
    expect(screen.getByRole("group", { name: "Cancel your server donation" }).textContent).not.toContain("including");
    fireEvent.click(screen.getByRole("button", { name: "Yes, cancel it" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Stripe is down");
    expect(screen.getByRole("group", { name: "Cancel your server donation" })).toBeTruthy();
  });

  it("lets the maker turn donations on from the panel once Stripe is ready", async () => {
    const load = vi.spyOn(api.donations, "info").mockResolvedValue({ ...info, cant: "It's your own app." });
    vi.spyOn(api.donations, "settings").mockResolvedValue(settings);
    vi.spyOn(api.donations, "hold").mockResolvedValue({ open: null, can_ask_at: null });
    const save = vi.spyOn(api.donations, "saveSettings").mockResolvedValue(undefined as never);
    show(true);
    const button = await donateButton();
    expect(button.className).toContain("quiet");
    fireEvent.click(button);
    fireEvent.click(await screen.findByRole("button", { name: "Turn on donations" }));
    await waitFor(() => expect(save).toHaveBeenCalledWith("herds", expect.objectContaining({ donations_on: true, donation_suggested_minor: 500 })));
    await waitFor(() => expect(load).toHaveBeenCalledTimes(2));
  });

  it("starts the maker's Stripe sign-up from the panel, coming back to the store page", async () => {
    vi.spyOn(api.donations, "info").mockResolvedValue({
      ...info,
      off_reason: "The maker's Stripe account isn't ready to receive donations yet.",
      cant: "It's your own app.",
    });
    vi.spyOn(api.donations, "settings").mockResolvedValue({ ...settings, account_status: "none" });
    vi.spyOn(api.donations, "hold").mockResolvedValue({ open: null, can_ask_at: null });
    const onboard = vi.spyOn(api.donations, "onboard").mockResolvedValue({ url: null, status: "active" });
    show(true);
    fireEvent.click(await donateButton());
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
    fireEvent.click(await donateButton());
    expect(await within(screen.getByRole("dialog")).findByText(/aren't switched on on this server yet/)).toBeTruthy();
    await waitFor(() => expect(read).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));
    // Only the button itself and the panel's Close.
    expect(screen.getAllByRole("button").map((b) => b.getAttribute("aria-label") ?? b.textContent)).toEqual(["♥ Donate", "Close"]);
  });

  it("shows nothing at all for an app that isn't in the store", async () => {
    vi.spyOn(api.donations, "info").mockRejectedValue(new ApiError(404, "not_found", "not found"));
    const { container } = show();
    await new Promise((r) => setTimeout(r, 0));
    expect(container.textContent).toBe("");
  });
});
