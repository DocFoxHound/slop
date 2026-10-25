// SPDX-License-Identifier: AGPL-3.0-or-later
// How to connect an AI tool to Slop with a maker's token (components/AgentSetup.tsx) without the
// token ever going into an AI chat or a command the AI sees: the person saves it once in an
// environment variable, SLOP_TOKEN, in their own terminal, and each tool is connected with a
// reference to that variable. Only `saveCommand` contains the token itself.
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

/** The environment variable the token is saved in. */
export const VARIABLE = "SLOP_TOKEN";

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
export function saveCommand(os: SetupOs, secret: string): string {
  if (os === "windows") return `setx ${VARIABLE} "${secret}"`;
  return `echo 'export ${VARIABLE}="${secret}"' >> ${profileFile(os)}`;
}

/** Says whether the token is saved, without showing it. */
export function checkCommand(os: SetupOs): string {
  if (os === "windows") return `if ($env:${VARIABLE}) { "${VARIABLE} is saved" } else { "${VARIABLE} is missing" }`;
  return `[ -n "$${VARIABLE}" ] && echo "${VARIABLE} is saved" || echo "${VARIABLE} is missing"`;
}

/**
 * Connects a tool to Slop's MCP address using the variable, never the token. The same on every
 * system (PowerShell reads single quotes the way bash and zsh do). Undefined: no command for it.
 */
export function connectCommand(tool: SetupTool, mcpUrl: string): string | undefined {
  switch (tool) {
    case "claude":
      return `claude mcp add --transport http --scope user slop ${mcpUrl} --header 'Authorization: Bearer \${${VARIABLE}}'`;
    case "codex":
      return `codex mcp add slop --url ${mcpUrl} --bearer-token-env-var ${VARIABLE}`;
    case "grok":
      return `grok mcp add --transport http slop ${mcpUrl} --header 'Authorization: Bearer \${${VARIABLE}}'`;
    case "other":
      return undefined;
  }
}

/** Codex's config file, for anyone who'd rather edit it than run `codex mcp add`. */
export function codexConfig(mcpUrl: string): string {
  return `[mcp_servers.slop]\nurl = "${mcpUrl}"\nbearer_token_env_var = "${VARIABLE}"`;
}

/** What to ask the AI once it's connected: no token needed. */
export const FIRST_PROMPT = "Read maker_guide, then publish my game on Slop.";
