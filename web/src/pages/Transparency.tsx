// SPDX-License-Identifier: AGPL-3.0-or-later
// The yearly transparency report (LEGAL-GUIDANCE CHAT-4): requests from authorities, and the
// year's reports, decisions and appeals. Counts only.

import { useState } from "react";

import { api } from "../api";
import { useDocumentTitle, useLoad } from "../hooks";
import { ErrorState, Loading } from "../components/ui";

const LABEL: Record<string, string> = {
  subpoena: "Subpoenas",
  court_order: "Court orders",
  warrant: "Warrants",
  eu_production_order: "EU production orders",
  preservation: "Preservation requests",
  emergency: "Emergency requests",
  uk_daa: "UK requests",
  removal_order: "Terrorist content removal orders",
  other: "Other",
  open: "Not yet answered",
  answered: "Answered",
  partly_answered: "Partly answered",
  refused: "Refused",
  withdrawn: "Withdrawn",
  actioned: "Acted on",
  dismissed: "No action needed",
  delist_app: "App taken out of the store",
  hide_review: "Review hidden",
  hide_profile: "Profile hidden",
  suspend_account: "Account suspended",
  warn: "Warning",
  pending: "Waiting",
  upheld: "Decision kept",
  reversed: "Decision reversed",
};

function Table({ caption, rows }: { caption: string; rows: Array<{ key: string; count: number }> }) {
  return (
    <table className="audit">
      <caption>{caption}</caption>
      <thead>
        <tr>
          <th scope="col">What</th>
          <th scope="col">How many</th>
        </tr>
      </thead>
      <tbody>
        {rows.length === 0 ? (
          <tr>
            <td colSpan={2} className="muted">
              None
            </td>
          </tr>
        ) : (
          rows.map((r) => (
            <tr key={r.key}>
              <td>{LABEL[r.key] ?? r.key.replace(/_/g, " ")}</td>
              <td>{r.count}</td>
            </tr>
          ))
        )}
      </tbody>
    </table>
  );
}

export function Transparency() {
  useDocumentTitle("Transparency report");
  const thisYear = new Date().getUTCFullYear();
  const [year, setYear] = useState(thisYear);
  const report = useLoad(() => api.transparency(year), [year]);
  const years = Array.from({ length: thisYear - 2026 + 1 }, (_, i) => thisYear - i);
  return (
    <div className="page narrow doc">
      <h1>Transparency report</h1>
      <p>
        What authorities asked Slop for, and how Slop handled reports about content, each year. Counts only: never who.
        Slop can only give authorities what it has (account details, the metadata it keeps, reported messages): chat is
        end-to-end encrypted, and we tell people about requests unless a law or the order forbids it.
      </p>
      <label>
        Year{" "}
        <select value={year} onChange={(e) => setYear(Number(e.target.value))}>
          {years.map((y) => (
            <option key={y} value={y}>
              {y}
            </option>
          ))}
        </select>
      </label>
      {report.error && <ErrorState error={report.error} onRetry={report.reload} />}
      {!report.data ? (
        !report.error && <Loading />
      ) : (
        <>
          {!report.data.complete && <p className="muted small">{year} isn't over yet: these are the numbers so far.</p>}
          <h2>Requests from authorities</h2>
          <Table caption="By country" rows={report.data.legal_requests.by_country} />
          <Table caption="By kind" rows={report.data.legal_requests.by_kind} />
          <Table caption="By outcome" rows={report.data.legal_requests.by_outcome} />
          <h2>Reports about content</h2>
          <Table caption="By reason" rows={report.data.reports.by_reason} />
          <Table caption="By outcome" rows={report.data.reports.by_outcome} />
          {report.data.reports.median_hours_to_decide !== null && (
            <p>Half of reports were decided within {report.data.reports.median_hours_to_decide} hours.</p>
          )}
          <h2>Decisions and appeals</h2>
          <Table caption="Decisions by action" rows={report.data.decisions.by_action} />
          <p>
            Decisions made by automated tools alone: {report.data.decisions.automated}. A person makes every decision on
            Slop.
          </p>
          <Table caption="Appeals by outcome" rows={report.data.appeals.by_outcome} />
        </>
      )}
    </div>
  );
}
