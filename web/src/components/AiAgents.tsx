// SPDX-License-Identifier: AGPL-3.0-or-later
// Account → AI agents: a maker's or creative's own AI agents. The easy way comes first: the AI
// tool is given SLAPorSLOP's address and approved in the browser (EasyConnect, crates/slop-api/src/
// agent_oauth.rs), with no token to handle. The other way is a token (crates/slop-api/src/
// maker_agents.rs). Both are listed together, and removed the same way. The agent does the work in its own session; SLAPorSLOP gives it tools and checks.
// An agent can't accept agreements, so a publishing token needs the Publisher Agreement accepted
// here: it's shown when the token is made, and again here whenever a new version waits.
// Each token shows when it expires (agentTokens.ts); one expiring soon or expired offers Make a
// replacement, which fills in the form with its settings. A new token's setup (AgentSetup.tsx)
// never puts the token into an AI chat.

import { useRef, useState } from "react";

import { api, ApiError, type AgentToken, type AppSummary, type LegalNotice } from "../api";
import { LIFETIMES, SOON_DAYS, expiryText, isExpired, lifetimeOf, needsReplacing, replacementOf } from "../agentTokens";
import { formatDate } from "../format";
import { useLoad, type Loaded } from "../hooks";
import { mcpAddress } from "../agentSetup";
import { siteOrigin } from "../browsing";
import { AgentSetup, EasyConnect, type MadeToken } from "./AgentSetup";
import { AcceptDocument, useTermsGate } from "./Terms";
import { ErrorState } from "./ui";

declare const __SLOP_SITE_URL__: string;

/** The Publisher Agreement, waiting to be accepted (a version in effect or announced). */
interface Waiting {
  version: string;
  title: string;
  /** Null: never accepted (a token made before publishing tokens needed it). */
  notice: LegalNotice | null;
}

/**
 * `notices`: the account page's agreements to hear about, reloaded here once one is accepted.
 * `tokens`: the account's agent tokens (the account page's badge counts the expiring ones).
 */
