// SPDX-License-Identifier: AGPL-3.0-or-later
// Finding an account and acting on it (crates/slop-api/src/agent.rs): its apps, reports and
// decisions, and warning, suspending, reinstating or giving passkey grace; fixing its profile and
// teams, and making it staff (ProfileFix.tsx). Opening an account is written to the audit log.

import { useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";

import { api } from "../../api";
import { ErrorState, Loading } from "../../components/ui";
import { formatDate } from "../../format";
import { useLoad } from "../../hooks";
import { ProfileFix, StaffControls } from "./ProfileFix";
import { ActionMessages, ReasonButton, RuleSelect, useAction } from "./shared";

export function UsersTab() {
  const [params, setParams] = useSearchParams();
  const user = params.get("user");
  if (user) return <UserRecord id={user} />;
  return <FindUsers initial={params.get("q") ?? ""} onSearch={(q) => setParams({ tab: "users", q }, { replace: true })} />;
}

function FindUsers({ initial, onSearch }: { initial: string; onSearch: (q: string) => void }) {
  const [text, setText] = useState(initial);
  const found = useLoad(() => (initial ? api.admin.findUsers(initial) : Promise.resolve([])), [initial]);
  return (
    <>
      <form
        className="filters"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) onSearch(text.trim());
        }}
      >
        <label>
          Find an account{" "}
          <input value={text} placeholder="Name, People handle or account ID" onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn small">Search</button>
      </form>
      {found.error && <ErrorState error={found.error} onRetry={found.reload} />}
      {initial && found.data?.length === 0 && <div className="empty-state">No accounts match.</div>}
      <ul className="release-list">
        {found.data?.map((u) => (
          <li key={u.id} className="panel panel-pad title-row spread">
            <div className="title-row">
              <Link to={`/admin?tab=users&user=${u.id}`}>
                <b>{u.display_name}</b>
              </Link>
              {u.handle && <span className="muted">@{u.handle}</span>}
              {u.is_admin && <span className="badge official">STAFF</span>}
              {u.suspended && <span className="badge bad">SUSPENDED</span>}
              {u.closed && <span className="badge neutral">CLOSED</span>}
            </div>
            <span className="muted small">joined {formatDate(u.created_at)}</span>
          </li>
        ))}
      </ul>
    </>
  );
}

