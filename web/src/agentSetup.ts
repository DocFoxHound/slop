// SPDX-License-Identifier: AGPL-3.0-or-later
// How to connect an AI tool to SLAPorSLOP (components/AgentSetup.tsx).
//
// The easy way (`EASY_TOOLS`, `easyCommand`): the person gives their AI tool SLAPorSLOP's MCP
// address, the tool opens SLAPorSLOP in their browser, they approve it there (crates/slop-api/src/
// agent_oauth.rs), and the tool keeps its own tokens. Nobody sees a token. Checked against each
// tool's documentation on 2026-10-27:
// - Claude Code (code.claude.com/docs/en/mcp; 2.1.301 installed, 2.1.311 current):
//   `claude mcp add --transport http NAME URL`, then `/mcp` in Claude Code (or
//   `claude mcp login NAME`) opens the browser. It names itself with its description at
//   claude.ai/oauth/claude-code-client-metadata.
// - Claude on the web, desktop and phone (claude.com/docs/connectors/custom/add-unlisted):
//   Customize → Connectors → Add custom connector, with the URL; Connect opens the sign-in.
// - ChatGPT (developers.openai.com/apps-sdk/deploy/connect-chatgpt): chatgpt.com/plugins, +, Add
//   custom MCP server, the URL, OAuth; then Create as a plugin.
// - Codex CLI (learn.chatgpt.com/docs/extend/mcp; rust-v0.164.0): `codex mcp add NAME --url URL`,
//   then `codex mcp login NAME`.
// - Cursor (cursor.com/docs/mcp): `{"mcpServers": {NAME: {"url": URL}}}` in ~/.cursor/mcp.json;
//   it signs in with OAuth when it connects.
// - Gemini CLI (google-gemini/gemini-cli docs/tools/mcp-server.md; v0.65.0):
//   `gemini mcp add --scope user --transport http NAME URL`, then `/mcp auth NAME`.
// - Grok Build (docs.x.ai/build/features/mcp-servers): `grok mcp add --transport http NAME URL`;
//   it opens the browser on first use (or `/mcps`, then i).
//
// The other way, for tools that can't sign in like that, is a token (`SETUPS`): the token never
// goes into an AI chat or a command the AI sees: the person saves it once in an environment
// variable in their own terminal, and each tool is connected with a reference to that variable.
// Only `saveCommand` contains the token itself. A maker's token (`slopmk_…`) goes in SLOP_TOKEN
// and connects to the maker MCP server; a staff member's (`slopadm_…`, Admin → AI reviewers) goes
// in SLOP_ADMIN_TOKEN and connects to the admin one (`SETUPS`).
//
// Checked against the tools themselves (2026-10-25):
// - Claude Code (code.claude.com/docs/en/mcp; tried with 2.1.301): `claude mcp add` stores the
//   header as typed, and expands `${VAR}` in a server's url and headers when it connects, for
//   user-scoped servers in ~/.claude.json too. Single quotes keep the shell (bash, zsh, PowerShell)
//   from putting the token itself into the config.
// - Codex CLI (learn.chatgpt.com/docs/extend/mcp; `codex mcp add --help`, 0.163.0):
//   `--bearer-token-env-var` stores `bearer_token_env_var = "SLOP_TOKEN"` in ~/.codex/config.toml.
// - Grok Build (docs.x.ai/build/features/mcp-servers): `grok mcp add --transport http … --header`,
//   with `${VAR}` expanded in headers when the config loads.

/** The AI tools the easy way covers. */
export type EasyTool = "claude-code" | "claude" | "chatgpt" | "codex" | "cursor" | "gemini" | "grok" | "other";

export const EASY_TOOLS: { id: EasyTool; label: string }[] = [
  { id: "claude-code", label: "Claude Code" },
  { id: "claude", label: "Claude app" },
  { id: "chatgpt", label: "ChatGPT" },
  { id: "codex", label: "Codex" },
  { id: "cursor", label: "Cursor" },
  { id: "gemini", label: "Gemini CLI" },
  { id: "grok", label: "Grok Build" },
  { id: "other", label: "Other AI tools" },
];

/** The server's name in the person's AI tool, the easy way. */
export const EASY_SERVER = "slaporslop";

/** SLAPorSLOP's MCP address for AI tools: the short one, which the tools are given. */
export function mcpAddress(origin: string): string {
  return `${origin.replace(/\/+$/, "")}/mcp`;
}

/**
 * The one command that adds SLAPorSLOP to a tool, with no token: the tool asks for approval in the
 * browser the first time it connects. Undefined for tools set up in their own settings.
 */
export function easyCommand(tool: EasyTool, mcpUrl: string): string | undefined {
  switch (tool) {
    case "claude-code":
      return `claude mcp add --transport http --scope user ${EASY_SERVER} ${mcpUrl}`;
    case "codex":
      return `codex mcp add ${EASY_SERVER} --url ${mcpUrl}`;
    case "gemini":
      return `gemini mcp add --scope user --transport http ${EASY_SERVER} ${mcpUrl}`;
    case "grok":
      return `grok mcp add --transport http ${EASY_SERVER} ${mcpUrl}`;
    default:
      return undefined;
  }
}

