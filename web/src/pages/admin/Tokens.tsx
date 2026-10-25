// SPDX-License-Identifier: AGPL-3.0-or-later
// Admin → AI reviewers: API tokens (`slopadm_…`) for a staff member's own AI agent
// (crates/slop-api/src/review.rs). Making one needs the security role but no fresh sign-in, like a
// maker's token: it expires, is listed here with its expiry, and can be revoked any time. A new
// token is set up the way makers' are (components/AgentSetup.tsx): saved once in SLOP_ADMIN_TOKEN
// in the staff member's own terminal, then each AI tool connected with a reference to it, so the
// token never goes into an AI chat or a command the AI sees.

import { useRef, useState } from "react";

import { api, type AdminToken } from "../../api";
import { LIFETIMES, SOON_DAYS, expiryText, isExpired, lifetimeOf, needsReplacing, replacementOf } from "../../agentTokens";
import { SETUPS } from "../../agentSetup";
import { AgentSetup, type MadeToken } from "../../components/AgentSetup";
import { ErrorState } from "../../components/ui";
import { formatDate } from "../../format";
import { useLoad } from "../../hooks";
import { useSession } from "../../session";
import { useAction } from "./shared";

/** The most powerful tokens expire sooner (review.rs). */
const MODERATE_MAX_DAYS = 90;

const lifetimesFor = (scope: AdminToken["scope"]) =>
  LIFETIMES.filter((l) => scope !== "moderate" || l.days <= MODERATE_MAX_DAYS);

