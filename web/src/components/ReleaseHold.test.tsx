// SPDX-License-Identifier: AGPL-3.0-or-later
// A release on hold, as its maker sees it: what holds it (with what was found), Scan again with
// the re-runs left and when the next is allowed, writing to SLAPorSLOP staff and their answers, a
// lowered rating, a missing Defender check (on GitHub and outside it), withdrawing it, and asking
// staff about a failed or turned-down release.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type MakerHold, type Release } from "../api";
import {
  AskAboutFailed,
  HOLD_ENDED,
  makerLine,
  newVersionLine,
  ReleaseHold,
  rescanLine,
  withoutRepeatedReason,
  writingFor,
} from "./ReleaseHold";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const release = (over: Partial<Release> = {}): Release => ({
  id: "rel1",
  version: "1.1.0",
  commit_sha: "a".repeat(40),
  status: "awaiting_review",
  status_reason: "On hold: a scanner flagged the Windows (.zip) build",
  commit_verified: true,
  created_at: "2026-11-07T10:00:00Z",
  source: "maker",
  auto_approved: false,
  files: [],
  source_checks: null,
  repo_scan: null,
  advisories: [],
  hold: { status: "open", kinds: ["scan"], needs_staff: true, rescans_left: 3, unanswered: false },
  ...over,
});

const hold = (over: Partial<MakerHold> = {}): MakerHold => ({
  release_id: "rel1",
  version: "1.1.0",
  release_status: "awaiting_review",
  status: "open",
  kinds: ["scan"],
  needs_staff: true,
  problems: [
    {
      kind: "scan",
      platform: "windows",
      file_label: "Windows (.zip)",
      what: "A scanner flagged the Windows (.zip) build",
      explained: "ClamAV, a virus scanner, matched the signature `Win.Test.Seed` in `pong.exe`. Scanners sometimes match harmless programs.",
      you_can: ["rescan", "write_to_staff", "new_release"],
    },
  ],
  you_can: ["rescan", "write_to_staff", "new_release", "withdraw"],
  rescans_left: 3,
  next_rescan_at: null,
  rescan_requested: false,
  rescan_running: false,
  asked_staff_at: null,
  unanswered: false,
  messages: [{ from: "slaporslop", by_agent: false, body: "On hold: a scanner flagged the Windows (.zip) build.", at: "2026-11-07T10:00:00Z" }],
  ...over,
});

function show(r: Release, onChanged = vi.fn()) {
  render(
    <MemoryRouter>
      <ReleaseHold slug="pocket-golf" release={r} onChanged={onChanged} />
    </MemoryRouter>,
  );
  return onChanged;
}

