// SPDX-License-Identifier: AGPL-3.0-or-later
// How to connect an AI tool to SLAPorSLOP with an agent token (components/AgentSetup.tsx) without the
// token ever going into an AI chat or a command the AI sees: the person saves it once in an
// environment variable in their own terminal, and each tool is connected with a reference to that
// variable. Only `saveCommand` contains the token itself. A maker's token (`slopmk_…`) goes in
// SLOP_TOKEN and connects to the maker MCP server; a staff member's (`slopadm_…`, Admin → AI
// reviewers) goes in SLOP_ADMIN_TOKEN and connects to the admin one (`SETUPS`).
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