export function AiAgents({ notices, tokens }: { notices: Loaded<LegalNotice[]>; tokens: Loaded<AgentToken[]> }) {
  const apps = useLoad(() => api.myApps(), []);
  const publisher = useLoad(() => api.legal.mine("publisher"), []);
  const terms = useTermsGate();
  const [accepting, setAccepting] = useState<string>();
  const [name, setName] = useState("");
  const [scope, setScope] = useState<AgentToken["scope"]>("publish");
  const [only, setOnly] = useState<string[]>([]);
  const [people, setPeople] = useState(false);
  const [days, setDays] = useState(30);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [created, setCreated] = useState<MadeToken & { scope: AgentToken["scope"] }>();
  /** The token a replacement is being made for. */
  const [replacing, setReplacing] = useState<AgentToken>();
  const nameInput = useRef<HTMLInputElement>(null);
  const now = Date.now();

  /** Fills in the form with an expiring token's settings; the old one keeps working meanwhile. */
  const replace = (t: AgentToken) => {
    const mine = new Set((apps.data ?? []).map((a) => a.slug));
    setReplacing(t);
    setName(t.name);
    setScope(t.scope);
    setOnly(t.apps.filter((a) => mine.size === 0 || mine.has(a)));
    setPeople(t.people);
    setDays(lifetimeOf(t));
    setCreated(undefined);
    setError(undefined);
    nameInput.current?.scrollIntoView({ block: "center" });
    nameInput.current?.focus();
  };

  const create = async () => {
    setBusy(true);
    setError(undefined);
    try {
      // A publishing token needs the Publisher Agreement: shown here, then the token is made.
      const t = await terms.guard(() => api.createAgentToken({ name, scope, apps: only, people, days }));
      setCreated(t);
      setName("");
      setReplacing(undefined);
      tokens.reload();
      if (t.scope === "publish") agreementsChanged();
    } catch (e) {
      setError(
        e instanceof ApiError && e.code === "terms_required"
          ? "A token that publishes needs the Publisher Agreement: accept it to make one, or choose Look."
          : e instanceof ApiError
            ? e.message
            : "Couldn't make the token.",
      );
    } finally {
      setBusy(false);
    }
  };

  const agreementsChanged = () => {
    publisher.reload();
    notices.reload();
  };

  // What the agent's publishing tools wait for: a change announced or in effect, or (for a token
  // made before tokens needed it) the agreement never accepted.
  const publishes = (tokens.data ?? []).some((t) => t.scope === "publish");
  const waiting: Waiting[] = (notices.data ?? [])
    .filter((n) => n.kind === "publisher" && n.action === "accept")
    .map((n) => ({ version: n.version, title: n.title, notice: n }));
  const p = publisher.data;
  if (publishes && p?.version && !p.accepted && !waiting.some((w) => w.version === p.version)) {
    waiting.unshift({ version: p.version, title: "Publisher Agreement", notice: null });
  }

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
        Let your own AI agent (such as Claude Code, Claude or ChatGPT) publish and fix your apps: create store pages,
        submit releases with notes, read SLAPorSLOP's checks and get exact fixes, register community builds and mods, and
        offer fixes to other apps. With <b>People</b> access it can also watch your jobs, send quotes and talk in job
        conversations. It can never pay, accept agreements, change payouts or your account. Your agent does the work on
        your side; SLAPorSLOP runs no AI for it. Each connection lasts up to 12 months: SLAPorSLOP reminds you a week and a day
        before it ends, and you can remove it any time.
      </p>
      <EasyConnect mcpUrl={mcpAddress(siteOrigin(window.location, typeof __SLOP_SITE_URL__ === "string" ? __SLOP_SITE_URL__ : ""))} />
      {terms.prompt}
      {waiting.map((w) => (
        <div className="notice" key={w.version} role="status">
          {w.notice && !w.notice.in_effect ? (
            <>
              The {w.title} changes on {formatDate(w.notice.effective_at)}: {w.notice.summary} Accept it before then so
              your AI agent can keep publishing.{" "}
            </>
          ) : w.notice ? (
            <>
              Your AI agent can't publish until you accept the {w.title}: it changed on{" "}
              {formatDate(w.notice.effective_at)}. {w.notice.summary}{" "}
            </>
          ) : (
            <>Your AI agent can't publish until you accept the {w.title}. </>
          )}
          <button type="button" className="btn small slime" onClick={() => setAccepting(w.version)}>
            Review and accept version {w.version}
          </button>
        </div>
      ))}
      {accepting && (
        <AcceptDocument
          kind="publisher"
          version={accepting}
          onAccepted={() => {
            setAccepting(undefined);
            agreementsChanged();
          }}
          onCancel={() => setAccepting(undefined)}
        />
      )}
      <h4 className="agent-way-title">The other way: make a token</h4>
      <p className="muted small">
        For AI tools that can't sign in to SLAPorSLOP in the browser: make a token here and set it up on your computer. Never paste
        it into an AI chat.
      </p>
      <form
        className="panel panel-pad form"
        aria-label="Make a token"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        {replacing && (
          <p className="notice agent-replacing" role="status">
            Making a replacement for “{replacing.name}”, with the same settings. The old token keeps working until{" "}
            {formatDate(replacing.expires_at)}
            {isExpired(replacing, now) ? " (it has expired)" : ""}; remove it once the new one is set up.{" "}
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
              placeholder="e.g. Claude on my laptop"
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            Can
            <select value={scope} onChange={(e) => setScope(e.target.value as AgentToken["scope"])}>
              <option value="read">Look: apps, releases, checks, reviews</option>
              <option value="publish">Also change store pages and submit releases</option>
            </select>
          </label>
        </div>
        {scope === "publish" && !publisher.data?.accepted && (
          <p className="small muted">Publishing tokens need the Publisher Agreement: it's shown when you make the token.</p>
        )}
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
            {LIFETIMES.map((l) => (
              <option key={l.days} value={l.days}>
                {l.label}
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
        <AgentSetup
          token={created}
          footer={
            <>
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
      {tokens.error && <ErrorState error={tokens.error} onRetry={tokens.reload} />}
      {tokens.data && tokens.data.length > 0 && (
        <ul className="sessions panel agent-tokens">
          {tokens.data.map((t) => {
            const all = tokens.data ?? [];
            const expired = isExpired(t, now);
            const replaced = replacementOf(t, all, now);
            const soon = needsReplacing(t, all, SOON_DAYS, now);
            const connected = t.connected;
            return (
              <li key={t.id} className={expired ? "token-expired" : soon ? "token-soon" : undefined}>
                <div>
                  <b>{t.name}</b>{" "}
                  <span className="muted small">
                    {t.scope === "publish" ? "publishes" : "looks"} · {t.apps.length ? t.apps.join(", ") : "all apps"}
                    {t.people ? " · People jobs" : ""}
                  </span>
                  {connected && (
                    <div className="small muted token-connected">
                      Approved in the browser for {connected.client}
                      {connected.redirect_host ? ` (${connected.redirect_host})` : " on your computer"}
                      {soon && !expired ? ": when it ends, approve it again when your tool asks" : ""}
                    </div>
                  )}
                  {expired ? (
                    <span className="badge token-flag bad">Expired</span>
                  ) : soon ? (
                    <span className="badge token-flag soon">Expires soon</span>
                  ) : null}
                  <div className="small token-expiry">
                    <span className="muted">
                      Made {formatDate(t.created_at)} · last used {formatDate(t.last_used_at)} ·{" "}
                    </span>
                    <time dateTime={t.expires_at} title={formatDate(t.expires_at)} className={soon || expired ? "" : "muted"}>
                      {expired ? `expired on ${formatDate(t.expires_at)}: it no longer works` : expiryText(t.expires_at, now)}
                    </time>
                    {replaced && <span className="muted"> · replaced by a newer “{replaced.name}”</span>}
                  </div>
                </div>
                <div className="row-actions token-actions">
                  {soon && !connected && (
                    <button
                      className="btn small slime"
                      type="button"
                      aria-label={`Make a replacement for ${t.name}`}
                      onClick={() => replace(t)}
                    >
                      Make a replacement
                    </button>
                  )}
                  <button className="btn small" type="button" onClick={() => void revoke(t.id)}>
                    Remove
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