function UserRecord({ id }: { id: string }) {
  const user = useLoad(() => api.admin.user(id), [id]);
  const rules = useLoad(() => api.admin.queue().then((q) => q.rules), []);
  const action = useAction(user.reload);
  if (user.error) return <ErrorState error={user.error} onRetry={user.reload} />;
  if (!user.data) return <Loading />;
  const u = user.data;
  return (
    <div className="admin-record">
      <p>
        <Link to="/admin?tab=users">← Find another account</Link>
      </p>
      <div className="title-row">
        <h2>{u.display_name}</h2>
        {u.is_admin && <span className="badge official">STAFF</span>}
        {u.suspended_at && <span className="badge bad">SUSPENDED</span>}
        {u.closed_at && <span className="badge neutral">CLOSED</span>}
      </div>
      <p className="muted">
        <code>{u.id}</code> · joined {formatDate(u.created_at)}
        {u.country_code && <> · {u.country_code}</>} · signs in with {u.sign_ins.join(", ") || "nothing linked"}
        {u.suspended_at && <> · suspended {formatDate(u.suspended_at)}</>}
        {u.closed_at && <> · closed {formatDate(u.closed_at)}</>}
      </p>

      {!u.closed_at && (
        <section className="panel panel-pad">
          <h3>Act on this account</h3>
          <div className="row-actions">
            <DecideForm
              label="Warn"
              action="warn"
              rules={rules.data ?? []}
              busy={action.busy !== undefined}
              onSubmit={(rule, facts) => void action.run("warn", () => api.admin.decideUser(u.id, { action: "warn", rule, facts }))}
            />
            {u.suspended_at ? (
              <ReasonButton
                label="Reinstate"
                placeholder="Why, for the record (they're told their account is back)"
                disabled={action.busy !== undefined}
                onSubmit={(reason) => void action.run("lift", () => api.admin.liftSuspension(u.id, reason))}
              />
            ) : (
              <DecideForm
                label="Suspend"
                action="suspend_account"
                danger
                rules={rules.data ?? []}
                busy={action.busy !== undefined}
                onSubmit={(rule, facts) =>
                  void action.run("suspend", () => api.admin.decideUser(u.id, { action: "suspend_account", rule, facts }))
                }
              />
            )}
            <ReasonButton
              label="Passkey grace"
              placeholder="Why (lets them publish without a passkey for a short time)"
              disabled={action.busy !== undefined}
              onSubmit={(reason) => void action.run("grace", () => api.admin.passkeyGrace(u.id, reason))}
            />
          </div>
          <p className="muted small">
            Warnings and suspensions send them a statement of reasons they can appeal. Suspending and reinstating are
            sensitive and may need a second staff member's approval.
          </p>
          <ActionMessages {...action} />
        </section>
      )}

      {!u.closed_at && <ProfileFix id={u.id} />}
      {!u.closed_at && <StaffControls id={u.id} name={u.display_name} isStaff={u.is_admin} onDone={user.reload} />}

      <section className="panel panel-pad">
        <h3>Apps ({u.apps.length})</h3>
        {u.apps.length === 0 ? (
          <p className="muted">None.</p>
        ) : (
          <ul className="plain">
            {u.apps.map((a) => (
              <li key={a.slug}>
                <Link to={`/admin?tab=apps&app=${encodeURIComponent(a.slug)}`}>{a.title}</Link>{" "}
                <span className="muted small">
                  {a.status}
                  {a.visibility !== "public" && ", private"}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel panel-pad">
        <h3>Reports about them ({u.reports_about.length})</h3>
        {u.reports_about.length === 0 ? (
          <p className="muted">None.</p>
        ) : (
          <ul className="plain">
            {u.reports_about.map((r) => (
              <li key={r.reference}>
                <b>{r.reference}</b> · {r.about} · {r.reason} · {r.status} · {formatDate(new Date(r.at * 1000).toISOString())}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel panel-pad">
        <h3>Decisions ({u.decisions.length})</h3>
        {u.decisions.length === 0 ? (
          <p className="muted">None.</p>
        ) : (
          <ul className="plain">
            {u.decisions.map((d) => (
              <li key={d.id}>
                <b>{d.reference}</b> · {d.action} ({d.rule}) about {d.about} ·{" "}
                {formatDate(new Date(d.at * 1000).toISOString())}
                {d.reversed && " · reversed"}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/** A warning or suspension: a rule and the facts, which the person reads. */
function DecideForm({
  label,
  action,
  danger,
  rules,
  busy,
  onSubmit,
}: {
  label: string;
  action: string;
  danger?: boolean;
  rules: Array<[string, string]>;
  busy: boolean;
  onSubmit: (rule: string, facts: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [rule, setRule] = useState("");
  const [facts, setFacts] = useState("");
  if (!open)
    return (
      <button className={`btn small ${danger ? "ghost danger" : "ghost"}`} disabled={busy} onClick={() => setOpen(true)}>
        {label}
      </button>
    );
  const submit = (e: FormEvent) => {
    e.preventDefault();
    onSubmit(rule, facts.trim());
  };
  return (
    <form className="form decide-form" onSubmit={submit} aria-label={`${label}: ${action}`}>
      <RuleSelect rules={rules} value={rule} onChange={setRule} />
      <label>
        The facts, as they will read them
        <textarea value={facts} rows={3} maxLength={4000} required onChange={(e) => setFacts(e.target.value)} />
      </label>
      <div className="row-actions">
        <button className={`btn small ${danger ? "danger" : ""}`} disabled={busy || !rule || !facts.trim()}>
          {label}
        </button>
        <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
    </form>
  );
}
