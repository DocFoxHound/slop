// SPDX-License-Identifier: AGPL-3.0-or-later
// Publish → Stats: numbers about every app the maker makes, drafts included, and their account
// overall (crates/slop-api/src/maker_stats.rs, DESIGN §8). Counts and totals only, never who.
// Library adds and first plays are the History counts (MakerHistory.tsx): UTC days, not for a
// private app or a dedicated server, and never the maker's own.

import type { ReactNode } from "react";
import { Link } from "react-router";

import { api, type AppStats, type MakerStats as Stats, type StatsMoney } from "../api";
import { PublishTabs } from "../components/PublishTabs";
import { ErrorState, Loading } from "../components/ui";
import { formatDay, money, STAGE_LABEL } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { SignInFirst } from "./Publish";

const STATUS_LABEL: Record<AppStats["status"], string> = {
  draft: "Draft",
  listed: "In the store",
  delisted: "Out of the store",
};

function n(value: number): string {
  return value.toLocaleString();
}

function plural(count: number, one: string, many = `${one}s`): string {
  return `${n(count)} ${count === 1 ? one : many}`;
}

/** "$12.50 from 3 donations · €7" — each currency's total, none when empty. */
export function moneyLine(rows: StatsMoney[]): string {
  return rows.map((m) => `${money(m.donated_minor, m.currency)} from ${plural(m.donations, "donation")}`).join(" · ");
}

/** "3 released · 1 in Beta" — the parts that aren't zero. */
function parts(list: Array<[number, string]>): string {
  return list
    .filter(([count]) => count > 0)
    .map(([count, label]) => `${n(count)} ${label}`)
    .join(" · ");
}

/** The text alternative of an app's 30-day sparkline. Exported for tests. */
export function sparkSummary(values: Array<number | null>, days: string[]): string {
  const counted = values.map((v, i) => [v, days[i]] as const).filter((p): p is readonly [number, string] => p[0] !== null);
  if (counted.length === 0) return "Library adds per day: nothing counted in the last 30 days.";
  const total = counted.reduce((s, [v]) => s + v, 0);
  const since = counted.length < values.length ? ` (counted from ${formatDay(counted[0][1])})` : "";
  if (total === 0) return `Library adds per day, last 30 days${since}: none.`;
  const [most, on] = counted.reduce((best, p) => (p[0] > best[0] ? p : best));
  return `Library adds per day, last 30 days${since}: ${n(total)} in all; the most on one day was ${n(most)}, on ${formatDay(on)}.`;
}

const SW = 96;
const SH = 26;

/** A tiny line of the last 30 days, with its numbers as its text alternative and on hover. */
export function Sparkline({ values, days }: { values: Array<number | null>; days: string[] }) {
  const max = Math.max(1, ...values.filter((v): v is number => v !== null));
  const x = (i: number) => 2 + (i * (SW - 4)) / Math.max(values.length - 1, 1);
  const y = (v: number) => SH - 3 - (v / max) * (SH - 6);
  // Broken where a day wasn't counted.
  const runs: string[][] = [[]];
  values.forEach((v, i) => {
    if (v === null) runs.push([]);
    else runs[runs.length - 1].push(`${x(i).toFixed(1)},${y(v).toFixed(1)}`);
  });
  const slot = (SW - 4) / Math.max(values.length - 1, 1);
  return (
    <svg className="spark" viewBox={`0 0 ${SW} ${SH}`} width={SW} height={SH} role="img" aria-label={sparkSummary(values, days)}>
      <line className="spark-base" x1={2} x2={SW - 2} y1={SH - 3} y2={SH - 3} />
      {runs
        .filter((r) => r.length > 1)
        .map((r, k) => (
          <polyline key={k} points={r.join(" ")} />
        ))}
      {runs
        .filter((r) => r.length === 1)
        .map((r, k) => {
          const [cx, cy] = r[0].split(",");
          return <circle key={`d${k}`} cx={cx} cy={cy} r={2} />;
        })}
      {values.map((v, i) => (
        <rect key={i} className="spark-hit" x={x(i) - slot / 2} y={0} width={slot} height={SH}>
          <title>{`${formatDay(days[i] ?? "")}: ${v === null ? "not counted" : plural(v, "library add")}`}</title>
        </rect>
      ))}
    </svg>
  );
}

