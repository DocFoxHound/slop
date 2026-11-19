// SPDX-License-Identifier: AGPL-3.0-or-later
// Account → Money: the yearly statement of money paid into the person's own Stripe accounts
// (statements.rs, PAYMENTS §8), with SLAPorSLOP's fee and the VAT inside it (LEGAL §3.5), and a
// CSV download. Totals only: payment-by-payment details are in their Stripe dashboard.

import { useState, type FormEvent } from "react";

import { api, ApiError, type StatementTotals, type VatTreatment, type YearStatement } from "../api";
import { useLoad } from "../hooks";
import { saveFile } from "../platform";
import { ExternalLink } from "./ui";

const SYMBOL: Record<string, string> = { usd: "$", eur: "€", gbp: "£" };
const CURRENCY: Record<string, string> = { usd: "US dollars", eur: "Euros", gbp: "Pounds sterling" };
const KINDS: Array<{ kind: StatementTotals["kind"]; label: string }> = [
  { kind: "donation", label: "Donations" },
  { kind: "bounty", label: "Bounties" },
  { kind: "people_job", label: "People jobs" },
];
const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** "$1,234.50", "−$0.25": always with cents, and a minus sign screen readers say. */
export function amount(minor: number, currency: string): string {
  const units = (Math.abs(minor) / 100).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return `${minor < 0 ? "−" : ""}${SYMBOL[currency] ?? ""}${units}${SYMBOL[currency] ? "" : ` ${currency.toUpperCase()}`}`;
}

type Sums = Omit<StatementTotals, "period" | "currency" | "kind">;
const ZERO: Sums = {
  payments: 0,
  charged_minor: 0,
  processing_covered_minor: 0,
  gross_minor: 0,
  refunded_minor: 0,
  disputed_minor: 0,
  slop_fee_minor: 0,
  vat_minor: 0,
  net_minor: 0,
};

/** Rows added up, field by field. */
export function sum(rows: Sums[]): Sums {
  const out = { ...ZERO };
  for (const r of rows) for (const k of Object.keys(ZERO) as Array<keyof Sums>) out[k] += r[k];
  return out;
}

