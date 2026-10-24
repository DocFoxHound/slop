// SPDX-License-Identifier: AGPL-3.0-or-later
// Account → AI agents: tokens for a maker's or creative's own AI agent (crates/slop-api/src/
// maker_agents.rs). The agent does the work in its own session; Slop gives it tools and checks.

import { useState } from "react";

import { api, ApiError, type AgentToken, type AppSummary } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { ErrorState } from "./ui";

export function AiAgents() {
  const tokens = useLoad(() => api.agentTokens(), []);
  const apps = useLoad(() => api.myApps(), []);
  const [name, setName] = useState("");
  const [scope, setScope] = useState<AgentToken["scope"]>("publish");
  const [only, setOnly] = useState<string[]>([]);
  const [people, setPeople] = useState(false);
  const [days, setDays] = useState(30);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [created, setCreated] = useState<{
    secret: string;
    claude_command: string;
    mcp_url: string;
    passkey_proved: boolean;
    scope: AgentToken["scope"];
  }>();

  const create = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const t = await api.createAgentToken({ name, scope, apps: only, people, days });
      setCreated(t);
      setName("");
      tokens.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't make the token.");
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: string) => {
    setError(undefined);
    try {
      await api.revokeAgentToken(id);
      tokens.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't remove the token.");
    }
  };

  return (
    <section id="ai-agents" className="account-panel" aria-labelledby="ai-agents-title">
      <h3 className="account-panel-title" id="ai-agents-title">
        AI agents
      </h3>
      <p className="muted small">
        Let your own AI coding agent (such as Claude Code) publish and fix your apps from your editor: create store pages,
        submit releases with notes, read Slop's checks and get exact fixes, register community builds and mods, and
        offer fixes to other apps. With <b>People</b> access it can also watch your jobs, send quotes and talk in job
        conversations. It can never pay, accept agreements, change payouts or your account. Your agent does the work on
        your side; Slop runs no AI for it. Each token lasts at most 30 days, and you can remove it any time.
      </p>
      <form
        className="panel panel-pad form"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <div className="row-2">
          <label>
            Name
            <input value={name} maxLength={60} required placeholder="e.g. Claude on my laptop" onChange={(e) => setName(e.target.value)} />
          </label>
          <label>
            Can
            <select value={scope} onChange={(e) => setScope(e.target.value as AgentToken["scope"])}>
              <option value="read">Look: apps, releases, checks, reviews</option>
              <option value="publish">Also change store pages and submit releases</option>
            </select>
          </label>
        </div>
        <fieldset>
          <legend>Apps</legend>
          <label className="check">
            <input type="radio" checked={only.length === 0} onChange={() => setOnly([])} /> All my apps, and new ones
          </label>
          {(apps.data ?? []).map((a: AppSummary) => (
            <label className="check" key={a.slug}>
              <input
                type="checkbox"
                checked={only.includes(a.slug)}
                onChange={(e) => setOnly(e.target.checked ? [...only, a.slug] : only.filter((s) => s !== a.slug))}
              />{" "}
              Only {a.title}
            </label>
          ))}
        </fieldset>
        <label className="check">
          <input type="checkbox" checked={people} onChange={(e) => setPeople(e.target.checked)} /> People jobs too: see
          requests, send quotes, propose changes and talk in job conversations
        </label>
        <label>
          Expires after
          <select value={days} onChange={(e) => setDays(Number(e.target.value))}>
            {[1, 7, 30].map((d) => (
              <option key={d} value={d}>
                {d} {d === 1 ? "day" : "days"}
              </option>
            ))}
          </select>
        </label>
        <div className="row-actions">
          <button className="btn slime" disabled={busy || !name.trim()}>
            Make token
          </button>
        </div>
      </form>
      {created && (
        <div className="panel panel-pad ai-notice">
          <b>Copy it now: Slop won't show this token again.</b>
          <p className="small">Connect Claude Code to Slop (MCP), then ask it to read maker_guide:</p>
          <pre className="code-block ai-prompt" tabIndex={0}>
            {created.claude_command}
          </pre>
          <p className="small">
            Other agents: send <code>Authorization: Bearer {created.secret}</code> to <code>{created.mcp_url}</code>.
          </p>
          {!created.passkey_proved && created.scope === "publish" && (
            <p className="small">
              Confirm with a passkey before making a token if you want it to publish once Slop asks you for one (30
              days after you first publish).
            </p>
          )}
          <div className="row-actions">
            <button className="btn small" type="button" onClick={() => void navigator.clipboard.writeText(created.claude_command)}>
              Copy command
            </button>
            <button className="btn small ghost" type="button" onClick={() => setCreated(undefined)}>
              Done
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {tokens.error && <ErrorState error={tokens.error} onRetry={tokens.reload} />}
      {tokens.data && tokens.data.length > 0 && (
        <ul className="sessions panel">
          {tokens.data.map((t) => (
            <li key={t.id}>
              <div>
                <b>{t.name}</b>{" "}
                <span className="muted small">
                  {t.scope === "publish" ? "publishes" : "looks"} · {t.apps.length ? t.apps.join(", ") : "all apps"}
                  {t.people ? " · People jobs" : ""}
                </span>
                <div className="muted small">
                  Made {formatDate(t.created_at)} · last used {formatDate(t.last_used_at)} · expires{" "}
                  {formatDate(t.expires_at)}
                </div>
              </div>
              <button className="btn small" type="button" onClick={() => void revoke(t.id)}>
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