export function MakerStats() {
  useDocumentTitle("Stats");
  const { me } = useSession();
  const stats = useLoad(() => (me ? api.makerStats() : Promise.resolve(undefined)), [me?.id]);

  if (me === undefined) return <Loading />;
  if (!me) return <SignInFirst what="/publish/stats" />;

  return (
    <div className="page">
      <h1 className="page-title">Stats</h1>
      <PublishTabs on="stats" />
      <p className="muted">
        Numbers for every app you make, drafts included, and your account overall. Totals only, never who. Days are UTC days, and your
        own plays and library adds aren't counted.
      </p>
      {stats.error && <ErrorState error={stats.error} onRetry={stats.reload} />}
      {!stats.data && !stats.error && <Loading />}
      {stats.data && stats.data.apps.length === 0 && (
        <div className="empty-state">
          No stats yet: you haven't made anything. <Link to="/publish/new">Create your first draft</Link>.
        </div>
      )}
      {stats.data && stats.data.apps.length > 0 && <StatsView stats={stats.data} />}
    </div>
  );
}

function StatsView({ stats }: { stats: Stats }) {
  const a = stats.account;
  const c = a.counts;
  const hasMoney = a.donations.length > 0;
  return (
    <>
      {stats.simulated && hasMoney && (
        <p className="notice" role="note">
          This server simulates payments: the donations here are test payments, not real money.
        </p>
      )}
      <section aria-labelledby="stats-account">
        <h2 id="stats-account" className="stats-h">
          Your account
        </h2>
        <dl className="stat-cards">
          <Card label="Apps" value={n(a.apps)}>
            {parts([
              [c.released, "released"],
              [c.beta, "in Beta"],
              [c.coming_soon, "coming soon"],
            ])}
            <br />
            {parts([
              [c.listed, "in the store"],
              [c.draft, c.draft === 1 ? "draft" : "drafts"],
              [c.delisted, "out of the store"],
              [c.private, "private"],
            ])}
          </Card>
          <Card label="Library adds, 30 days" value={n(a.library_adds_30d)}>
            {plural(a.library_adds, "add")} since counting began
          </Card>
          <Card label="In libraries now" value={n(a.in_libraries)} />
          <Card label="First plays, 30 days" value={n(a.first_plays_30d)}>
            Accounts starting an app for the first time
          </Card>
          <Card label="Slaps" value={a.not_slop_percent === null ? "—" : `${a.not_slop_percent}%`}>
            {plural(a.not_slop, "Slaps", "Slaps")} · {plural(a.slop, "Slop", "Slop")}
            {a.not_slop_percent === null && (
              <>
                <br />
                Not enough votes for a % yet
              </>
            )}
          </Card>
          <Card label="Interested" value={n(a.interested)}>
            Marks on your Coming soon and Beta apps
          </Card>
          <Card label="Releases published" value={n(a.releases_published)} />
          <Card label="App of the Month" value={n(a.awards)}>
            {a.awards === 0 ? "Not yet" : <Link to="/awards">See the awards</Link>}
          </Card>
          <Card label="Donations" value={hasMoney ? a.donations.map((m) => money(m.donated_minor, m.currency)).join(" · ") : "None yet"}>
            {hasMoney && (
              <>
                {plural(
                  a.donations.reduce((s, m) => s + m.donations, 0),
                  "donation",
                )}{" "}
                in all
                <br />
                This year: {a.donations_this_year.length > 0 ? moneyLine(a.donations_this_year) : "none"}
              </>
            )}
          </Card>
          <Card
            label="Busiest app, 30 days"
            value={a.busiest ? <Link to={`/publish/${a.busiest.slug}/manage`}>{a.busiest.title}</Link> : "—"}
          >
            {a.busiest
              ? [
                  plural(a.busiest.library_adds_30d, "library add"),
                  a.busiest.peak_players_30d ? `up to ${n(a.busiest.peak_players_30d)} playing at once` : null,
                ]
                  .filter(Boolean)
                  .join(" · ")
              : "Nothing counted in the last 30 days"}
          </Card>
        </dl>
      </section>

      <section aria-labelledby="stats-apps">
        <h2 id="stats-apps" className="stats-h">
          Each app
        </h2>
        <div className="table-scroll">
          <table className="table small stats-table">
            <caption>
              Your apps, newest changed first. Library adds, first plays and players at once are for the last 30 UTC days unless it
              says otherwise.
            </caption>
            <thead>
              <tr>
                <th scope="col">App</th>
                <th scope="col">Library adds</th>
                <th scope="col">In libraries</th>
                <th scope="col">First plays</th>
                <th scope="col">Most at once</th>
                <th scope="col">Slaps</th>
                <th scope="col">Interested</th>
                <th scope="col">Reviews</th>
                <th scope="col">Releases</th>
                <th scope="col">Open bounties</th>
                <th scope="col">Donations</th>
                <th scope="col">App of the Month</th>
              </tr>
            </thead>
            <tbody>
              {stats.apps.map((app) => (
                <AppRow key={app.slug} app={app} days={stats.days} />
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <p className="muted small">
        Library adds are the times an app was added to a library, counted from the day it was made or became public; an account that
        removes it and adds it again on a later day counts again. First plays are each account's first time starting it from SLAPorSLOP
        (SLAPorSLOP doesn't count installs). Players at once come from Slop Pulse, for games that use it. The Slaps % is weighted by
        playtime and shows from 5 votes. Donations are what donors gave you before SLAPorSLOP's fee, refunds included; each app's
        Manage page has more.
      </p>
    </>
  );
}

function Card({ label, value, children }: { label: string; value: ReactNode; children?: ReactNode }) {
  return (
    <div className="panel stat-card">
      <dt className="muted small">{label}</dt>
      <dd className="stat-value">{value}</dd>
      {children && <dd className="muted small">{children}</dd>}
    </div>
  );
}

const NOT_COUNTED = (
  <span className="muted" title="Not counted for private apps and dedicated servers">
    Not counted
  </span>
);

function AppRow({ app, days }: { app: AppStats; days: string[] }) {
  const what = [
    app.server_for ? `Dedicated server for ${app.server_for}` : STAGE_LABEL[app.stage],
    STATUS_LABEL[app.status],
    app.private ? "Private" : null,
    app.branch_of ? `Branch of ${app.branch_of}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const donated = app.donations.length > 0 ? moneyLine(app.donations) : "—";
  return (
    <tr>
      <th scope="row">
        <Link to={`/publish/${app.slug}/manage`}>{app.title}</Link>
        <div className="muted">{what}</div>
      </th>
      <td>
        {app.counted ? (
          <div className="spark-cell">
            <Sparkline values={app.daily_library_adds} days={days} />
            <div>
              <b>{n(app.library_adds_30d ?? 0)}</b>
              <div className="muted">
                {n(app.library_adds_7d ?? 0)} in 7 days · {n(app.library_adds ?? 0)} in all
              </div>
            </div>
          </div>
        ) : (
          NOT_COUNTED
        )}
      </td>
      <td>{n(app.in_libraries)}</td>
      <td>{app.counted ? n(app.first_plays_30d ?? 0) : NOT_COUNTED}</td>
      <td>{app.peak_players_30d === null ? "—" : n(app.peak_players_30d)}</td>
      <td>
        {app.not_slop_percent === null ? "—" : `${app.not_slop_percent}%`}
        <div className="muted">
          {n(app.not_slop)} Slaps · {n(app.slop)} Slop
        </div>
      </td>
      <td>{n(app.interested)}</td>
      <td>{n(app.reviews)}</td>
      <td>{n(app.releases_published)}</td>
      <td>{n(app.open_bounties)}</td>
      <td>{donated}</td>
      <td>{app.awards.length === 0 ? "—" : app.awards.map((w) => `${monthLabel(w.month)} (#${w.position})`).join(", ")}</td>
    </tr>
  );
}

/** "2026-10" → "Oct 2026". */
function monthLabel(month: string): string {
  return new Date(`${month}-01T00:00:00Z`).toLocaleDateString(undefined, { month: "short", year: "numeric", timeZone: "UTC" });
}