/** The section: hidden until there's money in some year. */
export function YearlyStatements({ years, simulated }: { years: number[]; simulated: boolean }) {
  const [picked, setPicked] = useState<number>();
  const year = picked ?? years[0];
  const statement = useLoad(() => (year ? api.statements.year(year) : Promise.resolve(undefined)), [year]);
  const [error, setError] = useState<string>();
  if (years.length === 0) return null;
  const download = async () => {
    setError(undefined);
    try {
      saveFile(`slaporslop-statement-${year}${simulated ? "-simulated" : ""}.csv`, await api.statements.csv(year), "text/csv");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  // The year asked for, not the one shown before it while it loads.
  const s = statement.data?.year === year ? statement.data : undefined;
  return (
    <section id="statements" className="account-panel" aria-labelledby="statements-title">
      <h3 className="account-panel-title" id="statements-title">
        Yearly statements
      </h3>
      <p className="muted small">
        A summary of the money paid into your Stripe account for your records. It isn't a tax form: Stripe sends any US 1099-K
        for your account, and payment-by-payment details are in your <ExternalLink href="https://dashboard.stripe.com/payments">Stripe dashboard</ExternalLink>.
        SLAPorSLOP doesn't give tax advice.
      </p>
      <div className="row-actions statement-actions">
        <label>
          Year{" "}
          <select value={year} onChange={(e) => setPicked(Number(e.target.value))}>
            {years.map((y) => (
              <option key={y} value={y}>
                {y}
              </option>
            ))}
          </select>
        </label>
        <button type="button" className="btn small" onClick={() => void download()}>
          Download CSV
        </button>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {simulated && (
        <p className="notice" role="note">
          This server simulates payments: these are test payments, not real money.
        </p>
      )}
      {statement.error && !s ? (
        <p className="form-error" role="alert">
          {statement.error.message}{" "}
          <button type="button" className="btn small ghost" onClick={statement.reload}>
            Try again
          </button>
        </p>
      ) : !s ? (
        <p className="muted small" aria-live="polite">
          Loading {year}…
        </p>
      ) : (
        <Statement statement={s} onVat={statement.reload} />
      )}
    </section>
  );
}

function Statement({ statement, onVat }: { statement: YearStatement; onVat: () => void }) {
  const { vat, year } = statement;
  const showVat = vat.kind === "included";
  const currencies = [...new Set(statement.totals.map((t) => t.currency))];
  return (
    <>
      {currencies.length === 0 && <p className="muted">No money in {year}.</p>}
      {currencies.map((currency) => {
        const rows = statement.totals.filter((t) => t.currency === currency);
        const kinds = KINDS.filter((k) => rows.some((r) => r.kind === k.kind));
        const columns: Array<{ label: string; sums: Sums }> = kinds.map((k) => ({
          label: k.label,
          sums: sum(rows.filter((r) => r.kind === k.kind)),
        }));
        if (columns.length > 1) columns.push({ label: "Total", sums: sum(rows) });
        const line = (label: string, value: (x: Sums) => string) => (
          <tr>
            <th scope="row">{label}</th>
            {columns.map((c) => (
              <td key={c.label}>{value(c.sums)}</td>
            ))}
          </tr>
        );
        const money = (pick: (x: Sums) => number) => (x: Sums) => amount(pick(x), currency);
        const months = statement.months.filter((m) => m.currency === currency);
        const byMonth = [...new Set(months.map((m) => m.period))].map((period) => ({
          period,
          sums: sum(months.filter((m) => m.period === period)),
        }));
        return (
          <div key={currency} className="statement">
            <div className="table-scroll">
              <table className="table small panel statement-table">
                <caption>
                  {year} in {CURRENCY[currency] ?? currency.toUpperCase()}
                </caption>
                <thead>
                  <tr>
                    <td />
                    {columns.map((c) => (
                      <th key={c.label} scope="col">
                        {c.label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {line("Payments", (x) => String(x.payments))}
                  {line("Received", money((x) => x.gross_minor))}
                  {line("Refunds", money((x) => -x.refunded_minor))}
                  {line("Disputes", money((x) => -x.disputed_minor))}
                  {line("SLAPorSLOP fee", money((x) => -x.slop_fee_minor))}
                  {showVat && line("VAT in the fee", money((x) => x.vat_minor))}
                  {line("Net", money((x) => x.net_minor))}
                </tbody>
              </table>
            </div>
            <div className="table-scroll">
              <table className="table small panel statement-table">
                <caption>
                  {year} by month, in {CURRENCY[currency] ?? currency.toUpperCase()}
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Month</th>
                    <th scope="col">Received</th>
                    <th scope="col">Refunds and disputes</th>
                    <th scope="col">SLAPorSLOP fee</th>
                    <th scope="col">Net</th>
                  </tr>
                </thead>
                <tbody>
                  {byMonth.map(({ period, sums }) => (
                    <tr key={period}>
                      <th scope="row">{MONTHS[Number(period.slice(5, 7)) - 1] ?? period}</th>
                      <td>{amount(sums.gross_minor, currency)}</td>
                      <td>{amount(-(sums.refunded_minor + sums.disputed_minor), currency)}</td>
                      <td>{amount(-sums.slop_fee_minor, currency)}</td>
                      <td>{amount(sums.net_minor, currency)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        );
      })}
      <p className="faint small">
        Received is what payers paid toward you; the card processing donors added on top went toward Stripe's fee, so it's
        left out (the CSV file has it). Net is before Stripe's own fees. Refunds and disputes count in the month they were made,
        and SLAPorSLOP's fee isn't returned with a refund. Months are in UTC.
      </p>
      <VatNote vat={vat} onSaved={onVat} />
    </>
  );
}

/** What VAT means for SLAPorSLOP's fee here (LEGAL §3.5, TAX-4), with the VAT number. */
function VatNote({ vat, onSaved }: { vat: VatTreatment; onSaved: () => void }) {
  if (vat.kind === "none") {
    return (
      <p id="fee-statements" className="muted small">
        There's no VAT on SLAPorSLOP's fee where you are.
      </p>
    );
  }
  const rate = vat.rate_bp ?? 0;
  return (
    <div id="fee-statements">
      <p className="muted small">
        {vat.kind === "included"
          ? `SLAPorSLOP's fee includes ${(rate / 100).toFixed(rate % 100 ? 1 : 0)}% VAT, which SLAPorSLOP pays out of its 5%; it's never added on top.`
          : `Reverse charge: you account for the VAT on SLAPorSLOP's fee yourself (VAT number ${vat.vat_number}).`}{" "}
        {vat.slop_vat_id ? `SLAPorSLOP's VAT number: ${vat.slop_vat_id}.` : "SLAPorSLOP's VAT registration is still being set up."}
      </p>
      <VatNumber vat={vat} onSaved={onSaved} />
    </div>
  );
}

function VatNumber({ vat, onSaved }: { vat: VatTreatment; onSaved: () => void }) {
  const [value, setValue] = useState(vat.vat_number ?? "");
  const [note, setNote] = useState<string>();
  const [error, setError] = useState<string>();
  const save = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    setNote(undefined);
    try {
      const res = await api.donations.setVat(value.trim() || null);
      setNote(res.note ?? (res.checked ? "Confirmed. SLAPorSLOP's fee is reverse-charged from now on." : "Removed."));
      onSaved();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <form className="form inline-form" onSubmit={(e) => void save(e)}>
      <label>
        VAT number, if you're a VAT-registered business{" "}
        <input value={value} onChange={(e) => setValue(e.target.value)} placeholder={`${vat.country === "GR" ? "EL" : vat.country}123456789`} />
      </label>
      <button className="btn small">Save</button>
      {vat.vat_number && !vat.vat_number_checked && !note && (
        <p className="muted small">Not confirmed yet, so VAT is shown as included in SLAPorSLOP's fee.</p>
      )}
      {note && <p className="muted small">{note}</p>}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}