/** What starts the browser sign-in after `easyCommand`, where the tool has a command for it. */
export function signInCommand(tool: EasyTool): string | undefined {
  switch (tool) {
    case "claude-code":
      return `claude mcp login ${EASY_SERVER}`;
    case "codex":
      return `codex mcp login ${EASY_SERVER}`;
    case "gemini":
      return `/mcp auth ${EASY_SERVER}`;
    default:
      return undefined;
  }
}

/** Cursor's settings file entry. */
export function cursorConfig(mcpUrl: string): string {
  return JSON.stringify({ mcpServers: { [EASY_SERVER]: { url: mcpUrl } } }, null, 2);
}

export type SetupOs = "windows" | "macos" | "linux";
export type SetupTool = "claude" | "codex" | "grok" | "other";

export const OSES: { id: SetupOs; label: string }[] = [
  { id: "windows", label: "Windows" },
  { id: "macos", label: "macOS" },
  { id: "linux", label: "Linux" },
];

export const TOOLS: { id: SetupTool; label: string }[] = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex (OpenAI)" },
  { id: "grok", label: "Grok Build (xAI)" },
  { id: "other", label: "Other AI tools" },
];

/** What differs between a maker's token and a staff member's. */
export interface SetupKind {
  /** The environment variable the token is saved in. */
  variable: string;
  /** The MCP server's name in the AI tool. */
  server: string;
  /** What to ask the AI once it's connected: no token needed. */
  firstPrompt: string;
  /** Where else the token works, for tools without MCP. */
  apiGuide: string;
  /** Whose account the token acts for, in the warning. */
  account: string;
  /** What the page's button for ending a token says. */
  remove: "remove" | "revoke";
}

export const SETUPS = {
  maker: {
    variable: "SLOP_TOKEN",
    server: "slop",
    firstPrompt: "Read maker_guide, then publish my game on SLAPorSLOP.",
    apiGuide: "the publishing guide, “Working with your AI agent”",
    account: "your SLAPorSLOP account",
    remove: "remove",
  },
  staff: {
    variable: "SLOP_ADMIN_TOKEN",
    server: "slop-admin",
    firstPrompt: "Read review_guide, then go through the review queue and tell me what needs me.",
    apiGuide: "the reviewer guide at /v1/admin/review/guide",
    account: "your SLAPorSLOP staff account, with your admin rights",
    remove: "revoke",
  },
} satisfies Record<string, SetupKind>;

const MAKER: SetupKind = SETUPS.maker;

/** The environment variable a maker's token is saved in. */
export const VARIABLE = MAKER.variable;

/** The visitor's system, from the browser (a guess they can change). */
export function guessOs(userAgent: string): SetupOs {
  if (/Windows/i.test(userAgent)) return "windows";
  if (/Mac OS X|Macintosh/i.test(userAgent)) return "macos";
  return "linux";
}

/** The shell's start-up file on macOS (zsh) and Linux (usually bash). */
export function profileFile(os: SetupOs): string {
  return os === "macos" ? "~/.zshrc" : "~/.bashrc";
}

/** How to open a terminal, in plain words. */
export function openTerminal(os: SetupOs): string {
  switch (os) {
    case "windows":
      return "Press the Windows key, type PowerShell, and open Windows PowerShell (or open Terminal, which starts PowerShell).";
    case "macos":
      return "Press ⌘ Command and Space together, type Terminal, and press Return.";
    case "linux":
      return "Open your Terminal app (on many systems: Ctrl+Alt+T).";
  }
}

/** The one command with the token in it: saves it on this computer, for this user. */
export function saveCommand(os: SetupOs, secret: string, kind: SetupKind = MAKER): string {
  const v = kind.variable;
  if (os === "windows") return `setx ${v} "${secret}"`;
  return `echo 'export ${v}="${secret}"' >> ${profileFile(os)}`;
}

/** Says whether the token is saved, without showing it. */
export function checkCommand(os: SetupOs, kind: SetupKind = MAKER): string {
  const v = kind.variable;
  if (os === "windows") return `if ($env:${v}) { "${v} is saved" } else { "${v} is missing" }`;
  return `[ -n "$${v}" ] && echo "${v} is saved" || echo "${v} is missing"`;
}

/**
 * Connects a tool to SLAPorSLOP's MCP address using the variable, never the token. The same on every
 * system (PowerShell reads single quotes the way bash and zsh do). Undefined: no command for it.
 */
export function connectCommand(tool: SetupTool, mcpUrl: string, kind: SetupKind = MAKER): string | undefined {
  const { variable: v, server } = kind;
  switch (tool) {
    case "claude":
      return `claude mcp add --transport http --scope user ${server} ${mcpUrl} --header 'Authorization: Bearer \${${v}}'`;
    case "codex":
      return `codex mcp add ${server} --url ${mcpUrl} --bearer-token-env-var ${v}`;
    case "grok":
      return `grok mcp add --transport http ${server} ${mcpUrl} --header 'Authorization: Bearer \${${v}}'`;
    case "other":
      return undefined;
  }
}

/** Codex's config file, for anyone who'd rather edit it than run `codex mcp add`. */
export function codexConfig(mcpUrl: string, kind: SetupKind = MAKER): string {
  return `[mcp_servers.${kind.server}]\nurl = "${mcpUrl}"\nbearer_token_env_var = "${kind.variable}"`;
}

/** What to ask the AI once a maker's token is connected: no token needed. */
export const FIRST_PROMPT = MAKER.firstPrompt;
