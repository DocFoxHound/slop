// SPDX-License-Identifier: AGPL-3.0-or-later
// Admin → Apps: every app on one line each (icon, title and address, maker, stage, and what needs
// staff about it, or a quiet line when nothing does), found by search, filtered to the ones that
// need attention, sorted most urgent first, and loaded a page at a time so thousands stay quick.
// Each line's actions are in its "⋯" menu: the record, the store page, the stage, putting it in
// or taking it out of the store, and withdrawing a release. Taking out and withdrawing ask
// "Are you sure?" on the line, with the reason the maker or players will read. What the facts
// mean is in appFacts.ts; the API is crates/slop-api/src/admin_web.rs (apps).

import { useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";

import { api, type AdminApp, type AppStage } from "../../api";
import { AppIcon, appIconProps } from "../../components/AppIcon";
import { MenuButton, type MenuSection } from "../../components/MenuButton";
import { ErrorState, Loading } from "../../components/ui";
import { formatDate, timeAgo } from "../../format";
import { useLoad } from "../../hooks";
import {
  cantListReason,
  PAGE,
  pageQuery,
  readShow,
  readSort,
  rowFacts,
  SHOWS,
  SORTS,
  STAGE_LABEL,
  type Show,
  type Sort,
} from "./appFacts";
import { ActionMessages, DelistButton, ReasonButton, useAction } from "./shared";

export function AppsTab() {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const show = readShow(params.get("show"));
  const sort = readSort(params.get("sort"), show);
  const [text, setText] = useState(q);
  // How many pages are loaded, for the current filters only: new filters start from one.
  const filters = `${show}\n${sort}\n${q}`;
  const [loaded, setLoaded] = useState({ filters, pages: 1 });
  const pages = loaded.filters === filters ? loaded.pages : 1;
  const apps = useLoad(
    () =>
      Promise.all(Array.from({ length: pages }, (_, i) => api.admin.apps(pageQuery(show, sort, q, i)))).then((all) => ({
        rows: all.flat(),
        more: all[all.length - 1].length === PAGE,
      })),
    [show, sort, q, pages],
  );
  // The Content Policy rules, for the statement of reasons when taking an app out.
  const rules = useLoad(() => api.admin.queue().then((q) => q.rules), []);

  const change = (key: string, value: string) => {
    const next = new URLSearchParams(params);
    if (value) next.set(key, value);
    else next.delete(key);
    setParams(next, { replace: true });
  };

  return (
    <>
      <form
        className="filters"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          change("q", text.trim());
        }}
      >
        <label>
          Find <input value={text} placeholder="Title, address or maker" onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn small">Search</button>
        <label>
          Show{" "}
          <select value={show} onChange={(e) => change("show", e.target.value as Show)}>
            {SHOWS.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label>
          Sort{" "}
          <select value={sort} onChange={(e) => change("sort", e.target.value as Sort)}>
            {SORTS.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
      </form>
      {apps.error && <ErrorState error={apps.error} onRetry={apps.reload} />}
      {!apps.data && !apps.error && <Loading />}
      {apps.data?.rows.length === 0 && (
        <div className="empty-state">{show === "attention" ? "Nothing needs attention." : "No apps."}</div>
      )}
      {apps.data && apps.data.rows.length > 0 && (
        <ul className="apps-rows" aria-label="Apps">
          {apps.data.rows.map((a) => (
            <AppRow key={a.slug} app={a} rules={rules.data ?? []} onDone={apps.reload} />
          ))}
        </ul>
      )}
      {apps.data?.more && (
        <button
          className="btn small ghost apps-more"
          disabled={apps.loading}
          onClick={() => setLoaded({ filters, pages: pages + 1 })}
        >
          {apps.loading ? "Loading…" : "Show more"}
        </button>
      )}
    </>
  );
}

/** What's open under a line: "Are you sure?" for taking it out, or for withdrawing a release. */
type Asking = { kind: "delist" } | { kind: "withdraw"; id: string; version: string };

function AppRow({ app, rules, onDone }: { app: AdminApp; rules: Array<[string, string]>; onDone: () => void }) {
  const { busy, error, notice, run } = useAction(onDone);
  const [asking, setAsking] = useState<Asking>();
  const line = useRef<HTMLDivElement>(null);
  const disabled = busy !== undefined;
  const { shown, more } = rowFacts(app);
  const record = `/admin?tab=apps&app=${encodeURIComponent(app.slug)}`;

  /** Closes the question; focus goes back to the line's menu button. */
  const closeAsking = () => {
    setAsking(undefined);
    window.setTimeout(() => line.current?.querySelector<HTMLButtonElement>(".menu-button-btn")?.focus(), 0);
  };
  // Closed only when the action went through: a refusal keeps what was typed.
  const runThenClose = (key: string, action: () => Promise<unknown>) =>
    void run(key, async () => {
      const result = await action();
      closeAsking();
      return result;
    });

  const cantList = cantListReason(app);
  const sections: MenuSection[] = [
    {
      items: [
        { kind: "link", label: "Open its record", to: record },
        { kind: "link", label: "Store page", to: `/app/${app.slug}` },
      ],
    },
    {
      label: "Stage",
      items: (Object.keys(STAGE_LABEL) as AppStage[]).map((stage) => ({
        kind: "radio" as const,
        label: STAGE_LABEL[stage],
        checked: app.stage === stage,
        onSelect: () => {
          if (stage !== app.stage) void run("stage", () => api.admin.stage(app.slug, stage));
        },
      })),
    },
    {
      label: "In the store",
      items: [
        app.status === "listed"
          ? {
              label: "Take out of the store…",
              danger: true,
              keepsFocus: true,
              onSelect: () => setAsking({ kind: "delist" }),
            }
          : {
              label: "Put in the store",
              disabled: cantList ?? undefined,
              onSelect: () => void run("list", () => api.admin.list(app.slug)),
            },
        ...app.releases.map((r) => ({
          label: `Withdraw ${r.version}…`,
          danger: true,
          keepsFocus: true,
          onSelect: () => setAsking({ kind: "withdraw", id: r.id, version: r.version }),
        })),
      ],
    },
  ];

  return (
    <li className={`apps-row${asking ? " asking" : ""}`}>
      <div className="apps-line" ref={line}>
        <AppIcon {...appIconProps(app)} size={32} />
        <span className="apps-open">
          <Link to={record}>
            <b>{app.title}</b>
          </Link>{" "}
          <span className="muted small">{app.slug}</span>
        </span>
        <span className="apps-maker muted small">by {app.maker}</span>
        <span className="apps-stage small">
          <span className="muted">{STAGE_LABEL[app.stage]}</span>{" "}
          <span className={`badge ${app.status === "listed" ? "official" : app.status === "delisted" ? "bad" : "neutral"}`}>
            {app.status.toUpperCase()}
          </span>
          {app.visibility === "private" && <span className="badge neutral">PRIVATE</span>}
        </span>
        <span className="apps-facts small" title={[...shown, ...more].map((f) => f.text).join("\n")}>
          {shown.map((f, i) => (
            <span key={i} className={`apps-fact ${f.tone}`}>
              {f.tone !== "quiet" && (
                <span aria-hidden="true" className="apps-fact-dot">
                  ●
                </span>
              )}
              {f.text}
            </span>
          ))}
          {more.length > 0 && <span className="muted"> +{more.length} more</span>}
        </span>
        <time className="apps-when small muted" dateTime={app.updated_at} title={`Last changed ${formatDate(app.updated_at)}`}>
          {busy ? "…" : timeAgo(app.updated_at)}
          <span className="sr-only"> last changed</span>
        </time>
        <MenuButton label={`Actions for ${app.title}`} sections={sections} />
      </div>
      {(error || notice) && (
        <div className="apps-messages">
          <ActionMessages error={error} notice={notice} />
        </div>
      )}
      {asking?.kind === "delist" && (
        <div className="apps-confirm">
          <p>
            <b>Are you sure?</b> Take “{app.title}” out of the store. Its maker gets a statement of reasons.
          </p>
          <DelistButton
            rules={rules}
            disabled={disabled}
            startOpen
            onCancel={closeAsking}
            onSubmit={(rule, reason) => runThenClose("delist", () => api.admin.delist(app.slug, rule, reason))}
          />
        </div>
      )}
      {asking?.kind === "withdraw" && (
        <div className="apps-confirm">
          <p>
            <b>Are you sure?</b> Withdraw {asking.version} of “{app.title}”: it stops launching for everyone who has it.
          </p>
          <ReasonButton
            key={asking.id}
            label={`Withdraw ${asking.version}`}
            placeholder="Reason, shown to players (stops it launching)"
            danger
            disabled={disabled}
            startOpen
            onCancel={closeAsking}
            onSubmit={(reason) => runThenClose(asking.id, () => api.admin.revoke(asking.id, reason))}
          />
        </div>
      )}
    </li>
  );
}
