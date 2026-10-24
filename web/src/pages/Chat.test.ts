// SPDX-License-Identifier: AGPL-3.0-or-later
// Setting chat up by itself (ensureChat): the app's chat engine is faked.
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ChatStatus } from "../platform";

const engine = vi.hoisted(() => ({ state: "none" as ChatStatus["state"], becomes: "active" as ChatStatus["state"], calls: [] as Array<Record<string, unknown>> }));

vi.mock("../platform", async (original) => {
  const real = await original<typeof import("../platform")>();
  const status = (): ChatStatus => ({ state: engine.state, device_id: null, code: null, pending: [], devices: [], backups: false });
  return {
    ...real,
    desktop: {
      ...real.desktop,
      chat: async (a: Record<string, unknown>) => {
        engine.calls.push(a);
        // Answers come back later, like the app's.
        await new Promise((r) => setTimeout(r, 5));
        if (a.action === "status") return status();
        if (a.action === "setup") {
          engine.state = engine.becomes;
          return status();
        }
        if (a.action === "set_backups") return a.on ? "CODE-1" : null;
        return null;
      },
    },
  };
});

const { ensureChat } = await import("./Chat");
const actions = () => engine.calls.map((c) => c.action);

beforeEach(() => {
  engine.state = "none";
  engine.becomes = "active";
  engine.calls = [];
});

describe("ensureChat", () => {
  it("sets up a device that never had chat, without a name, and turns backups on for the first", async () => {
    const s = await ensureChat();
    expect(s.state).toBe("active");
    expect(s.backups).toBe(true);
    expect(engine.calls.find((c) => c.action === "setup")).toEqual({ action: "setup" });
    expect(actions()).toEqual(["status", "setup", "set_backups"]);
  });

  it("sets up once when asked from several places at the same time", async () => {
    const [a, b] = await Promise.all([ensureChat(), ensureChat()]);
    expect(a).toBe(b);
    expect(actions().filter((x) => x === "setup")).toHaveLength(1);
    expect(actions().filter((x) => x === "set_backups")).toHaveLength(1);
    // Later calls find it set up and leave it.
    engine.calls = [];
    expect((await ensureChat()).state).toBe("active");
    expect(actions()).toEqual(["status"]);
  });

  it("leaves a later device waiting for approval, with no recovery code", async () => {
    engine.becomes = "pending";
    expect((await ensureChat()).state).toBe("pending");
    expect(actions()).toEqual(["status", "setup"]);
  });

  it("sets a removed device up again only when asked", async () => {
    engine.state = "revoked";
    expect((await ensureChat()).state).toBe("revoked");
    expect(actions()).toEqual(["status"]);
    expect((await ensureChat(true)).state).toBe("active");
    expect(actions()).toEqual(["status", "status", "setup", "set_backups"]);
  });
});
