// SPDX-License-Identifier: AGPL-3.0-or-later
// People skill icons (tagIcons.ts SKILL_SHAPES, SkillIcon): every skill the server offers has one.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { SKILL_ICON_NAMES, skillIconPaths } from "./tagIcons";

/** The skills in crates/slop-api/src/people/mod.rs SKILLS. */
function serverSkills(): string[] {
  const src = readFileSync(join(__dirname, "../../crates/slop-api/src/people/mod.rs"), "utf8");
  const list = src.match(/pub const SKILLS: &\[\(&str, &str\)\] = &\[([\s\S]*?)\];/)![1];
  return [...list.matchAll(/\("([a-z0-9-]+)", "[^"]+"\)/g)].map((m) => m[1]);
}

describe("skill icons", () => {
  it("cover every skill, plus everyone", () => {
    const skills = serverSkills();
    expect(skills.length).toBeGreaterThan(5);
    expect(new Set(SKILL_ICON_NAMES)).toEqual(new Set(["everyone", ...skills]));
  });
  it("are all different, and an unknown skill gets everyone's", () => {
    const drawn = SKILL_ICON_NAMES.map((s) => JSON.stringify(skillIconPaths(s)));
    expect(new Set(drawn).size).toBe(drawn.length);
    expect(skillIconPaths("juggling")).toEqual(skillIconPaths("everyone"));
  });
});
