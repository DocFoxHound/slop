// SPDX-License-Identifier: AGPL-3.0-or-later
// Small SVG charts: a line over time and up/down bars per day. Each has a text summary for
// screen readers and a table behind a "Show numbers" toggle (A11Y-1: no chart-only information).

import { useId, useState, type ReactNode } from "react";

interface Series {
  name: string;
  /** null: no value at that point (a gap). */
  values: Array<number | null>;
  className: string;
}

const W = 600;
const H = 180;
const PAD = { l: 34, r: 8, t: 10, b: 24 };

function DataTable({ labels, columns }: { labels: string[]; columns: Array<{ name: string; values: Array<number | string | null> }> }) {
  return (
    <table className="audit chart-table">
      <thead>
        <tr>
          <th>When</th>
          {columns.map((c) => (
            <th key={c.name}>{c.name}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {labels.map((l, i) => (
          <tr key={l}>
            <td>{l}</td>
            {columns.map((c) => (
              <td key={c.name}>{c.values[i] ?? "—"}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Frame({ title, summary, legend, children, table }: { title: string; summary: string; legend: ReactNode; children: ReactNode; table: ReactNode }) {
  const [showTable, setShowTable] = useState(false);
  const id = useId();
  return (
    <figure className="chart">
      <figcaption>
        <b>{title}</b> <span className="chart-legend">{legend}</span>
      </figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-labelledby={id} preserveAspectRatio="none">
        <title id={id}>{summary}</title>
        {children}
      </svg>
      <button type="button" className="link-btn small" onClick={() => setShowTable((s) => !s)} aria-expanded={showTable}>
        {showTable ? "Hide numbers" : "Show numbers"}
      </button>
      {showTable && table}
    </figure>
  );
}

function xAt(i: number, n: number) {
  return PAD.l + (n <= 1 ? (W - PAD.l - PAD.r) / 2 : (i * (W - PAD.l - PAD.r)) / (n - 1));
}

function labelEvery(n: number) {
  return Math.max(1, Math.ceil(n / 8));
}

/** Lines over time, on a fixed 0–max scale. */
export function LineChart({
  title,
  summary,
  labels,
  series,
  max,
  unit = "",
}: {
  title: string;
  summary: string;
  labels: string[];
  series: Series[];
  max: number;
  unit?: string;
}) {
  const y = (v: number) => PAD.t + (1 - v / max) * (H - PAD.t - PAD.b);
  const every = labelEvery(labels.length);
  return (
    <Frame
      title={title}
      summary={summary}
      legend={series.map((s) => (
        <span key={s.name} className={`key ${s.className}`}>
          {s.name}
        </span>
      ))}
      table={<DataTable labels={labels} columns={series.map((s) => ({ name: s.name, values: s.values }))} />}
    >
      {[0, 0.5, 1].map((f) => (
        <g key={f}>
          <line className="grid" x1={PAD.l} x2={W - PAD.r} y1={y(max * f)} y2={y(max * f)} />
          <text className="axis" x={PAD.l - 4} y={y(max * f) + 4} textAnchor="end">
            {Math.round(max * f)}
            {unit}
          </text>
        </g>
      ))}
      {labels.map((l, i) =>
        i % every === 0 ? (
          <text key={l} className="axis" x={xAt(i, labels.length)} y={H - 6} textAnchor="middle">
            {l}
          </text>
        ) : null,
      )}
      {series.map((s) => {
        const pts = s.values.map((v, i) => (v === null ? null : `${xAt(i, labels.length)},${y(v)}`));
        // Break the line at gaps.
        const runs: string[][] = [[]];
        for (const p of pts) {
          if (p === null) runs.push([]);
          else runs[runs.length - 1].push(p);
        }
        return (
          <g key={s.name} className={s.className}>
            {runs.filter((r) => r.length > 1).map((r, k) => <polyline key={k} points={r.join(" ")} />)}
            {runs.filter((r) => r.length === 1).map((r, k) => {
              const [cx, cy] = r[0].split(",");
              return <circle key={`d${k}`} cx={cx} cy={cy} r={3} />;
            })}
          </g>
        );
      })}
    </Frame>
  );
}

/** Up (above the line) and down (below) bars per day; `marked` days are shaded. */
export function UpDownBars({
  title,
  summary,
  labels,
  up,
  down,
  upName,
  downName,
  marked,
  markedName = "Marked",
}: {
  title: string;
  summary: string;
  labels: string[];
  up: number[];
  down: number[];
  upName: string;
  downName: string;
  marked?: boolean[];
  markedName?: string;
}) {
  const anyMarked = Boolean(marked?.some(Boolean));
  const max = Math.max(1, ...up, ...down);
  const mid = PAD.t + (H - PAD.t - PAD.b) / 2;
  const half = (H - PAD.t - PAD.b) / 2;
  const bw = Math.max(1, (W - PAD.l - PAD.r) / Math.max(labels.length, 1) - 1);
  const every = labelEvery(labels.length);
  return (
    <Frame
      title={title}
      summary={summary}
      legend={
        <>
          <span className="key up">{upName}</span>
          <span className="key down">{downName}</span>
          {anyMarked && <span className="key marked">{markedName}</span>}
        </>
      }
      table={
        <DataTable
          labels={labels}
          columns={[
            { name: upName, values: up },
            { name: downName, values: down },
            ...(anyMarked ? [{ name: markedName, values: marked!.map((m) => (m ? "yes" : "")) }] : []),
          ]}
        />
      }
    >
      <line className="grid" x1={PAD.l} x2={W - PAD.r} y1={mid} y2={mid} />
      <text className="axis" x={PAD.l - 4} y={PAD.t + 8} textAnchor="end">
        {max}
      </text>
      <text className="axis" x={PAD.l - 4} y={H - PAD.b} textAnchor="end">
        {max}
      </text>
      {labels.map((l, i) => {
        const x = PAD.l + (i * (W - PAD.l - PAD.r)) / Math.max(labels.length, 1);
        return (
          <g key={l}>
            {marked?.[i] && <rect className="marked" x={x - 1} width={bw + 2} y={PAD.t} height={H - PAD.t - PAD.b} />}
            {up[i] > 0 && <rect className="up" x={x} width={bw} y={mid - (up[i] / max) * half} height={(up[i] / max) * half} />}
            {down[i] > 0 && <rect className="down" x={x} width={bw} y={mid} height={(down[i] / max) * half} />}
            {i % every === 0 && (
              <text className="axis" x={x + bw / 2} y={H - 6} textAnchor="middle">
                {l}
              </text>
            )}
          </g>
        );
      })}
    </Frame>
  );
}