describe("a release on hold, for its maker", () => {
  it("says what holds it in plain words, with what was found, and that staff look at it", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold());
    show(release());
    const section = await screen.findByRole("region", { name: "Why it's on hold" });
    expect(section.textContent).toContain("A scanner flagged the Windows (.zip) build");
    // Names and paths as code, never as HTML.
    expect(within(section).getByText("Win.Test.Seed", { selector: "code" })).toBeTruthy();
    expect(within(section).getByText("pong.exe", { selector: "code" })).toBeTruthy();
    expect(section.textContent).toContain("SLAPorSLOP staff will look at it. You'll get a notice when they decide or answer.");
    // Fixing it is doable right here: the new-version form is on this page.
    expect(within(section).getByRole("link", { name: "Send a new version" }).getAttribute("href")).toBe("#new-release");
    // SLAPorSLOP's opening line only repeats the reason: not shown again.
    expect(within(section).queryByRole("list", { name: /Conversation with SLAPorSLOP staff/ })).toBeNull();
    expect(section.textContent).not.toMatch(/\bsafe\b|\bverified\b|\btrusted\b/i);
  });

  it("drops only SLAPorSLOP's first line, and only when it repeats the reason", () => {
    const lines = [
      { from: "slaporslop", body: "On hold: a scanner flagged it.", at: "1" },
      { from: "you", body: "On hold: a scanner flagged it.", at: "2" },
      { from: "slaporslop", body: "On hold: a scanner flagged it.", at: "3" },
    ];
    expect(withoutRepeatedReason(lines, "On hold: a scanner flagged it").map((l) => l.at)).toEqual(["2", "3"]);
    expect(withoutRepeatedReason(lines, "Something else.")).toHaveLength(3);
    expect(withoutRepeatedReason(lines, null)).toHaveLength(3);
    // Staff lines carry text instead.
    expect(withoutRepeatedReason([{ from: "slaporslop", text: "On hold: x." }], "On hold: x.")).toEqual([]);
  });

  it("Send a new version takes focus to the form at the top of the page", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold());
    const form = document.createElement("div");
    form.id = "new-release";
    form.tabIndex = -1;
    form.innerHTML = '<input type="hidden" name="x"><label>Version <input name="version"></label>';
    form.scrollIntoView = () => {};
    document.body.appendChild(form);
    try {
      show(release());
      fireEvent.click(await screen.findByRole("link", { name: "Send a new version" }));
      expect(document.activeElement).toBe(form.querySelector("input[name=version]"));
    } finally {
      form.remove();
    }
  });

  it("withdraws it after asking; Not yet puts focus back on the button", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold());
    const withdraw = vi.spyOn(api, "withdrawRelease").mockResolvedValue(release({ status: "withdrawn" }));
    const changed = show(release());
    const button = await screen.findByRole("button", { name: "Withdraw version 1.1.0…" });
    fireEvent.click(button);
    const sure = screen.getByRole("group", { name: "Withdraw version 1.1.0" });
    expect(sure.textContent).toContain("It won't be published or reach players.");
    fireEvent.click(within(sure).getByRole("button", { name: "Not yet" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Withdraw version 1.1.0…" }));
    expect(withdraw).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Withdraw version 1.1.0…" }));
    fireEvent.click(screen.getByRole("button", { name: "Yes, withdraw it" }));
    await waitFor(() => expect(withdraw).toHaveBeenCalledWith("pocket-golf", "rel1"));
    await waitFor(() => expect(changed).toHaveBeenCalled());
  });

  it("runs the scans again, saying how many re-runs are left, and then that they're under way", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold());
    const rescan = vi
      .spyOn(api, "rescanRelease")
      .mockResolvedValue(hold({ rescans_left: 2, rescan_requested: true, you_can: ["write_to_staff", "new_release", "withdraw"] }));
    const changed = show(release());
    const button = await screen.findByRole("button", { name: "Scan again" });
    expect(screen.getByText("3 of 3 re-runs left.")).toBeTruthy();
    expect(button.getAttribute("aria-describedby")).toBeTruthy();
    fireEvent.click(button);
    expect(rescan).toHaveBeenCalledWith("pocket-golf", "rel1");
    expect(await screen.findByText("SLAPorSLOP will scan the held files again shortly. You'll be told the result.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Scan again" })).toBeNull();
    expect(changed).toHaveBeenCalled();
  });

  it("says when the next re-run is allowed, or that none are left, with no button", () => {
    const at = "2026-11-07T16:00:00Z";
    expect(rescanLine(hold({ next_rescan_at: at, you_can: ["write_to_staff"] }))).toMatch(/^3 of 3 re-runs left\. The next is allowed from /);
    expect(rescanLine(hold({ rescans_left: 0, you_can: ["write_to_staff"] }))).toBe(
      "No re-runs left: SLAPorSLOP runs the scans again up to 3 times per release. Write to SLAPorSLOP staff, or send a new version.",
    );
    expect(rescanLine(hold({ rescan_requested: true, rescan_running: true }))).toBe(
      "SLAPorSLOP is scanning the held files again now. You'll be told the result.",
    );
    // An archive too big or nested to scan: running it again won't help.
    expect(rescanLine(hold({ you_can: ["write_to_staff", "new_release"] }))).toBe("Scanning again won't help with this hold.");
    // This server's scanner is off.
    expect(rescanLine(hold({ you_can: ["write_to_staff", "new_release"] }), false)).toBe(
      "SLAPorSLOP's scanner is off right now, so it can't scan again.",
    );
  });

  it("hides Scan again until it's allowed", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold({ next_rescan_at: "2026-11-07T16:00:00Z", you_can: ["write_to_staff"] }));
    show(release());
    expect(await screen.findByText(/The next is allowed from/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Scan again" })).toBeNull();
  });

  it("writes to SLAPorSLOP staff from a labelled form, and shows their answer", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold());
    const write = vi.spyOn(api, "writeToStaff").mockResolvedValue(
      hold({
        unanswered: true,
        asked_staff_at: "2026-11-07T11:00:00Z",
        messages: [
          ...hold().messages,
          { from: "you", by_agent: false, body: "pong.exe is our own test harness.", at: "2026-11-07T11:00:00Z" },
          { from: "staff", staff_as: "SLAPorSLOP reviewer", by_agent: false, body: "Thanks, looking now.", at: "2026-11-07T12:00:00Z" },
        ],
      }),
    );
    const changed = show(release());
    const box = await screen.findByRole("textbox", { name: "Your message to SLAPorSLOP staff" });
    const send = screen.getByRole("button", { name: "Write to SLAPorSLOP staff" }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    expect(box.getAttribute("maxlength")).toBe("2000");
    fireEvent.change(box, { target: { value: "pong.exe is our own test harness." } });
    fireEvent.click(send);
    await waitFor(() => expect(write).toHaveBeenCalledWith("pocket-golf", "rel1", "pong.exe is our own test harness."));
    expect(await screen.findByText("Sent. SLAPorSLOP staff are told, and you'll get a notice when they answer.")).toBeTruthy();
    const log = screen.getByRole("list", { name: /Conversation with SLAPorSLOP staff/ });
    expect(log.textContent).toContain("You");
    expect(log.textContent).toContain("SLAPorSLOP reviewer");
    expect(log.textContent).toContain("Thanks, looking now.");
    expect(changed).toHaveBeenCalled();
  });

  it("shows why a message wasn't sent", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold());
    vi.spyOn(api, "writeToStaff").mockRejectedValue(
      new ApiError(429, "not_yet", "You've written 5 messages about this release today. The next is allowed at 10:00 (in 3 hours)."),
    );
    show(release());
    fireEvent.change(await screen.findByRole("textbox", { name: "Your message to SLAPorSLOP staff" }), { target: { value: "Hello" } });
    fireEvent.click(screen.getByRole("button", { name: "Write to SLAPorSLOP staff" }));
    expect((await screen.findByRole("alert")).textContent).toContain("You've written 5 messages about this release today.");
  });

  it("marks the box when what was written is refused, and says why next to it", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(hold());
    vi.spyOn(api, "writeToStaff").mockRejectedValue(new ApiError(422, "invalid", "at most 3 links", "body"));
    show(release());
    const box = await screen.findByRole("textbox", { name: "Your message to SLAPorSLOP staff" });
    expect(box.getAttribute("aria-invalid")).toBeNull();
    fireEvent.change(box, { target: { value: "a b c d" } });
    fireEvent.click(screen.getByRole("button", { name: "Write to SLAPorSLOP staff" }));
    const error = await screen.findByRole("alert");
    expect(box.getAttribute("aria-invalid")).toBe("true");
    expect(box.getAttribute("aria-describedby")?.split(" ")).toContain(error.id);
  });

  it("explains a missing Defender check, with Slop Check to add right here; staff have nothing to do yet", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(
      hold({
        kinds: ["defender_missing"],
        needs_staff: false,
        problems: [
          {
            kind: "defender_missing",
            platform: "windows",
            file_label: "Windows (.zip)",
            what: "The Windows (.zip) build is waiting for Microsoft Defender's check",
            explained:
              "Windows builds need Microsoft Defender's check. Run Slop Check in your repository (see /publish/guide#slop-check). This release goes on by itself as soon as the result arrives.",
            you_can: ["run_slop_check", "write_to_staff"],
          },
        ],
        you_can: ["run_slop_check", "write_to_staff", "withdraw"],
      }),
    );
    show(release({ hold: { status: "open", kinds: ["defender_missing"], needs_staff: false, rescans_left: 3, unanswered: false } }));
    const section = await screen.findByRole("region", { name: "Why it's on hold" });
    expect(section.textContent).toContain("Run Slop Check in your repository. This release goes on by itself");
    expect(section.textContent).not.toContain("/publish/guide#slop-check)");
    expect(within(section).getByRole("link", { name: "How Slop Check works" }).getAttribute("href")).toBe("/publish/guide#slop-check");
    expect(within(section).getByRole("link", { name: "Add Slop Check to your repository" }).getAttribute("href")).toBe("#slop-check");
    expect(section.textContent).toContain("Nothing for SLAPorSLOP staff to do yet");
    expect(within(section).queryByRole("button", { name: "Scan again" })).toBeNull();
  });

  it("outside GitHub, a missing Defender check is staff's: no Slop Check, and writing to them is right there", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(
      hold({
        kinds: ["defender_missing"],
        needs_staff: true,
        problems: [
          {
            kind: "defender_missing",
            platform: "windows",
            file_label: "Windows (.zip)",
            what: "The Windows build needs Microsoft Defender's check",
            explained:
              "Slop Check runs on GitHub Actions, so it can't run for a repository outside GitHub yet. SLAPorSLOP staff check the Windows build by hand; you can write to them below.",
            you_can: ["write_to_staff", "new_release"],
          },
        ],
        you_can: ["write_to_staff", "new_release", "withdraw"],
      }),
    );
    show(release({ hold: { status: "open", kinds: ["defender_missing"], needs_staff: true, rescans_left: 3, unanswered: false } }));
    const section = await screen.findByRole("region", { name: "Why it's on hold" });
    expect(section.textContent).toContain("The Windows build needs Microsoft Defender's check");
    expect(section.textContent).toContain("SLAPorSLOP staff will look at it.");
    expect(section.textContent).not.toContain("Nothing for SLAPorSLOP staff to do yet");
    expect(within(section).queryByRole("link", { name: /Slop Check/ })).toBeNull();
    const box = within(section).getByRole("textbox", { name: "Your message to SLAPorSLOP staff" });
    expect(box.getAttribute("placeholder")).toBe("What you'd like SLAPorSLOP staff to know");
    expect(section.textContent).not.toContain("flagged");
  });

  it("explains a lowered rating: from and to, and the labels it drops", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(
      hold({
        kinds: ["rating_lowered"],
        problems: [
          {
            kind: "rating_lowered",
            what: "It lowers the age rating",
            explained: "Lowers the age rating from 16+ to 12+ (drops: horror). A SLAPorSLOP staff member checks a lowered rating before you can publish.",
            you_can: ["write_to_staff", "new_release"],
          },
        ],
        you_can: ["write_to_staff", "new_release", "withdraw"],
        rating: { from_age: 16, to_age: 12, dropped_labels: ["horror"], content_labels: ["violence"] },
      }),
    );
    show(release({ hold: { status: "open", kinds: ["rating_lowered"], needs_staff: true, rescans_left: 3, unanswered: false } }));
    const section = await screen.findByRole("region", { name: "Why it's on hold" });
    expect(section.textContent).toContain("A SLAPorSLOP staff member checks a lowered rating before you can publish.");
    expect(section.textContent).toContain("Age rating16+ → 12+");
    expect(section.textContent).toContain("Content labels it dropsHorror");
    expect(section.textContent).toContain("A new version with the rating of the last published one needs no check by staff.");
    expect(section.textContent).toContain("Changing it back instead? Send a new version: once it passes its checks, this one is withdrawn.");
    expect(within(section).queryByRole("button", { name: "Scan again" })).toBeNull();
    // Writing to staff asks about the rating, not a flagged file.
    const box = within(section).getByRole("textbox", { name: "Your message to SLAPorSLOP staff" });
    expect(box.getAttribute("placeholder")).toBe("What changed, so the lower rating fits");
    expect(section.textContent).toContain("Say what changed so the lower rating fits (content removed or toned down).");
    expect(section.textContent).not.toContain("flagged file");
  });

  it("words the box and the new-version line by what holds it", () => {
    expect(writingFor(["scan", "rating_lowered"], false)).toBe("flagged");
    expect(writingFor(["rating_lowered"], false)).toBe("rating");
    expect(writingFor(["defender_missing"], false)).toBe("other");
    expect(writingFor(["rating_lowered"], true)).toBe("ask");
    expect(newVersionLine(["defender"], false).before).toBe("Fixing the build instead?");
    expect(newVersionLine(["defender_missing"], false).before).toBe("Replacing it instead?");
    expect(newVersionLine([], true)).toEqual({ before: "Fixed it?", after: "." });
  });
});

