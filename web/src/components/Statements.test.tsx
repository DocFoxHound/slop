// SPDX-License-Identifier: AGPL-3.0-or-later
// Yearly statements (Statements.tsx): the totals as accessible tables, the year picker, the CSV
// download, the note that it isn't a tax form, and simulated payments said as such.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type StatementTotals, type VatTreatment, type YearStatement } from "../api";
import { saveFile } from "../platform";
import { amount, sum, YearlyStatements } from "./Statements";

vi.mock("../platform", async (original) => ({
  ...(await original<typeof import("../platform")>()),
  saveFile: vi.fn(),
  openExternal: vi.fn(async () => {}),
}));

const totals = (period: string, kind: StatementTotals["kind"], over: Partial<StatementTotals> = {}): StatementTotals => ({
  period,
  currency: "usd",
  kind,
  payments: 1,
  charged_minor: 1059,
  processing_covered_minor: 59,
  gross_minor: 1000,
  refunded_minor: 0,
  disputed_minor: 0,
  slop_fee_minor: 50,
  vat_minor: 0,
  net_minor: 950,
  ...over,
});
const vat = (over: Partial<VatTreatment> = {}): VatTreatment => ({
  kind: "none",
  rate_bp: null,
  country: "US",
  vat_number: null,
  vat_number_checked: false,
  slop_vat_id: null,
  ...over,
});
const statement = (year: number, over: Partial<YearStatement> = {}): YearStatement => ({
  year,
  simulated: false,
  totals: [
    totals(String(year), "donation", { payments: 2, gross_minor: 3000, refunded_minor: 800, slop_fee_minor: 150, net_minor: 2050 }),
    totals(String(year), "people_job", { gross_minor: 12000, disputed_minor: 3000, slop_fee_minor: 600, net_minor: 8400 }),
  ],
  months: [
    totals(`${year}-01`, "donation", { payments: 2, gross_minor: 3000, refunded_minor: 800, slop_fee_minor: 150, net_minor: 2050 }),
    totals(`${year}-03`, "people_job", { gross_minor: 12000, disputed_minor: 3000, slop_fee_minor: 600, net_minor: 8400 }),
  ],
  vat: vat(),
  ...over,
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("amount and sum", () => {
  it("formats with cents and a real minus sign", () => {
    expect(amount(123450, "usd")).toBe("$1,234.50");
    expect(amount(-25, "eur")).toBe("−€0.25");
    expect(amount(0, "gbp")).toBe("£0.00");
  });

  it("adds rows field by field", () => {
    const s = sum([totals("2026", "donation"), totals("2026", "bounty", { refunded_minor: 100, net_minor: 850 })]);
    expect(s.payments).toBe(2);
    expect(s.gross_minor).toBe(2000);
    expect(s.refunded_minor).toBe(100);
    expect(s.net_minor).toBe(1800);
  });
});

describe("YearlyStatements", () => {
  it("shows nothing without a year of money", () => {
    const year = vi.spyOn(api.statements, "year");
    const { container } = render(<YearlyStatements years={[]} simulated={false} />);
    expect(container.innerHTML).toBe("");
    expect(year).not.toHaveBeenCalled();
  });

  it("shows the newest year as tables with row and column headers, and the note", async () => {
    vi.spyOn(api.statements, "year").mockResolvedValue(statement(2026));
    render(<YearlyStatements years={[2026, 2025]} simulated={false} />);
    const table = await screen.findByRole("table", { name: "2026 in US dollars" });
    const cols = within(table).getAllByRole("columnheader").map((c) => c.textContent);
    expect(cols).toEqual(["Donations", "People jobs", "Total"]);
    const net = within(table).getByRole("rowheader", { name: "Net" }).closest("tr")!;
    expect([...net.querySelectorAll("td")].map((c) => c.textContent)).toEqual(["$20.50", "$84.00", "$104.50"]);
    const refunds = within(table).getByRole("rowheader", { name: "Refunds" }).closest("tr")!;
    expect(refunds.querySelector("td")!.textContent).toBe("−$8.00");
    expect(within(table).queryByRole("rowheader", { name: "VAT in the fee" })).toBeNull();
    const months = screen.getByRole("table", { name: "2026 by month, in US dollars" });
    expect(within(months).getAllByRole("rowheader").map((c) => c.textContent)).toEqual(["January", "March"]);
    const section = screen.getByRole("region", { name: "Yearly statements" });
    expect(section.textContent).toContain("isn't a tax form");
    expect(section.textContent).toContain("1099-K");
    expect(section.textContent).toContain("doesn't give tax advice");
    expect(screen.getByRole("link", { name: "Stripe dashboard" })).toBeTruthy();
    expect(screen.getByLabelText("Year")).toHaveProperty("value", "2026");
    expect(screen.queryByText(/simulates payments/)).toBeNull();
  });

  it("loads another year when picked, and downloads its CSV", async () => {
    const year = vi.spyOn(api.statements, "year").mockImplementation(async (y) => statement(y));
    const csv = vi.spyOn(api.statements, "csv").mockResolvedValue("Period,Currency\r\n");
    render(<YearlyStatements years={[2026, 2025]} simulated={false} />);
    await screen.findByRole("table", { name: "2026 in US dollars" });
    fireEvent.change(screen.getByLabelText("Year"), { target: { value: "2025" } });
    await screen.findByRole("table", { name: "2025 in US dollars" });
    expect(year).toHaveBeenLastCalledWith(2025);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Download CSV" })));
    expect(csv).toHaveBeenCalledWith(2025);
    expect(saveFile).toHaveBeenCalledWith("slaporslop-statement-2025.csv", "Period,Currency\r\n", "text/csv");
  });

  it("shows the VAT inside the fee where it's included, and says when payments are simulated", async () => {
    const s = statement(2026, { vat: vat({ kind: "included", rate_bp: 1900, country: "DE" }) });
    s.totals = s.totals.map((t) => ({ ...t, vat_minor: 24 }));
    vi.spyOn(api.statements, "year").mockResolvedValue(s);
    vi.spyOn(api.statements, "csv").mockResolvedValue("x");
    render(<YearlyStatements years={[2026]} simulated={true} />);
    const table = await screen.findByRole("table", { name: "2026 in US dollars" });
    const row = within(table).getByRole("rowheader", { name: "VAT in the fee" }).closest("tr")!;
    expect(row.querySelector("td")!.textContent).toBe("$0.24");
    expect(screen.getByText(/includes 19% VAT/)).toBeTruthy();
    expect(screen.getByRole("note").textContent).toContain("not real money");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Download CSV" })));
    await waitFor(() => expect(saveFile).toHaveBeenCalledWith("slaporslop-statement-2026-simulated.csv", "x", "text/csv"));
  });
});
