// SPDX-License-Identifier: AGPL-3.0-or-later
// The release date on the store page editor (Publish.tsx): for a Coming soon or Beta app, when its
// maker expects to release it, to the day or only the month (catalog.rs `parse_release_date`).
// Players see it on tiles, the store page and the front page's Releasing soon shelf; once it has
// passed they don't, and its maker is asked to update it here.

import type { AppSummary, ReleasePrecision } from "../api";
import { releaseDateText } from "../format";

export interface ReleaseDateValue {
  /** YYYY-MM-DD, or "" for none. With precision "month", only its month counts. */
  date: string;
  precision: ReleasePrecision;
}

/** The editor's value for an app: its date (the maker sees it even once passed) and precision. */
export function releaseDateValue(app?: Pick<AppSummary, "release_date" | "release_date_precision">): ReleaseDateValue {
  return { date: app?.release_date ?? "", precision: app?.release_date_precision ?? "day" };
}

/** A month's first day, as the API keeps a month ("2026-12-17" → "2026-12-01"). */
function monthStart(date: string): string {
  return /^\d{4}-\d{2}/.test(date) ? `${date.slice(0, 7)}-01` : date;
}

/**
 * What to send to save `value` over the app's saved date: nothing when it's the same (so saving
 * other fields never trips over a date that has passed), `null` to clear it, or the new date and
 * precision. A released app has no date to send. Exported for tests.
 */
export function releaseDateChanges(
  value: ReleaseDateValue,
  saved: Pick<AppSummary, "release_date" | "release_date_precision" | "stage"> | undefined,
): { release_date?: string | null; release_date_precision?: ReleasePrecision } {
  const date = value.date.trim();
  if (saved?.stage === "released") return {};
  const next = date ? (value.precision === "month" ? monthStart(date) : date) : "";
  const before = saved?.release_date ?? "";
  if (next === before && (!next || value.precision === (saved?.release_date_precision ?? "day"))) return {};
  if (!next) return saved ? { release_date: null } : {};
  return { release_date: next, release_date_precision: value.precision };
}

/** Today in UTC (YYYY-MM-DD): release dates are calendar days, checked in UTC. */
function todayUtc(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}

export function ReleaseDateField({
  value,
  onChange,
  passed,
  error,
  now = Date.now(),
}: {
  value: ReleaseDateValue;
  onChange: (value: ReleaseDateValue) => void;
  /** The saved date has passed (the maker's own view says so), and it hasn't been changed yet. */
  passed: boolean;
  /** The API's message for release_date or release_date_precision. */
  error?: string;
  now?: number;
}) {
  const today = todayUtc(now);
  const max = todayUtc(now + (3 * 365 + 1) * 86_400_000);
  const preview = value.date
    ? releaseDateText({ release_date: value.precision === "month" ? monthStart(value.date) : value.date, release_date_precision: value.precision }, now)
    : null;
  return (
    <fieldset className="visibility-choice release-date-field">
      <legend>
        Release date <span className="muted small">(optional)</span>
      </legend>
      <label>
        When you expect to release it
        <input
          type="date"
          value={value.date}
          min={value.precision === "month" ? `${today.slice(0, 7)}-01` : today}
          max={max}
          aria-describedby="release-date-hint"
          onChange={(e) => onChange({ ...value, date: e.target.value })}
        />
      </label>
      <div className="choice-list inline" role="radiogroup" aria-label="How exact the date is">
        <label className="check">
          <input
            type="radio"
            name="release-precision"
            checked={value.precision === "day"}
            onChange={() => onChange({ ...value, precision: "day" })}
          />
          <span>Exact day</span>
        </label>
        <label className="check">
          <input
            type="radio"
            name="release-precision"
            checked={value.precision === "month"}
            onChange={() => onChange({ ...value, precision: "month" })}
          />
          <span>Month only</span>
        </label>
        {value.date && (
          <button type="button" className="btn small ghost" onClick={() => onChange({ ...value, date: "" })}>
            Clear the date
          </button>
        )}
      </div>
      <span className="hint" id="release-date-hint">
        {preview ? `Players see “${preview}”` : "No date: players see none"} on its tiles and store page, and on the store's
        Releasing soon shelf in the 60 days before. Once it has passed they don't, and releasing it clears it.
      </span>
      {passed && (
        <p className="field-error" role="alert">
          The date you gave has passed, so players no longer see it: give a new date, or clear it.
        </p>
      )}
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
    </fieldset>
  );
}
