// SPDX-License-Identifier: AGPL-3.0-or-later
// Approving an AI tool (crates/slop-api/src/agent_oauth.rs): the tool was given SLAPorSLOP's
// address and opened this page in the browser. Signed in (signing in or up first, and coming
// back here), the person sees which tool is asking and, prominently, where approving sends the
// browser, chooses what it may do exactly as for a token on Account → AI agents, and approves or
// denies. Approve sends the browser back to the tool with a one-time code; Deny tells the tool
// no. A publishing tool needs the Publisher Agreement, shown here; at the limit of AI agents, one
// can be removed right here. No "confirm it's you" (the owner's decision of 2026-10-25). The
// site's pages can't be framed (frame-ancestors 'none'), so nobody can trick a click onto Approve.

import { useState } from "react";
import { Link, Navigate, useLocation, useParams, useSearchParams } from "react-router";

import { api, ApiError, type AgentToken, type AiConnectRequest, type AppSummary } from "../api";
import { LIFETIMES } from "../agentTokens";
import { useTermsGate } from "../components/Terms";
import { Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";

const PROBLEMS: Record<string, { title: string; text: string }> = {
  client: {
    title: "SLAPorSLOP doesn't know this AI tool",
    text: "The tool didn't say who it is in a way SLAPorSLOP can check, so it can't connect. Go back to it and try connecting again; if it keeps happening, the tool may not support signing in to SLAPorSLOP yet: make a token instead.",
  },
  redirect: {
    title: "This connection can't go on",
    text: "The AI tool asked SLAPorSLOP to send you somewhere it hadn't registered, so nothing was sent anywhere. Go back to your tool and try connecting again.",
  },
};

export function ConnectAi() {
  const { id } = useParams();
  const [params] = useSearchParams();
  useDocumentTitle("Connect an AI tool");
  if (!id) return <Problem kind={params.get("problem") ?? ""} />;
  return <Approve id={id} />;
}

function Problem({ kind }: { kind: string }) {
  const p = PROBLEMS[kind] ?? PROBLEMS.client;
  return (
    <div className="page narrow">
      <div className="panel panel-pad auth-card">
        <h1>{p.title}</h1>
        <p>{p.text}</p>
        <p>
          <Link to="/account/agents">Your AI agents</Link>
        </p>
      </div>
    </div>
  );
}

function Approve({ id }: { id: string }) {
  const location = useLocation();
  const { me } = useSession();
  const request = useLoad(() => (me ? api.aiConnect.request(id) : Promise.resolve(undefined)), [id, me?.id]);
  const apps = useLoad(() => (me ? api.myApps() : Promise.resolve([] as AppSummary[])), [me?.id]);

  if (me === undefined) return <Loading />;
  if (!me) return <Navigate to={`/signin?return_to=${encodeURIComponent(location.pathname)}`} replace />;
  if (request.error) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad auth-card">
          <h1>This connection request has expired</h1>
          <p>Go back to your AI tool and connect to SLAPorSLOP again. It opens a new page here.</p>
        </div>
      </div>
    );
  }
  if (!request.data) return <Loading />;
  return <ApproveForm id={id} request={request.data} apps={apps.data ?? []} who={me.display_name} />;
}

