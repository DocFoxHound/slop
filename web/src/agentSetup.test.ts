// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

import { OSES, TOOLS, checkCommand, codexConfig, connectCommand, guessOs, saveCommand } from "./agentSetup";

const SECRET = "slopmk_abc123DEF-456_xyz";
const URL = "https://slopai.games/v1/maker/mcp";

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
      "claude mcp add --transport http --scope user slop https://slopai.games/v1/maker/mcp --header 'Authorization: Bearer ${SLOP_TOKEN}'",
    );
    expect(connectCommand("codex", URL)).toBe("codex mcp add slop --url https://slopai.games/v1/maker/mcp --bearer-token-env-var SLOP_TOKEN");
    expect(connectCommand("grok", URL)).toBe(
      "grok mcp add --transport http slop https://slopai.games/v1/maker/mcp --header 'Authorization: Bearer ${SLOP_TOKEN}'",
    );
    expect(connectCommand("other", URL)).toBeUndefined();
    expect(codexConfig(URL)).toBe('[mcp_servers.slop]\nurl = "https://slopai.games/v1/maker/mcp"\nbearer_token_env_var = "SLOP_TOKEN"');
  });

  it("guesses the computer from the browser", () => {
    expect(guessOs("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows");
    expect(guessOs("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)")).toBe("macos");
    expect(guessOs("Mozilla/5.0 (X11; Linux x86_64)")).toBe("linux");
  });
});
