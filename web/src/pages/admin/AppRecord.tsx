// SPDX-License-Identifier: AGPL-3.0-or-later
// One app's whole record for staff (crates/slop-api/src/admin_console.rs): its store page (and
// fixing it), where it is in the store, its money and any donation pause, reports, decisions,
// releases and everything staff did to it.

import { useState, type FormEvent } from "react";
import { Link } from "react-router";

import { api, type AdminAppRecord, type AppStage } from "../../api";
import { ErrorState, ExternalLink, Loading } from "../../components/ui";
import { formatDate, money, STAGE_LABEL } from "../../format";
import { useLoad } from "../../hooks";
import { HoldCard, HoldForm } from "./DonationHolds";
import { ActionMessages, DelistButton, ReasonButton, useAction } from "./shared";

const AGES = [0, 3, 7, 12, 16, 18];

export function AppRecord({ slug }: { slug: string }) {
  const record = useLoad(() => api.admin.appRecord(slug), [slug]);
  const holds = useLoad(() => api.admin.donationHolds("all"), [slug]);
  const rules = useLoad(() => api.admin.queue().then((q) => q.rules), []);
  const reload = () => {
    record.reload();
    holds.reload();
  };
  const action = useAction(reload);
  if (record.error) return <ErrorState error={record.error} onRetry={record.reload} />;
  if (!record.data) return <Loading />;
  const { app } = record.data;
  const appHolds = (holds.data ?? []).filter((h) => h.slug === slug);
  const open = appHolds.find((h) => !h.lifted_at);
  const approved = record.data.releases.filter((r) => r.status === "approved");

  return (
    <div className="admin-record">
      <p>
        <Link to="/admin?tab=apps">← All apps</Link>
      </p>
      <div className="title-row spread">
        <div className="title-row">
          <h2>{app.title}</h2>
          <span className={`badge ${app.status === "listed" ? "official" : app.status === "delisted" ? "bad" : "neutral"}`}>
            {app.status.toUpperCase()}
          </span>
          {app.visibility !== "public" && <span className="badge neutral">PRIVATE</span>}
          {app.frozen_at && <span className="badge bad">FROZEN</span>}
          {open && <span className="badge bad">DONATIONS PAUSED</span>}
        </div>
        <span className="muted small">
          <Link to={`/app/${app.slug}`}>Store page</Link> · <ExternalLink href={app.repo_url}>Repository</ExternalLink>
        </span>
      </div>
      <p className="muted">
        By <Link to={`/admin?tab=users&user=${app.maker_id}`}>{app.maker}</Link> · <code>{app.slug}</code> ·{" "}
        {app.license_spdx} · added {formatDate(app.created_at)}
        {app.parent && (
          <>
            {" "}
            · a branch of <Link to={`/admin?tab=apps&app=${encodeURIComponent(app.parent)}`}>{app.parent}</Link>
          </>
        )}
        {app.frozen_at && (
          <>
            {" "}
            · frozen since {formatDate(app.frozen_at)}
            {app.frozen_by ? <> by staff ({app.frozen_reason})</> : <> (Slop lost access to its repository)</>}
          </>
        )}
      </p>

      <section className="panel panel-pad">
        <h3>In the store</h3>
        <div className="row-actions">
          <label className="small">
            Stage{" "}
            <select
              value={app.stage}
              disabled={action.busy !== undefined}
              onChange={(e) => void action.run("stage", () => api.admin.stage(app.slug, e.target.value as AppStage))}
            >
              {(["coming_soon", "beta", "released"] as const).map((s) => (
                <option key={s} value={s}>
                  {STAGE_LABEL[s]}
                </option>
              ))}
            </select>
          </label>
          {app.status === "listed" ? (
            <DelistButton
              rules={rules.data ?? []}
              disabled={action.busy !== undefined}
              onSubmit={(rule, reason) => void action.run("delist", () => api.admin.delist(app.slug, rule, reason))}
            />
          ) : (
            <button
              className="btn small"
              disabled={action.busy !== undefined}
              onClick={() => void action.run("list", () => api.admin.list(app.slug))}
            >
              Put in the store
            </button>
          )}
          {app.frozen_by ? (
            <ReasonButton
              label="Lift the freeze"
              placeholder="For the maker and the record: what you found"
              disabled={action.busy !== undefined}
              onSubmit={(note) => void action.run("unfreeze", () => api.admin.unfreeze(app.slug, note))}
            />
          ) : (
            <>
              <ReasonButton
                label="Freeze"
                placeholder="For the maker: what's wrong and what would fix it (no new releases until you lift it)"
                danger
                disabled={action.busy !== undefined}
                onSubmit={(reason) => void action.run("freeze", () => api.admin.freeze(app.slug, reason))}
              />
              {app.frozen_at && (
                <ReasonButton
                  label="Check the repository now"
                  placeholder="For the record: why check now"
                  disabled={action.busy !== undefined}
                  onSubmit={(note) => void action.run("recheck", () => api.admin.unfreeze(app.slug, note))}
                />
              )}
            </>
          )}
          {approved.slice(0, 5).map((r) => (
            <ReasonButton
              key={r.id}
              label={`Withdraw ${r.version}`}
              placeholder="Reason, shown to players (stops it launching)"
              danger
              disabled={action.busy !== undefined}
              onSubmit={(reason) => void action.run(r.id, () => api.admin.revoke(r.id, reason))}
            />
          ))}
        </div>
        <p className="muted small">
          Freezing stops new releases while you look into something; the app stays in the store and installed copies
          keep working. The daily repository check never lifts a staff freeze.
        </p>
        <ActionMessages {...action} />
      </section>

      <ListingEditor record={record.data} onSaved={record.reload} />

      <section className="panel panel-pad">
        <h3>Donations</h3>
        <p>
          {app.donations_on ? "The maker takes donations" : "The maker hasn't turned donations on"}
          {app.donation_account && <> · Stripe account {app.donation_account}</>} ·{" "}
          {record.data.active_server_donations} monthly server donations
        </p>
        {record.data.donations.length > 0 && (
          <table className="audit">
            <thead>
              <tr>
                <th>Currency</th>
                <th>Donations</th>
                <th>Donated</th>
                <th>Slop's fees</th>
                <th>Refunded</th>
              </tr>
            </thead>
            <tbody>
              {record.data.donations.map((d) => (
                <tr key={d.currency}>
                  <td>{d.currency.toUpperCase()}</td>
                  <td>{d.donations}</td>
                  <td>{money(d.donated_minor, d.currency)}</td>
                  <td>{money(d.slop_fee_minor, d.currency)}</td>
                  <td>{money(d.refunded_minor, d.currency)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {appHolds.map((h) => (
          <HoldCard key={h.id} hold={h} onDone={reload} />
        ))}
        {!open && <HoldForm slug={app.slug} onDone={reload} />}
      </section>

      <section className="panel panel-pad">
        <h3>Reports ({record.data.reports.length})</h3>
        {record.data.reports.length === 0 ? (
          <p className="muted">None.</p>
        ) : (
          <ul className="plain">
            {record.data.reports.map((r) => (
              <li key={r.id}>
                <b>{r.reference}</b> · {r.reason} · <span className="badge neutral">{r.status.toUpperCase()}</span> ·{" "}
                {formatDate(r.created_at)}
                <p className="muted small pre-wrap">{r.details}</p>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel panel-pad">
        <h3>Decisions ({record.data.decisions.length})</h3>
        {record.data.decisions.length === 0 ? (
          <p className="muted">None.</p>
        ) : (
          <ul className="plain">
            {record.data.decisions.map((d) => (
              <li key={d.id}>
                <b>{d.reference}</b> · {d.action} under {d.rule} by {d.decided_by}, {formatDate(d.created_at)}
                {d.reversed_at && <> · reversed {formatDate(d.reversed_at)}</>}
                {d.appeal_status && <> · appeal {d.appeal_status}</>}
                <p className="muted small pre-wrap">{d.facts}</p>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel panel-pad">
        <h3>Releases</h3>
        {record.data.releases.length === 0 ? (
          <p className="muted">None yet.</p>
        ) : (
          <table className="audit">
            <thead>
              <tr>
                <th>Version</th>
                <th>Status</th>
                <th>Submitted</th>
                <th>Reviewed</th>
              </tr>
            </thead>
            <tbody>
              {record.data.releases.map((r) => (
                <tr key={r.id}>
                  <td>{r.version}</td>
                  <td>{r.status}</td>
                  <td>{formatDate(r.created_at)}</td>
                  <td>{r.reviewed_at ? formatDate(r.reviewed_at) : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section className="panel panel-pad">
        <h3>History</h3>
        {record.data.audit.length === 0 ? (
          <p className="muted">Nothing recorded.</p>
        ) : (
          <AuditRows rows={record.data.audit} />
        )}
      </section>
    </div>
  );
}

/** Audit entries as a table, details summarised. */
export function AuditRows({
  rows,
}: {
  rows: Array<{ id: number; at: string; actor: string; action: string; subject?: string; detail: Record<string, unknown> }>;
}) {
  return (
    <table className="audit">
      <thead>
        <tr>
          <th>When</th>
          <th>Who</th>
          <th>What</th>
          <th>Details</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((e) => (
          <tr key={e.id}>
            <td>{new Date(e.at).toLocaleString()}</td>
            <td>{e.actor}</td>
            <td>
              <code>{e.action}</code> {e.subject}
            </td>
            <td className="muted small">
              {Object.entries(e.detail)
                .map(([k, v]) => `${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`)
                .join(" · ")}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Staff fix the store page directly. Only changed fields are sent; the maker is told why. */
function ListingEditor({ record, onSaved }: { record: AdminAppRecord; onSaved: () => void }) {
  const { app } = record;
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState(app.title);
  const [summary, setSummary] = useState(app.summary);
  const [description, setDescription] = useState(app.description_md);
  const [minAge, setMinAge] = useState(app.min_age);
  const [labels, setLabels] = useState(app.content_labels.join(", "));
  const [reason, setReason] = useState("");
  const [saved, setSaved] = useState<string>();
  const action = useAction(() => {
    setOpen(false);
    setReason("");
    onSaved();
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const newLabels = labels
      .split(",")
      .map((l) => l.trim())
      .filter(Boolean);
    void action.run("edit", async () => {
      const res = await api.admin.editListing(app.slug, {
        title: title !== app.title ? title : undefined,
        summary: summary !== app.summary ? summary : undefined,
        description_md: description !== app.description_md ? description : undefined,
        min_age: minAge !== app.min_age ? minAge : undefined,
        content_labels: newLabels.join(",") !== app.content_labels.join(",") ? newLabels : undefined,
        reason: reason.trim(),
      });
      setSaved(`Changed: ${res.changed.join(", ")}. The maker has been told.`);
    });
  };
  return (
    <section className="panel panel-pad">
      <div className="title-row spread">
        <h3>Store page</h3>
        {!open && (
          <button className="btn small ghost" onClick={() => setOpen(true)}>
            Fix the store page
          </button>
        )}
      </div>
      {saved && (
        <p className="notice" role="status">
          {saved}
        </p>
      )}
      {!open ? (
        <>
          <p>
            <b>{app.title}</b>: {app.summary}
          </p>
          <p className="muted small">
            Age {app.min_age}+ · labels: {app.content_labels.join(", ") || "none"} · platforms:{" "}
            {app.platforms.join(", ") || "none"}
          </p>
        </>
      ) : (
        <form className="form" onSubmit={submit}>
          <label>
            Title
            <input value={title} maxLength={80} required onChange={(e) => setTitle(e.target.value)} />
          </label>
          <label>
            Summary
            <input value={summary} maxLength={200} required onChange={(e) => setSummary(e.target.value)} />
          </label>
          <label>
            Description (Markdown)
            <textarea value={description} rows={8} maxLength={20000} onChange={(e) => setDescription(e.target.value)} />
          </label>
          <label>
            Age rating
            <select value={minAge} onChange={(e) => setMinAge(Number(e.target.value))}>
              {AGES.map((a) => (
                <option key={a} value={a}>
                  {a === 0 ? "Everyone" : `${a}+`}
                </option>
              ))}
            </select>
          </label>
          <label>
            Content labels (comma-separated)
            <input value={labels} onChange={(e) => setLabels(e.target.value)} />
          </label>
          <label>
            Why, as the maker will read it
            <textarea value={reason} rows={2} maxLength={2000} required onChange={(e) => setReason(e.target.value)} />
          </label>
          <div className="row-actions">
            <button className="btn small" disabled={action.busy !== undefined || !reason.trim()}>
              Save and tell the maker
            </button>
            <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
              Cancel
            </button>
          </div>
          <ActionMessages {...action} />
        </form>
      )}
    </section>
  );
}