describe("a failed release", () => {
  it("lets its maker ask SLAPorSLOP staff about it", async () => {
    const load = vi.spyOn(api, "releaseHold").mockResolvedValue(
      hold({
        release_status: "failed",
        status: null,
        kinds: [],
        needs_staff: false,
        problems: [{ kind: "failed", what: "Its checks failed", explained: "No signature.", you_can: ["new_release", "write_to_staff"] }],
        you_can: ["new_release", "write_to_staff"],
        messages: [],
      }),
    );
    const write = vi.spyOn(api, "writeToStaff").mockResolvedValue(hold({ release_status: "failed", kinds: ["question"], unanswered: true }));
    render(
      <MemoryRouter>
        <AskAboutFailed slug="pocket-golf" release={release({ status: "failed", status_reason: "No signature.", hold: undefined })} onChanged={() => {}} />
      </MemoryRouter>,
    );
    expect(load).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Ask SLAPorSLOP staff" }));
    const box = await screen.findByRole("textbox", { name: "Your question for SLAPorSLOP staff" });
    // Its reason is on the release's card already.
    expect(screen.queryByText("Its checks failed")).toBeNull();
    fireEvent.change(box, { target: { value: "Which signature is missing?" } });
    fireEvent.click(screen.getByRole("button", { name: "Ask SLAPorSLOP staff" }));
    await waitFor(() => expect(write).toHaveBeenCalledWith("pocket-golf", "rel1", "Which signature is missing?"));
  });

  it("a release staff turned down: its maker can ask about it too", async () => {
    vi.spyOn(api, "releaseHold").mockResolvedValue(
      hold({
        release_status: "rejected",
        status: "rejected",
        kinds: ["rating_lowered"],
        needs_staff: false,
        problems: [
          { kind: "rejected", what: "SLAPorSLOP staff turned it down", explained: "Its pictures show fights.", you_can: ["new_release", "write_to_staff"] },
        ],
        you_can: ["new_release", "write_to_staff"],
      }),
    );
    const write = vi.spyOn(api, "writeToStaff").mockResolvedValue(hold({ release_status: "rejected", kinds: ["question"], unanswered: true }));
    const turnedDown = release({
      status: "rejected",
      status_reason: "Its pictures show fights.",
      hold: { status: "rejected", kinds: ["rating_lowered"], needs_staff: false, rescans_left: 3, unanswered: false },
    });
    render(
      <MemoryRouter>
        <AskAboutFailed slug="pocket-golf" release={turnedDown} onChanged={() => {}} />
      </MemoryRouter>,
    );
    // Folded until asked: its past hold is on the card already.
    expect(screen.getByText(/Want SLAPorSLOP staff to explain, or look again\?/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Ask SLAPorSLOP staff" }));
    const ask = await screen.findByRole("region", { name: "Ask SLAPorSLOP staff" });
    expect(ask.textContent).toContain("The hold ended: SLAPorSLOP staff turned it down.");
    expect(ask.textContent).not.toContain("SLAPorSLOP staff turned it down" + "Its pictures");
    // No withdrawing a release that's already ended.
    expect(within(ask).queryByRole("button", { name: /^Withdraw/ })).toBeNull();
    fireEvent.change(within(ask).getByRole("textbox", { name: "Your question for SLAPorSLOP staff" }), { target: { value: "We removed the fights." } });
    fireEvent.click(within(ask).getByRole("button", { name: "Ask SLAPorSLOP staff" }));
    await waitFor(() => expect(write).toHaveBeenCalledWith("pocket-golf", "rel1", "We removed the fights."));
  });

  it("shows an answered question, and how other holds ended, in words", () => {
    expect(HOLD_ENDED.answered).toBe("SLAPorSLOP staff answered your question. You can ask again below.");
    expect(HOLD_ENDED.cleared).toBe("The hold ended: SLAPorSLOP staff accepted it.");
    expect(makerLine({ from: "you", by_agent: true, body: "x", at: "2026-11-07T10:00:00Z" }, 0).who).toBe("Your AI agent");
    expect(makerLine({ from: "staff", by_agent: false, body: "x", at: "2026-11-07T10:00:00Z" }, 0).who).toBe("SLAPorSLOP staff");
    expect(
      makerLine({ from: "slaporslop", by_agent: false, body: "Scanned again.", detail: { platform: "windows", before: "on_hold", after: "passed" }, at: "2026-11-07T10:00:00Z" }, 0)
        .facts,
    ).toBe("windows: on hold → passed");
  });
});
