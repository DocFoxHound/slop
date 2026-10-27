// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import {
  EASY_TOOLS,
  OSES,
  SETUPS,
  TOOLS,
  checkCommand,
  codexConfig,
  connectCommand,
  cursorConfig,
  easyCommand,
  guessOs,
  mcpAddress,
  saveCommand,
  signInCommand,
} from "./agentSetup";

const SECRET = "slopmk_abc123DEF-456_xyz";
const URL = "https://slaporslop.games/v1/maker/mcp";

describe("setting up an AI tool without pasting the token anywhere", () => {
  it("only the save step has the token in it", () => {
    for (const { id: os } of OSES) {
      expect(saveCommand(os, SECRET)).toContain(SECRET);
      expect(checkCommand(os)).not.toContain("slopmk_");
      for (const { id: tool } of TOOLS) {
        expect(connectCommand(tool, URL) ?? "").not.toContain("slopmk_");
      }
    }
    expect(codexConfig(URL)).not.toContain("slopmk_");
  });

  it("saves it in SLOP_TOKEN for this user", () => {
    expect(saveCommand("windows", SECRET)).toBe(`setx SLOP_TOKEN "${SECRET}"`);
    expect(saveCommand("macos", SECRET)).toBe(`echo 'export SLOP_TOKEN="${SECRET}"' >> ~/.zshrc`);
    expect(saveCommand("linux", SECRET)).toBe(`echo 'export SLOP_TOKEN="${SECRET}"' >> ~/.bashrc`);
    // Checking never prints it.
    expect(checkCommand("windows")).toBe('if ($env:SLOP_TOKEN) { "SLOP_TOKEN is saved" } else { "SLOP_TOKEN is missing" }');
    expect(checkCommand("linux")).toBe('[ -n "$SLOP_TOKEN" ] && echo "SLOP_TOKEN is saved" || echo "SLOP_TOKEN is missing"');
  });

  it("connects each tool with a reference to the variable", () => {
    // Single quotes: the shell leaves ${SLOP_TOKEN} for Claude Code to read when it connects.
    expect(connectCommand("claude", URL)).toBe(
      "claude mcp add --transport http --scope user slop https://slaporslop.games/v1/maker/mcp --header 'Authorization: Bearer ${SLOP_TOKEN}'",
    );
    expect(connectCommand("codex", URL)).toBe("codex mcp add slop --url https://slaporslop.games/v1/maker/mcp --bearer-token-env-var SLOP_TOKEN");
    expect(connectCommand("grok", URL)).toBe(
      "grok mcp add --transport http slop https://slaporslop.games/v1/maker/mcp --header 'Authorization: Bearer ${SLOP_TOKEN}'",
    );
    expect(connectCommand("other", URL)).toBeUndefined();
    expect(codexConfig(URL)).toBe('[mcp_servers.slop]\nurl = "https://slaporslop.games/v1/maker/mcp"\nbearer_token_env_var = "SLOP_TOKEN"');
  });

  it("a staff token goes in SLOP_ADMIN_TOKEN and connects to the admin tools", () => {
    const staff = SETUPS.staff;
    const secret = "slopadm_abc123DEF-456_xyz";
    const admin = "https://slaporslop.games/v1/admin/mcp";
    expect(saveCommand("windows", secret, staff)).toBe(`setx SLOP_ADMIN_TOKEN "${secret}"`);
    expect(saveCommand("linux", secret, staff)).toBe(`echo 'export SLOP_ADMIN_TOKEN="${secret}"' >> ~/.bashrc`);
    expect(checkCommand("macos", staff)).toBe(
      '[ -n "$SLOP_ADMIN_TOKEN" ] && echo "SLOP_ADMIN_TOKEN is saved" || echo "SLOP_ADMIN_TOKEN is missing"',
    );
    expect(connectCommand("claude", admin, staff)).toBe(
      "claude mcp add --transport http --scope user slop-admin https://slaporslop.games/v1/admin/mcp --header 'Authorization: Bearer ${SLOP_ADMIN_TOKEN}'",
    );
    expect(connectCommand("codex", admin, staff)).toBe(
      "codex mcp add slop-admin --url https://slaporslop.games/v1/admin/mcp --bearer-token-env-var SLOP_ADMIN_TOKEN",
    );
    expect(codexConfig(admin, staff)).toBe(
      '[mcp_servers.slop-admin]\nurl = "https://slaporslop.games/v1/admin/mcp"\nbearer_token_env_var = "SLOP_ADMIN_TOKEN"',
    );
    // Only the save step has the token.
    for (const { id: os } of OSES) {
      expect(checkCommand(os, staff)).not.toContain("slopadm_");
      for (const { id: tool } of TOOLS) expect(connectCommand(tool, admin, staff) ?? "").not.toContain("slopadm_");
    }
    expect(staff.firstPrompt).toMatch(/^Read review_guide, then /);
  });

  it("guesses the computer from the browser", () => {
    expect(guessOs("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows");
    expect(guessOs("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)")).toBe("macos");
    expect(guessOs("Mozilla/5.0 (X11; Linux x86_64)")).toBe("linux");
  });
});

describe("the easy way: approving the tool in the browser, no token", () => {
  const MCP = "https://slaporslop.games/mcp";

  it("uses the short MCP address", () => {
    expect(mcpAddress("https://slaporslop.games")).toBe(MCP);
    expect(mcpAddress("https://slaporslop.games/")).toBe(MCP);
    expect(mcpAddress("http://127.0.0.1:8080")).toBe("http://127.0.0.1:8080/mcp");
  });

  it("adds SLAPorSLOP to each tool with one command and no token or header", () => {
    expect(easyCommand("claude-code", MCP)).toBe("claude mcp add --transport http --scope user slaporslop https://slaporslop.games/mcp");
    expect(easyCommand("codex", MCP)).toBe("codex mcp add slaporslop --url https://slaporslop.games/mcp");
    expect(easyCommand("gemini", MCP)).toBe("gemini mcp add --scope user --transport http slaporslop https://slaporslop.games/mcp");
    expect(easyCommand("grok", MCP)).toBe("grok mcp add --transport http slaporslop https://slaporslop.games/mcp");
    // Set up in the tool's own settings instead.
    for (const tool of ["claude", "chatgpt", "cursor", "other"] as const) expect(easyCommand(tool, MCP)).toBeUndefined();
    for (const { id } of EASY_TOOLS) {
      const command = easyCommand(id, MCP) ?? "";
      expect(command).not.toMatch(/slopmk_|SLOP_TOKEN|Authorization|--header|bearer/i);
    }
    expect(JSON.parse(cursorConfig(MCP))).toEqual({ mcpServers: { slaporslop: { url: MCP } } });
  });

  it("says how each tool starts the browser sign-in, where it has a command", () => {
    expect(signInCommand("claude-code")).toBe("claude mcp login slaporslop");
    expect(signInCommand("codex")).toBe("codex mcp login slaporslop");
    expect(signInCommand("gemini")).toBe("/mcp auth slaporslop");
    expect(signInCommand("claude")).toBeUndefined();
    expect(signInCommand("chatgpt")).toBeUndefined();
  });

  it("lists the easy way's tools, with Claude Code first", () => {
    expect(EASY_TOOLS.map((t) => t.id)).toEqual(["claude-code", "claude", "chatgpt", "codex", "cursor", "gemini", "grok", "other"]);
  });
});