export function TokensTab() {
  const { me } = useSession();
  const tokens = useLoad(() => api.admin.tokens(), []);
  const { busy, error, run } = useAction(tokens.reload);
  const [name, setName] = useState("");
  const [scope, setScope] = useState<AdminToken["scope"]>("read");
  const [days, setDays] = useState(90);
  const [created, setCreated] = useState<MadeToken>();
  const [replacing, setReplacing] = useState<AdminToken>();
  const nameInput = useRef<HTMLInputElement>(null);
  const now = Date.now();
  const choices = lifetimesFor(scope);

  const changeScope = (next: AdminToken["scope"]) => {
    setScope(next);
    if (next === "moderate" && days > MODERATE_MAX_DAYS) setDays(MODERATE_MAX_DAYS);
  };

  /** Fills in the form with an expiring token's settings; the old one keeps working meanwhile. */
  const replace = (t: AdminToken) => {
    setReplacing(t);
    setName(t.name);
    setScope(t.scope);
    setDays(Math.min(lifetimeOf(t), t.scope === "moderate" ? MODERATE_MAX_DAYS : Infinity));
    setCreated(undefined);
    nameInput.current?.scrollIntoView({ block: "center" });
    nameInput.current?.focus();
  };

  return (
    <>
      <p className="muted">
        An AI reviewer gets an API token that acts for you on the admin API. It reads everything waiting (releases,
        change requests, pictures, apps ready for the store) at <code>/v1/admin/review</code>, follows the guide at{" "}
        <code>/v1/admin/review/guide</code>, and leaves notes you see in the review queue. It can watch the server at{" "}
        <code>/v1/admin/status</code> (alerts, queues, backups) and, at your request, act on reports and accounts. A{" "}
        <b>read</b> token can only look and leave notes; a <b>review</b> token can also approve and reject; a{" "}
        <b>moderate</b> token can also suspend and reinstate accounts, withdraw releases and take apps out of the store.
        Everything it does is in the audit log under the token's name. Agents that speak MCP (such as Claude Code) connect
        to <code>/v1/admin/mcp</code> and get all of this as tools. A token lasts up to 12 months (a moderate one up to 3
        months), and you can revoke it any time.
      </p>
      <form
        className="panel panel-pad form"
        onSubmit={(e) => {
          e.preventDefault();
          void run("create", async () => {
            const t = await api.admin.createToken({ name, scope, days });
            setCreated({ secret: t.token, mcp_url: t.mcp_url, name: t.name });
            setName("");
            setReplacing(undefined);
          });
        }}
      >
        <h2>New token</h2>
        {replacing && (
          <p className="notice agent-replacing" role="status">
            Making a replacement for “{replacing.name}”, with the same settings. The old token keeps working until{" "}
            {formatDate(replacing.expires_at)}
            {isExpired(replacing, now) ? " (it has expired)" : ""}; revoke it once the new one is set up.{" "}
            <button type="button" className="btn small ghost" onClick={() => setReplacing(undefined)}>
              Cancel
            </button>
          </p>
        )}
        <div className="row-2">
          <label>
            Name
            <input
              ref={nameInput}
              value={name}
              maxLength={60}
              required
              placeholder="e.g. Claude, picture checks"
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            Can
            <select value={scope} onChange={(e) => changeScope(e.target.value as AdminToken["scope"])}>
              <option value="read">Look and leave notes (read)</option>
              <option value="review">Also approve and reject (review)</option>
              <option value="moderate">Also suspend accounts and withdraw releases (moderate)</option>
            </select>
          </label>
        </div>
        <label>
          Expires after
          <select value={days} onChange={(e) => setDays(Number(e.target.value))}>
            {choices.map((l) => (
              <option key={l.days} value={l.days}>
                {l.label}
              </option>
            ))}
          </select>
        </label>
        <div className="row-actions">
          <button className="btn slime" disabled={busy !== undefined || !name.trim()}>
            Make token
          </button>
        </div>
      </form>
      {created && (
        <AgentSetup
          token={created}
          kind={SETUPS.staff}
          footer={
            <>
              <p className="small muted">
                Only ask it to suspend accounts, withdraw releases or take apps out of the store when you mean it: it acts
                as you.
              </p>
              <div className="row-actions">
                <button className="btn small ghost" type="button" onClick={() => setCreated(undefined)}>
                  Done
                </button>
              </div>
            </>
          }
        />
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <h2 className="section-title">Tokens</h2>
      {tokens.error && <ErrorState error={tokens.error} onRetry={tokens.reload} />}
      {tokens.data?.length === 0 && <div className="empty-state">No tokens yet.</div>}
      <ul className="release-list">
        {tokens.data?.map((t) => {
          const all = (tokens.data ?? []).filter((o) => !o.revoked_at && o.admin === t.admin);
          const expired = isExpired(t, now);
          const live = !t.revoked_at && !expired;
          const replaced = !t.revoked_at ? replacementOf(t, all, now) : undefined;
          const soon = !t.revoked_at && needsReplacing(t, all, SOON_DAYS, now);
          // A replacement acts for whoever makes it, so only the token's own admin is offered one.
          const mine = t.admin === me?.display_name;
          return (
            <li
              key={t.id}
              className={`panel panel-pad title-row spread${expired && !t.revoked_at ? " token-expired" : soon ? " token-soon" : ""}`}
            >
              <div>
                <div className="title-row">
                  <b>{t.name}</b>
                  <span
                    className={`badge ${t.scope === "moderate" ? "bad" : t.scope === "review" ? "soon" : "neutral"}`}
                  >
                    {t.scope.toUpperCase()}
                  </span>
                  {t.revoked_at ? (
                    <span className="badge bad">REVOKED</span>
                  ) : expired ? (
                    <span className="badge bad">EXPIRED</span>
                  ) : (
                    soon && <span className="badge soon">EXPIRES SOON</span>
                  )}
                </div>
                <div className="muted small">
                  for {t.admin} · made {formatDate(t.created_at)} ·{" "}
                  {t.last_used_at ? `last used ${formatDate(t.last_used_at)}` : "never used"} ·{" "}
                  <time dateTime={t.expires_at} title={formatDate(t.expires_at)}>
                    {expired ? `expired on ${formatDate(t.expires_at)}` : expiryText(t.expires_at, now)}
                  </time>
                  {replaced && <> · replaced by a newer “{replaced.name}”</>}
                </div>
              </div>
              <div className="row-actions">
                {soon && mine && (
                  <button
                    className="btn small slime"
                    type="button"
                    aria-label={`Make a replacement for ${t.name}`}
                    disabled={busy !== undefined}
                    onClick={() => replace(t)}
                  >
                    Make a replacement
                  </button>
                )}
                {live && (
                  <button
                    className="btn small ghost danger"
                    disabled={busy !== undefined}
                    onClick={() => void run(t.id, () => api.admin.revokeToken(t.id))}
                  >
                    Revoke
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </>
  );
}
