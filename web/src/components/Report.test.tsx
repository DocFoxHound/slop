import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from "../api";
import { ReportForm } from "./Report";

vi.mock("../session", () => ({ useSession: () => ({ me: null }) }));

describe("ReportForm", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  const reasons = [
    { code: "malware", label: "Malware, hidden mining or data theft", urgent: true },
    { code: "copyright", label: "Copyright or trademark infringement", urgent: true },
    { code: "broken", label: "The build is broken", urgent: false },
  ];

  it("asks a signed-out reporter who they are, and acknowledges with a reference", async () => {
    vi.spyOn(api.reports, "reasons").mockResolvedValue(reasons);
    const send = vi.spyOn(api.reports, "send").mockResolvedValue({ reference: "R-ABCD-EFGH", due_at: "2026-10-21T00:00:00Z" });
    render(
      <MemoryRouter>
        <ReportForm kind="app" subject="miner" label="this app" onClose={() => {}} />
      </MemoryRouter>,
    );
    fireEvent.click(await screen.findByLabelText(/Malware/));
    expect(screen.getByText("We review reports like this within 24 hours.")).toBeTruthy();
    fireEvent.change(screen.getByLabelText(/What did you see/), { target: { value: "It mines crypto." } });
    fireEvent.change(screen.getByLabelText(/Your name/), { target: { value: "Sam" } });
    fireEvent.change(screen.getByLabelText(/Your email/), { target: { value: "sam@example.com" } });
    const button = screen.getByRole("button", { name: "Send report" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.click(screen.getByLabelText(/accurate and complete/));
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    await waitFor(() => expect(screen.getByText("R-ABCD-EFGH")).toBeTruthy());
    expect(send).toHaveBeenCalledWith({
      kind: "app",
      subject: "miner",
      reason: "malware",
      details: "It mines crypto.",
      good_faith: true,
      name: "Sam",
      email: "sam@example.com",
    });
  });

  it("collects a copyright notice's details", async () => {
    vi.spyOn(api.reports, "reasons").mockResolvedValue(reasons);
    render(
      <MemoryRouter>
        <ReportForm kind="app" subject="miner" label="this app" onClose={() => {}} />
      </MemoryRouter>,
    );
    expect(screen.queryByText("Copyright notice")).toBeNull();
    fireEvent.click(await screen.findByLabelText(/Copyright/));
    expect(screen.getByText("Copyright notice")).toBeTruthy();
    expect(screen.getByLabelText(/penalty of perjury/)).toBeTruthy();
    expect(screen.getByLabelText(/as your signature/)).toBeTruthy();
  });
});
