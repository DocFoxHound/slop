// SPDX-License-Identifier: AGPL-3.0-or-later
// After an AI agent token is made (AiAgents.tsx for makers, Admin → AI reviewers for staff): how
// to set it up on your own computer, step by step, for people who've never used a terminal. The token goes into an environment variable
// once, in the person's own terminal; every AI tool is then connected with a reference to that
// variable (agentSetup.ts), so the token never goes into an AI chat or a command the AI sees.

import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import {
  OSES,
  SETUPS,
  TOOLS,
  checkCommand,
  codexConfig,
  connectCommand,
  guessOs,
  openTerminal,
  profileFile,
  saveCommand,
  type SetupKind,
  type SetupOs,
  type SetupTool,
} from "../agentSetup";

export interface MadeToken {
  secret: string;
  mcp_url: string;
  name: string;
}

/** `kind`: a maker's token (the default) or a staff member's (`SETUPS.staff`). */
export function AgentSetup({
  token,
  footer,
  kind = SETUPS.maker,
}: {
  token: MadeToken;
  footer?: ReactNode;
  kind?: SetupKind;
}) {
  const VARIABLE = kind.variable;
  const [os, setOs] = useState<SetupOs>(() => guessOs(typeof navigator === "undefined" ? "" : navigator.userAgent));
  const [tool, setTool] = useState<SetupTool>("claude");
  const id = useId();

  return (
    <div className="panel panel-pad agent-setup" aria-labelledby={`${id}-title`} role="region">
      <h4 className="agent-setup-title" id={`${id}-title`}>
        Set up “{token.name}” on your computer
      </h4>
      <div className="agent-setup-warning" role="note">
        <p>
          <b>Never paste this token into a chat with Claude, ChatGPT, Codex, Grok or any AI.</b>
        </p>
        <p>It's a password for {kind.account}. Set it up on your computer instead, with the steps below.</p>
      </div>

      <div className="agent-setup-token">
        <p className="small">
          <b>Your token.</b> Copy it now: Slop won't show this token again. If you lose it, {kind.remove} it below and
          make a new one.
        </p>
        <CopyRow text={token.secret} what="the token" secret />
      </div>

      <ol className="agent-setup-steps">
        <li>
          <h5>Open a terminal on your computer</h5>
          <Tabs label="Your computer" items={OSES} current={os} onChange={setOs} idBase={`${id}-os`}>
            <p className="small">{openTerminal(os)}</p>
            <p className="small">
              Type or paste the commands below <b>into that terminal window</b>, then press Enter. Not into the AI chat,
              and not after Claude Code's <code>!</code> (that puts the command, and your token, into the conversation).
            </p>
          </Tabs>
        </li>
        <li>
          <h5>Save the token on your computer</h5>
          {os === "windows" ? (
            <>
              <p className="small">In PowerShell, run this. It's the only step with the token in it:</p>
              <CopyRow text={saveCommand(os, token.secret, kind)} what="the command that saves your token" />
              <details className="small agent-setup-more">
                <summary>Or without a command: Windows settings</summary>
                <ol>
                  <li>Press the Windows key and type <b>environment variables</b>.</li>
                  <li>
                    Open <b>Edit environment variables for your account</b>.
                  </li>
                  <li>
                    Under <b>User variables</b>, click <b>New…</b>.
                  </li>
                  <li>
                    Variable name: <code>{VARIABLE}</code>. Variable value: paste your token.
                  </li>
                  <li>
                    Click <b>OK</b>, then <b>OK</b> again.
                  </li>
                </ol>
              </details>
            </>
          ) : (
            <>
              <p className="small">Run this. It's the only step with the token in it:</p>
              <CopyRow text={saveCommand(os, token.secret, kind)} what="the command that saves your token" />
              <p className="small muted">
                It adds the token to {profileFile(os)}, which your terminal reads when it opens.
                {os === "linux" ? " If your terminal uses zsh, put ~/.zshrc instead." : ""}
              </p>
            </>
          )}
          <p className="small">
            Then <b>close the terminal and open a new one</b>: only new windows see the saved token. To check it's there
            (without showing it):
          </p>
          <CopyRow text={checkCommand(os, kind)} what="the command that checks the token is saved" />
        </li>
        <li>
          <h5>Connect your AI tool</h5>
          <Tabs label="Your AI tool" items={TOOLS} current={tool} onChange={setTool} idBase={`${id}-tool`}>
            <ConnectSteps tool={tool} os={os} mcpUrl={token.mcp_url} kind={kind} />
          </Tabs>
        </li>
        <li>
          <h5>Start working</h5>
          <p className="small">Open your AI tool and ask it (no token needed in the chat):</p>
          <CopyRow text={kind.firstPrompt} what="what to ask your AI" />
        </li>
      </ol>
      {footer}
    </div>
  );
}

