// SPDX-License-Identifier: AGPL-3.0-or-later
// History for makers, on Manage: for each UTC day, how many accounts played the app, how many
// played it for the first time and how many times it was added to a library, with the day's
// busiest moment from Slop Pulse (crates/slop-api/src/history.rs; DESIGN §11, METRICS §3).
// Totals only, never who. A day that wasn't counted is null, not zero: a gap in the line and a
// dash in the numbers.

import { useState } from "react";

import { api, type AppHistory } from "../api";
import { useLoad } from "../hooks";
import { LineChart } from "./Charts";
import { ErrorState, Loading } from "./ui";

type Range = 30 | 90 | 365;

const RANGES: Array<[Range, string]> = [
  [30, "30 days"],
  [90, "90 days"],
  [365, "A year"],
];

/** The days that were counted. */
function counted(values: Array<number | null>): number[] {
  return values.filter((v): v is number => v !== null);
}

function sum(values: number[]): number {
  return values.reduce((n, v) => n + v, 0);
}

function plural(n: number, word: string): string {
  return `${n.toLocaleString()} ${word}${n === 1 ? "" : "s"}`;
}

export function HistoryPanel({ slug }: { slug: string }) {
  const [days, setDays] = useState<Range>(30);
  // The last answer stays up while another range loads, so the charts and an open table keep
  // their place.
  const h = useLoad(() => api.history(slug, days), [slug, days]);
  return (
    <section className="panel panel-pad" aria-labelledby="history-title" aria-busy={h.loading}>
      <h2 id="history-title">History</h2>
      <p className="muted small">
        Each day's totals for this app: how many accounts played it, how many played it for the first time, and how many times it was
        added to a library. Totals only, never who. Days are UTC days. Your own plays and adds aren't counted, and neither are plays by
        people who switched off "Share my playtime".
      </p>
      <div className="seg" role="group" aria-label="Days shown">
        {RANGES.map(([n, label]) => (
          <button key={n} type="button" className={days === n ? "on" : ""} aria-pressed={days === n} onClick={() => setDays(n)}>
            {label}
          </button>
        ))}
      </div>
      {h.error && <ErrorState error={h.error} onRetry={h.reload} />}
      {!h.data && !h.error && <Loading />}
      {h.data && <Numbers history={h.data} />}
    </section>
  );
}

function Numbers({ history }: { history: AppHistory }) {
  const { days, history_from, players_from } = history;
  const players = days.map((d) => d.players);
  const firstPlays = days.map((d) => d.first_plays);
  const libraryAdds = days.map((d) => d.library_adds);
  const peaks = days.map((d) => d.peak);
  // YYYY-MM-DD as the server gave it: formatDate would print the local date, which is the day
  // before in the Americas.
  const first = days[0]?.day ?? "";
  const last = days[days.length - 1]?.day ?? "";
  // MM-DD for 30 or 90 days; a year's view gives the year too, so a day like 01-05 isn't left
  // to guess.
  const labels = days.map((d) => (days.length > 90 ? d.day : d.day.slice(5)));
  const fp = sum(counted(firstPlays));
  const la = sum(counted(libraryAdds));
  const maxPlayers = Math.max(0, ...counted(players));
  const maxPeak = Math.max(0, ...counted(peaks));
  const empty = [players, firstPlays, libraryAdds, peaks].every((values) => counted(values).every((v) => v === 0));
  return (
    <>
      {empty ? (
        <p className="muted history-numbers">Nothing to show yet: no plays or library adds were counted in these days.</p>
      ) : (
        <div className="history-numbers">
          <p>
            In the days shown: {plural(fp, "first play")} and {plural(la, "library add")}.
            <span className="muted small"> Players per day aren't added up: an account that plays on three days counts on each.</span>
          </p>
          <LineChart
            title="Players and first plays per day"
            summary={`Accounts that played each day from ${first} to ${last} (UTC days). The most on one day was ${maxPlayers}. First plays in that time: ${fp}.`}
            labels={labels}
            max={Math.max(5, maxPlayers, ...counted(firstPlays))}
            series={[
              { name: "Players", values: players, className: "line-players" },
              { name: "First plays", values: firstPlays, className: "line-first" },
            ]}
          />
          <LineChart
            title="Library adds per day"
            summary={`Times it was added to a library each day from ${first} to ${last} (UTC days): ${la} in all.`}
            labels={labels}
            max={Math.max(5, ...counted(libraryAdds))}
            series={[{ name: "Library adds", values: libraryAdds, className: "line-main" }]}
          />
          {maxPeak > 0 && (
            <>
              <LineChart
                title="Players, busiest moment each day"
                summary={`The most players at once each day from ${first} to ${last}. The highest was ${maxPeak}.`}
                labels={labels}
                max={Math.max(5, maxPeak)}
                series={[{ name: "Players at once", values: peaks, className: "line-players" }]}
              />
              <p className="muted small">From Slop Pulse: it counts everyone who shares their playtime, you too if you do.</p>
            </>
          )}
        </div>
      )}
      <p className="muted small">
        SLAPorSLOP doesn't count installs. First plays are the closest number: each account's first time starting the app from SLAPorSLOP.
      </p>
      <p className="muted small">
        Library adds are the times it was added to a library: an account that removes it and adds it again on a later day counts again.
      </p>
      {players_from > first && (
        <p className="muted small">
          Players per day are counted from {players_from}, and that first day is a part day.
          {!empty && " A dash in the numbers means not counted, which isn't zero."}
        </p>
      )}
      {/* Only while a back-filled day is shown. */}
      {players_from > first && players_from > history_from && (
        <p className="muted small">
          First plays and library adds before {players_from} were worked out from what was still on SLAPorSLOP that day: accounts closed
          before then, and library entries removed before then, aren't in them.
        </p>
      )}
      {history_from > first && (
        <p className="muted small">Nothing is counted before {history_from}: the day the app was made, or the day it became public.</p>
      )}
    </>
  );
}
