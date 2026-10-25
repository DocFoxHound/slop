// SPDX-License-Identifier: AGPL-3.0-or-later
// The admin panel for Slop staff: the review queue (an inbox, admin/Inbox.tsx), apps, accounts,
// the front page, money, the audit log, AI messages, AI reviewer tokens, staff, policies and legal
// requests. Every action is recorded in the audit log under the admin's name
// (crates/slop-api/src/admin_web.rs).

import { useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";

import { api, type AdminApp, type AppStage, type AppStatus, type AuditEntry, type StaffRole } from "../api";
import { ErrorState, Loading } from "../components/ui";
import { accountId, formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";
import { NotFound } from "./Listing";
import { LegalRequestsTab } from "../components/LegalRequests";
import { AiMessagesTab } from "./admin/AiMessages";
import { AppRecord, AuditRows } from "./admin/AppRecord";
import { AdminHome } from "./admin/Home";
import { QueueTab, useWaiting } from "./admin/Inbox";
import { LegalDocsTab } from "./admin/LegalDocs";
import { StoreTab } from "./admin/StoreTab";
import { MoneyTab } from "./admin/Money";
import { DelistButton, ReasonButton, useAction } from "./admin/shared";
import { TokensTab } from "./admin/Tokens";
import { UsersTab } from "./admin/Users";

type Tab =
  | "home"
  | "queue"
  | "apps"
  | "users"
  | "store"
  | "money"
  | "audit"
  | "ai-messages"
  | "ai"
  | "staff"
  | "docs"
  | "legal";

export function Admin() {
  useDocumentTitle("Admin");
  const { me } = useSession();
  if (me === undefined) return <Loading />;
  if (!me?.is_admin) return <NotFound />;
  return <AdminPanel />;
}

function AdminPanel() {
  const [params, setParams] = useSearchParams();
  const tab = (params.get("tab") as Tab | null) ?? "home";
  // What's waiting, shared by Home, the review queue and the queue tab's count.
  const waiting = useWaiting();
  // Back on Home or the queue from another tab: what's waiting may have changed there.
  const first = useRef(true);
  useEffect(() => {
    if (first.current) first.current = false;
    else if (tab === "home" || tab === "queue") waiting.reload();
  }, [tab]);

  return (
    <div className="page admin">
      <h1 className="page-title">Admin</h1>
      <div className="seg" role="tablist">
        {(
          [
            ["home", "Home"],
            ["queue", "Review queue"],
            ["apps", "Apps"],
            ["users", "Accounts"],
            ["store", "Front page"],
            ["money", "Money"],
            ["audit", "Audit log"],
            ["ai-messages", "AI messages"],
            ["ai", "AI reviewers"],
            ["staff", "Staff"],
            ["docs", "Terms & policies"],
            ["legal", "Legal requests"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            role="tab"
            aria-selected={tab === key}
            className={tab === key ? "on" : ""}
            onClick={() => setParams({ tab: key }, { replace: true })}
          >
            {label}
            {key === "queue" && waiting.total !== undefined && (
              <>
                {" "}
                <span className="admin-tab-count">{waiting.total}</span>
                <span className="sr-only"> waiting</span>
              </>
            )}
          </button>
        ))}
      </div>
      {tab === "home" && <AdminHome waiting={waiting} />}
      {tab === "queue" && <QueueTab waiting={waiting} />}
      {tab === "apps" && (params.get("app") ? <AppRecord slug={params.get("app")!} /> : <AppsTab />)}
      {tab === "users" && <UsersTab />}
      {tab === "store" && <StoreTab />}
      {tab === "money" && <MoneyTab />}
      {tab === "audit" && <AuditTab />}
      {tab === "ai-messages" && <AiMessagesTab />}
      {tab === "ai" && <TokensTab />}
      {tab === "staff" && <StaffTab />}
      {tab === "docs" && <LegalDocsTab />}
      {tab === "legal" && <LegalRequestsTab />}
    </div>
  );
}

/** Staff and their roles (staff.rs). Changing a role is itself a sensitive action. */
function StaffTab() {
  const staff = useLoad(() => api.admin.staff(), []);
  const history = useLoad(() => api.admin.actions(true), []);
  const { busy, error, notice, run } = useAction(() => {
    staff.reload();
    history.reload();
  });
  const [newId, setNewId] = useState("");
  const [newIdError, setNewIdError] = useState<string>();
  const [newRole, setNewRole] = useState<StaffRole>("reviewer");
  if (staff.error)
    return <ErrorState error={staff.error} onRetry={staff.reload} />;
  if (!staff.data) return <Loading />;
  const s = staff.data;
  const canManage =
    s.my_roles.includes("owner") || s.my_roles.includes("security");
  const ROLE_HELP: Record<StaffRole, string> = {
    owner: "everything",
    moderator: "reviews, reports, appeals and sensitive actions",
    reviewer: "releases, changes, pictures, portfolios and listings",
    support: "reports, appeals and People problems",
    finance: "payments",
    security: "tokens, staff roles and the audit log",
  };
  return (
    <>
      <p className="muted">
        Sensitive actions (withdrawing a release, taking an app out of the
        store, suspending or reinstating an account, changing roles) need{" "}
        <b>{s.approvals_required}</b> different staff member
        {s.approvals_required === 1 ? "" : "s"}
        {s.approvals_required === 1
          ? ": they happen at once but are recorded as approvals. Set SLOP_APPROVALS_REQUIRED=2 on the server once there's a second person."
          : ", counting the one who asks."}
      </p>
      <ul className="small">
        {s.roles.map((r) => (
          <li key={r}>
            <b>{r}</b>: {ROLE_HELP[r]}
          </li>
        ))}
      </ul>
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <table className="kv-table">
        <thead>
          <tr>
            <th scope="col">Staff</th>
            {s.roles.map((r) => (
              <th scope="col" key={r}>
                {r}
              </th>
            ))}
            <th scope="col">Name shown to AI agents</th>
            {canManage && <th scope="col">Remove</th>}
          </tr>
        </thead>
        <tbody>
          {s.staff.map((m) => (
            <tr key={m.id}>
              <td>
                <Link to={`/admin?tab=users&user=${m.id}`}>{m.display_name}</Link>
                {m.suspended && <span className="badge bad">SUSPENDED</span>}
              </td>
              {s.roles.map((r) => {
                const on = m.roles.includes(r);
                return (
                  <td key={r}>
                    <input
                      type="checkbox"
                      aria-label={`${m.display_name}: ${r}`}
                      checked={on}
                      disabled={
                        !canManage ||
                        busy !== undefined ||
                        (r === "owner" && !s.my_roles.includes("owner"))
                      }
                      onChange={() =>
                        void run(`${m.id}/${r}`, () =>
                          api.admin.changeRole(m.id, r, !on),
                        )
                      }
                    />
                  </td>
                );
              })}
              <td>
                <input
                  type="checkbox"
                  aria-label={`${m.display_name}: name shown to AI agents`}
                  checked={m.listed_by_name}
                  disabled={busy !== undefined || (m.id !== s.me && !s.my_roles.includes("owner"))}
                  onChange={() => void run(`${m.id}/listed`, () => api.admin.setListedByName(m.id, !m.listed_by_name))}
                />
              </td>
              {canManage && (
                <td>
                  <ReasonButton
                    label={`Remove ${m.display_name}`}
                    placeholder="Why (10+ characters): every role and API token ends"
                    danger
                    disabled={busy !== undefined}
                    onSubmit={(reason) => void run(`${m.id}/remove`, () => api.admin.removeStaff(m.id, reason))}
                  />
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {canManage && (
        <form
          className="reason-form"
          onSubmit={(e) => {
            e.preventDefault();
            const id = accountId(newId);
            setNewIdError(
              id ? undefined : "Paste the account's ID, or the link to their profile (…/u/…).",
            );
            if (!id) return;
            void run("new", () => api.admin.changeRole(id, newRole, true)).then(
              () => setNewId(""),
            );
          }}
        >
          <input
            aria-label="Account ID or profile link"
            placeholder="Account ID or profile link"
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
          />
          <select
            aria-label="Role"
            value={newRole}
            onChange={(e) => setNewRole(e.target.value as StaffRole)}
          >
            {s.roles.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <button
            className="btn small"
            disabled={!newId.trim() || busy !== undefined}
          >
            Add to staff
          </button>
          {newIdError && (
            <p className="form-error" role="alert">
              {newIdError}
            </p>
          )}
        </form>
      )}
      <h2 className="section-title">Sensitive actions</h2>
      {history.data?.actions.length === 0 && (
        <div className="empty-state">None yet.</div>
      )}
      <ul className="release-list">
        {history.data?.actions.map((a) => (
          <li key={a.id} className="small">
            <span
              className={`badge ${a.status === "done" ? "official" : a.status === "pending" ? "soon" : "neutral"}`}
            >
              {a.status.toUpperCase()}
            </span>{" "}
            {a.summary}{" "}
            <span className="muted">
              · {a.approvals.join(", ")} · {formatDate(a.requested_at)}
            </span>
          </li>
        ))}
      </ul>
    </>
  );
}


const STATUSES: Array<AppStatus | ""> = ["", "draft", "listed", "delisted"];

function AppsTab() {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const [text, setText] = useState(q);
  const [status, setStatus] = useState<AppStatus | "">("");
  const apps = useLoad(() => api.admin.apps(status || undefined, q), [status, q]);
  // The Content Policy rules, for the statement of reasons when taking an app out.
  const rules = useLoad(() => api.admin.queue().then((q) => q.rules), []);
  const { busy, error, notice, run } = useAction(apps.reload);

  return (
    <>
      <form
        className="filters"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          setParams({ tab: "apps", q: text.trim() }, { replace: true });
        }}
      >
        <label>
          Find{" "}
          <input value={text} placeholder="Title, address or maker" onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn small">Search</button>
        <label>
          Show{" "}
          <select
            value={status}
            onChange={(e) => setStatus(e.target.value as AppStatus | "")}
          >
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {s ? s[0].toUpperCase() + s.slice(1) : "All apps"}
              </option>
            ))}
          </select>
        </label>
      </form>
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {apps.error && <ErrorState error={apps.error} onRetry={apps.reload} />}
      {!apps.data && !apps.error && <Loading />}
      {apps.data?.length === 0 && <div className="empty-state">No apps.</div>}
      <ul className="release-list">
        {apps.data?.map((a: AdminApp) => (
          <li key={a.slug} className="panel panel-pad">
            <div className="title-row spread">
              <div className="title-row">
                <Link to={`/admin?tab=apps&app=${encodeURIComponent(a.slug)}`}>
                  <b>{a.title}</b>
                </Link>
                <span className="muted">by {a.maker}</span>
                <Link className="small" to={`/app/${a.slug}`}>
                  store page
                </Link>
                <span
                  className={`badge ${a.status === "listed" ? "official" : a.status === "delisted" ? "bad" : "neutral"}`}
                >
                  {a.status.toUpperCase()}
                </span>
              </div>
              <label className="small">
                Stage{" "}
                <select
                  value={a.stage}
                  disabled={busy !== undefined}
                  onChange={(e) =>
                    void run(a.slug, () =>
                      api.admin.stage(a.slug, e.target.value as AppStage),
                    )
                  }
                >
                  <option value="coming_soon">Coming soon</option>
                  <option value="beta">Beta</option>
                  <option value="released">Released</option>
                </select>
              </label>
            </div>
            <div className="row-actions">
              {a.status === "listed" ? (
                <DelistButton
                  rules={rules.data ?? []}
                  disabled={busy !== undefined}
                  onSubmit={(rule, reason) =>
                    void run(a.slug, () =>
                      api.admin.delist(a.slug, rule, reason),
                    )
                  }
                />
              ) : a.stage !== "coming_soon" && a.releases.length === 0 ? (
                <span className="muted small">
                  {a.pending_releases > 0
                    ? "Can't list yet: approve its release under Review queue → New releases first."
                    : "Can't list yet: the maker hasn't submitted a release. Set the stage to Coming soon to list the page without one."}
                </span>
              ) : (
                <button
                  className="btn small"
                  disabled={busy !== undefined}
                  onClick={() => void run(a.slug, () => api.admin.list(a.slug))}
                >
                  Put in the store
                </button>
              )}
              {a.releases.map((r) => (
                <ReasonButton
                  key={r.id}
                  label={`Withdraw ${r.version}`}
                  placeholder="Reason, shown to players (stops it launching)"
                  danger
                  disabled={busy !== undefined}
                  onSubmit={(reason) =>
                    void run(r.id, () => api.admin.revoke(r.id, reason))
                  }
                />
              ))}
            </div>
          </li>
        ))}
      </ul>
    </>
  );
}

function AuditTab() {
  const [text, setText] = useState("");
  const [q, setQ] = useState("");
  const [pages, setPages] = useState<AuditEntry[][]>([]);
  const first = useLoad(() => api.admin.audit(q), [q]);
  const [more, setMore] = useState<"loading" | "done" | undefined>();
  if (first.error) return <ErrorState error={first.error} onRetry={first.reload} />;
  if (!first.data) return <Loading />;
  const rows = [first.data, ...pages].flat();
  const older = async () => {
    setMore("loading");
    const page = await api.admin.audit(q, rows[rows.length - 1]?.id);
    setPages((p) => [...p, page]);
    setMore(page.length < 100 ? "done" : undefined);
  };
  return (
    <>
      <form
        className="filters"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          setPages([]);
          setMore(undefined);
          setQ(text.trim());
        }}
      >
        <label>
          Find{" "}
          <input value={text} placeholder="Who, what (e.g. donations.hold) or an app's address" onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn small">Search</button>
      </form>
      {rows.length === 0 ? (
        <div className="empty-state">Nothing recorded{q && " that matches"}.</div>
      ) : (
        <AuditRows rows={rows} />
      )}
      {first.data.length === 100 && more !== "done" && (
        <button className="btn small ghost" disabled={more === "loading"} onClick={() => void older()}>
          Older entries
        </button>
      )}
    </>
  );
}