function ApproveForm({ id, request, apps, who }: { id: string; request: AiConnectRequest; apps: AppSummary[]; who: string }) {
  const terms = useTermsGate();
  const asked = request.requested_scopes;
  const [name, setName] = useState(request.client_name);
  // Where the choices start: what the tool asked for (all of it, if it didn't say).
  const [scope, setScope] = useState<AgentToken["scope"]>(asked.length === 0 || asked.includes("publish") ? "publish" : "read");
  const [only, setOnly] = useState<string[]>([]);
  const [people, setPeople] = useState(asked.includes("people"));
  const [days, setDays] = useState(30);
  const [state, setState] = useState<"ask" | "sending" | "done" | "denied">("ask");
  const [error, setError] = useState<string>();
  const [full, setFull] = useState(false);
  const tool = request.client_name;

  const go = (url: string) => window.location.assign(url);

  const approve = async () => {
    setState("sending");
    setError(undefined);
    try {
      const { redirect_url } = await terms.guard(() =>
        api.aiConnect.approve(id, { name: name.trim(), scope, apps: only, people, days }),
      );
      setState("done");
      go(redirect_url);
    } catch (e) {
      setState("ask");
      if (e instanceof ApiError && e.code === "conflict") setFull(true);
      setError(
        e instanceof ApiError && e.code === "terms_required"
          ? "Publishing needs the Publisher Agreement: accept it to approve, or choose Look."
          : e instanceof ApiError
            ? e.message
            : "Couldn't approve it. Try again.",
      );
    }
  };

  const deny = async () => {
    setState("sending");
    try {
      const { redirect_url } = await api.aiConnect.deny(id);
      setState("denied");
      go(redirect_url);
    } catch (e) {
      setState("ask");
      setError(e instanceof ApiError ? e.message : "Couldn't send the answer. Try again.");
    }
  };

  if (state === "done" || state === "denied") {
    return (
      <div className="page narrow">
        <div className="panel panel-pad auth-card">
          {state === "done" ? (
            <>
              <h1>Connected</h1>
              <p>
                Go back to {tool}: it can work on SLAPorSLOP now. You can close this tab. It's listed under{" "}
                <Link to="/account/agents">Account → AI agents</Link>, where you can remove it any time.
              </p>
            </>
          ) : (
            <>
              <h1>Not connected</h1>
              <p>{tool} wasn't given access. You can close this tab.</p>
            </>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="page narrow">
      <form
        className="panel panel-pad auth-card form connect-ai"
        aria-labelledby="connect-ai-title"
        onSubmit={(e) => {
          e.preventDefault();
          void approve();
        }}
      >
        <h1 id="connect-ai-title">Connect {tool} to SLAPorSLOP?</h1>
        <p>
          <b>{tool}</b> wants to work on SLAPorSLOP as <b>{who}</b>.{" "}
          {request.client_kind === "document" && request.client_host ? (
            <>It identifies itself from {request.client_host}.</>
          ) : (
            <span className="muted">That's the name it gives itself.</span>
          )}
        </p>
        <div className={`connect-ai-where${request.unfamiliar ? " warn" : ""}`} role="note">
          {request.redirect_kind === "loopback" ? (
            <p>
              Approving hands it a one-time code <b>on this computer</b>: a program running here receives it.
            </p>
          ) : (
            <p>
              Approving sends your browser to <b className="connect-ai-host">{request.redirect_host}</b>.
            </p>
          )}
          {request.unfamiliar && (
            <p>
              <b>SLAPorSLOP doesn't know {request.redirect_host}.</b> Only approve if you're connecting an AI tool on that website
              yourself, right now. Whoever runs it can then work on your apps as you.
            </p>
          )}
        </div>
        <p className="notice">
          Only continue if you just asked your AI tool to connect to SLAPorSLOP. If you didn't, someone may be trying to get into
          your account: choose <b>Deny</b>.
        </p>

        {terms.prompt}
        <label>
          Name it
          <input value={name} maxLength={60} required onChange={(e) => setName(e.target.value)} />
          <span className="hint">How it's listed under Account → AI agents.</span>
        </label>
        <label>
          It can
          <select value={scope} onChange={(e) => setScope(e.target.value as AgentToken["scope"])}>
            <option value="read">Look: apps, releases, checks, reviews</option>
            <option value="publish">Also change store pages and submit releases</option>
          </select>
        </label>
        {scope === "publish" && (
          <p className="small muted">Publishing needs the Publisher Agreement: if you haven't accepted it, it's shown when you approve.</p>
        )}
        <fieldset>
          <legend>Apps</legend>
          <label className="check">
            <input type="radio" checked={only.length === 0} onChange={() => setOnly([])} /> All my apps, and new ones
          </label>
          {apps.map((a) => (
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
          <input type="checkbox" checked={people} onChange={(e) => setPeople(e.target.checked)} /> People jobs too: see requests,
          send quotes, propose changes and talk in job conversations
        </label>
        <label>
          For how long
          <select value={days} onChange={(e) => setDays(Number(e.target.value))}>
            {LIFETIMES.map((l) => (
              <option key={l.days} value={l.days}>
                {l.label}
              </option>
            ))}
          </select>
          <span className="hint">After that it asks you to approve it again. You can remove it sooner any time.</span>
        </label>
        <p className="small muted">
          It can never pay, accept agreements, change payouts or your account. What it publishes goes through SLAPorSLOP's checks and
          review like anything else.
        </p>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        {full && <MakeRoom onRemoved={() => setFull(false)} />}
        <div className="actions">
          <button type="button" className="btn ghost" disabled={state === "sending"} onClick={() => void deny()}>
            Deny
          </button>
          <button type="submit" className="btn slime" disabled={state === "sending" || !name.trim()}>
            Approve
          </button>
        </div>
      </form>
    </div>
  );
}

/** At the limit of AI agents: remove one here, then approve. */
function MakeRoom({ onRemoved }: { onRemoved: () => void }) {
  const tokens = useLoad(() => api.agentTokens(), []);
  const [error, setError] = useState<string>();
  const remove = async (t: AgentToken) => {
    setError(undefined);
    try {
      await api.revokeAgentToken(t.id);
      tokens.reload();
      onRemoved();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't remove it.");
    }
  };
  const live = (tokens.data ?? []).filter((t) => !t.expired);
  return (
    <div className="panel panel-pad" role="region" aria-label="Your AI agents">
      <p className="small">Remove one you don't use any more, then choose Approve again:</p>
      <ul className="sessions">
        {live.map((t) => (
          <li key={t.id}>
            <div>
              <b>{t.name}</b>{" "}
              <span className="muted small">
                {t.connected ? `approved in the browser for ${t.connected.client}` : "token"} · last used {formatDate(t.last_used_at)}
              </span>
            </div>
            <button type="button" className="btn small" aria-label={`Remove ${t.name}`} onClick={() => void remove(t)}>
              Remove
            </button>
          </li>
        ))}
      </ul>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