function ConnectSteps({ tool, os, mcpUrl, kind }: { tool: SetupTool; os: SetupOs; mcpUrl: string; kind: SetupKind }) {
  const VARIABLE = kind.variable;
  const server = kind.server;
  const command = connectCommand(tool, mcpUrl, kind);
  const shell = os === "windows" ? "In PowerShell (not the old Command Prompt), run" : "In the new terminal, run";
  switch (tool) {
    case "claude":
      return (
        <>
          <p className="small">{shell}:</p>
          <CopyRow text={command!} what="the Claude Code command" />
          <p className="small muted">
            Keep the single quotes: they make Claude Code save a reference to {VARIABLE}, not the token, and read it each
            time it starts.
          </p>
          <p className="small">
            Then start Claude Code from a new terminal (type <code>claude</code>) and type <code>/mcp</code>: <b>{server}</b>{" "}
            should say connected. If Claude Code was already open, quit it and start it again.
          </p>
          <NotConnecting os={os} kind={kind} />
        </>
      );
    case "codex":
      return (
        <>
          <p className="small">{shell}:</p>
          <CopyRow text={command!} what="the Codex command" />
          <p className="small muted">Codex saves only the name {VARIABLE} and reads the token from it when it starts.</p>
          <p className="small">
            Then start Codex from a new terminal (type <code>codex</code>) and type <code>/mcp</code> to see {server} listed.
          </p>
          <details className="small agent-setup-more">
            <summary>Or add it to Codex's settings file</summary>
            <p>
              Add this to <code>{os === "windows" ? "%USERPROFILE%\\.codex\\config.toml" : "~/.codex/config.toml"}</code>:
            </p>
            <CopyRow text={codexConfig(mcpUrl, kind)} what="the Codex settings" />
          </details>
          <NotConnecting os={os} kind={kind} />
        </>
      );
    case "grok":
      return (
        <>
          <p className="small">{shell}:</p>
          <CopyRow text={command!} what="the Grok Build command" />
          <p className="small muted">Keep the single quotes: Grok Build then reads the token from {VARIABLE} when it starts.</p>
          <p className="small">
            Then run <code>grok mcp doctor {server}</code> to check it connects, and start Grok Build from a new terminal (type{" "}
            <code>grok</code>).
          </p>
          <NotConnecting os={os} kind={kind} />
        </>
      );
    case "other":
      return (
        <>
          <p className="small">
            Look in your tool's settings for <b>MCP servers</b>, <b>connectors</b> or <b>integrations</b>, and add a server
            with:
          </p>
          <dl className="agent-setup-details small">
            <dt>Address (URL)</dt>
            <dd>
              <CopyRow text={mcpUrl} what="the MCP address" />
            </dd>
            <dt>Header</dt>
            <dd>
              <code>Authorization</code> with the value <code>Bearer</code>, a space, then your token. If the tool can read
              it from an environment variable, use <code>{VARIABLE}</code>.
            </dd>
          </dl>
          <p className="small">
            Tools that call web APIs instead can use Slop's API with the same header (see {kind.apiGuide}).
          </p>
          <p className="small">
            <b>If a tool asks for the token in its chat, don't paste it there:</b> look for its MCP or connector settings
            instead.
          </p>
        </>
      );
  }
}

function NotConnecting({ os, kind }: { os: SetupOs; kind: SetupKind }) {
  return (
    <details className="small agent-setup-more">
      <summary>If it doesn't connect</summary>
      <ul>
        <li>Check the token is saved (step 2), in a new terminal window.</li>
        <li>
          Using it inside an editor such as VS Code? Quit the editor completely and open it again, so it sees the saved
          token.
          {os === "macos" ? " If it still can't, start the editor from a new Terminal window (for VS Code: type code)." : ""}
        </li>
        <li>
          Still nothing? {kind.remove === "revoke" ? "Revoke" : "Remove"} the token below, make a new one, and follow these
          steps again.
        </li>
      </ul>
    </details>
  );
}

/** Text to copy, with a Copy button named for `what` it copies, that says when it's done. */
function CopyRow({ text, what, secret = false }: { text: string; what: string; secret?: boolean }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  };
  return (
    <div className="copy-row">
      <pre className={`code-block copy-row-text${secret ? " copy-row-secret" : ""}`} tabIndex={0}>
        {text}
      </pre>
      <button type="button" className="btn small" onClick={() => void copy()}>
        {copied ? "Copied" : "Copy"}
        <span className="sr-only"> {what}</span>
      </button>
      <span className="sr-only" aria-live="polite">
        {copied ? "Copied" : ""}
      </span>
    </div>
  );
}

/** Tabs: arrow keys, Home and End move between them (WAI-ARIA tabs pattern). */
function Tabs<T extends string>({
  label,
  items,
  current,
  onChange,
  idBase,
  children,
}: {
  label: string;
  items: { id: T; label: string }[];
  current: T;
  onChange: (id: T) => void;
  idBase: string;
  children: ReactNode;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const at = items.findIndex((i) => i.id === current);
    const next =
      e.key === "ArrowRight" ? (at + 1) % items.length
      : e.key === "ArrowLeft" ? (at - 1 + items.length) % items.length
      : e.key === "Home" ? 0
      : e.key === "End" ? items.length - 1
      : -1;
    if (next < 0) return;
    e.preventDefault();
    onChange(items[next].id);
    refs.current[next]?.focus();
  };
  return (
    <>
      <div className="seg wrap agent-setup-tabs" role="tablist" aria-label={label} onKeyDown={onKey}>
        {items.map((i, n) => (
          <button
            key={i.id}
            ref={(el) => {
              refs.current[n] = el;
            }}
            type="button"
            role="tab"
            id={`${idBase}-${i.id}`}
            aria-selected={i.id === current}
            aria-controls={`${idBase}-panel`}
            tabIndex={i.id === current ? 0 : -1}
            className={i.id === current ? "on" : ""}
            onClick={() => onChange(i.id)}
          >
            {i.label}
          </button>
        ))}
      </div>
      <div className="agent-setup-panel" role="tabpanel" id={`${idBase}-panel`} aria-labelledby={`${idBase}-${current}`}>
        {children}
      </div>
    </>
  );
}
