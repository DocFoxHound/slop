// SPDX-License-Identifier: AGPL-3.0-or-later
// Where an app is on its way into the store, step by step, for its maker.
// Built from the app, its releases (with each file's check, crates/slop-api/src/release_progress.rs)
// and what this server checks (GET /v1/server). A browser game's release is a play address
// instead of files: its steps are the address and its checks, with no signature or scan, and a
// step that says plainly what SLAPorSLOP can't check (the site itself). The checks end with the
// release passed (its maker then publishes it), on hold (ReleaseHold.tsx says what holds it), or
// failed; the last step puts a draft in the store, which its maker does from here or Manage.

import { useState, type ReactNode } from "react";
import { Link } from "react-router";

import { addressCheckItems } from "../addressChecks";
import { api, ApiError, publishBlocked, type AppSummary, type FileCheckStage, type Release, type ReleaseFile } from "../api";
import { formatDate, isDedicatedServer } from "../format";
import { useConfirmStep, useNow } from "../hooks";
import { useInviteGate } from "./Invite";

type State = "done" | "active" | "problem" | "waiting" | "skipped";

/** One file of the release, shown under the files step. */
export interface SubItem {
  key: string;
  label: string;
  state: State;
  /** In words, so the state never rests on the icon or colour alone. */
  status: string;
  detail?: ReactNode;
}

export interface Step {
  title: string;
  state: State;
  detail: ReactNode;
  /** Replaces the usual "In progress" / "Done" next to the title. */
  stateLabel?: string;
  /** A second line: an estimate, or what happens next. */
  more?: ReactNode;
  items?: SubItem[];
  /** What the items are, for their list's name: "file" (the default) or "check". */
  itemName?: "file" | "check";
  /** A button the maker presses right here: "list" puts the draft in the store. */
  action?: "list";
}

const ICON: Record<State, string> = {
  done: "✓",
  active: "…",
  problem: "!",
  waiting: "○",
  skipped: "–",
};
const STATE_LABEL: Record<State, string> = {
  done: "Done",
  active: "In progress",
  problem: "Needs attention",
  waiting: "Not yet",
  skipped: "Not checked",
};

const STAGE_LABEL: Record<FileCheckStage, string> = {
  downloading: "Downloading",
  signature: "Checking the maker signature",
  scanning: "Scanning for malware",
  waiting_for_scan: "Waiting for its scan from GitHub",
};

/** The scanner's longest run on one file (crates/slop-api/src/scan.rs). */
const LONGEST_SCAN_MINUTES = 30;

type Server = { scanning: boolean; maker_signatures_required: boolean };
type ProgressApp = Pick<AppSummary, "stage" | "status"> & {
  slug?: string;
  /** Private: shared, never in the store (going public is for SLAPorSLOP staff). */
  private?: boolean;
  plays_in_browser?: boolean;
  tags?: string[];
  /** A dedicated server: its game's address. It has no store page of its own. */
  server_for?: string;
};

/**
 * A browser game's release whose address Play opens (`plays`), and, when Play opens none because
 * SLAPorSLOP withdrew it, that one (`withdrawn`). The maker's releases say which plays
 * (`plays_now`). From a server that doesn't, the newest approved or withdrawn one decides (Play
 * never falls back to an older address). Exported for tests.
 */
export function wherePlayGoes(releases: Release[]): { plays?: Release; withdrawn?: Release } {
  const told = releases.some((r) => r.plays_now !== undefined);
  const decided = releases.find((r) => r.status === "approved" || r.status === "revoked");
  const plays = told ? releases.find((r) => r.plays_now) : decided?.status === "approved" ? decided : undefined;
  return { plays, withdrawn: plays ? undefined : releases.find((r) => r.status === "revoked") };
}

/** "Windows (.zip)", "Linux (.deb)": the platform and the kind of file (as the API says it). */
export function fileLabel(f: Pick<ReleaseFile, "platform" | "file_name">): string {
  const name = f.file_name.toLowerCase();
  const ext = [".tar.gz", ".tgz", ".zip", ".deb", ".rpm", ".apk"].find((e) => name.endsWith(e));
  const system: string =
    f.platform === "windows"
      ? "Windows"
      : f.platform.startsWith("linux")
        ? "Linux"
        : f.platform === "mac"
          ? "macOS"
          : f.platform === "android"
            ? "Android"
            : f.platform === "any"
              ? "Mod"
              : f.platform;
  return ext ? `${system} (${ext})` : system;
}

const minutes = (n: number) => `${n} minute${n === 1 ? "" : "s"}`;

/** "under a minute", "about 5 minutes", "about 1 hour 10 minutes". */
export function about(seconds: number): string {
  if (seconds < 60) return "under a minute";
  const total = Math.round(seconds / 60);
  if (total < 60) return `about ${minutes(total)}`;
  const h = Math.floor(total / 60);
  const rest = total % 60;
  return `about ${h} hour${h === 1 ? "" : "s"}${rest ? ` ${minutes(rest)}` : ""}`;
}

/** "just now", "4 minutes ago", "1 hour 5 minutes ago". */
export function ago(iso: string, now: number): string {
  const seconds = Math.max(0, (now - Date.parse(iso)) / 1000);
  if (seconds < 60) return "just now";
  return `${about(seconds).replace(/^about /, "")} ago`;
}

/** A file's check: from the API, or worked out for releases from before it said. */
export function fileCheck(f: ReleaseFile, status: Release["status"]): NonNullable<ReleaseFile["check"]> {
  if (f.check) return f.check;
  if (f.checked_at) return "checked";
  if (status === "fetching") return f.check_stage ? "checking" : "waiting";
  return f.sha256 ? "checked" : "not_checked";
}

const held = (r: ReleaseFile["scan_result"]) => r === "on_hold" || r === "rejected" || r === "blocked";

/** The other checks on one file: its signature and, for Windows, Microsoft Defender. */
function otherChecks(f: ReleaseFile, status: Release["status"], server: Server): string[] {
  const out: string[] = [];
  if (f.provenance) out.push("signed by your build");
  else if (f.maker_key) out.push(`signed with your key "${f.maker_key.key_name}"`);
  else if (f.provenance_problem && server.maker_signatures_required && status !== "fetching") {
    out.push(`not signed: ${f.provenance_problem}`);
  }
  if (f.platform === "windows") {
    if (f.defender) {
      const n = f.defender.detections.length;
      out.push(n === 0 ? "Microsoft Defender found nothing" : `Microsoft Defender: ${n} detection${n === 1 ? "" : "s"}`);
    } else if (status !== "fetching" && server.scanning) {
      out.push("no Microsoft Defender check yet (Slop Build or Slop Check runs it)");
    }
  }
  return out;
}

function sentence(parts: Array<string | null | undefined>): string | undefined {
  const text = parts.filter(Boolean).join(" · ");
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : undefined;
}

/** One line per file: done (and how it went), being checked (and at which step), or waiting. */
function fileItem(f: ReleaseFile, status: Release["status"], server: Server, now: number): SubItem {
  const check = fileCheck(f, status);
  const base = { key: f.platform, label: fileLabel(f) };
  const extra = otherChecks(f, status, server);
  const took =
    f.checked_at && f.check_started_at
      ? `took ${about((Date.parse(f.checked_at) - Date.parse(f.check_started_at)) / 1000).replace(/^about /, "")}`
      : null;
  if (check === "checking") {
    return {
      ...base,
      state: "active",
      status: STAGE_LABEL[f.check_stage ?? "downloading"],
      detail: sentence([f.check_started_at ? `started ${ago(f.check_started_at, now)}` : null, ...extra]),
    };
  }
  if (check === "waiting") return { ...base, state: "waiting", status: "Waiting" };
  if (check === "not_checked") return { ...base, state: "skipped", status: "Not checked" };
  const r = f.scan_result;
  if (held(r) && f.cleared) {
    return {
      ...base,
      state: "done",
      status: "Cleared by SLAPorSLOP staff",
      detail: sentence([
        `found: ${f.scan_detail ?? "a scanner finding"}; cleared as a false positive${f.cleared_reason ? `: ${f.cleared_reason}` : ""}`,
        ...extra,
      ]),
    };
  }
  if (held(r)) {
    return {
      ...base,
      state: "problem",
      status: "On hold",
      detail: sentence([f.scan_detail ?? "the scanner found something", ...extra]),
    };
  }
  if (r === "note") return { ...base, state: "done", status: "Passed, with a note", detail: sentence([f.scan_detail, took, ...extra]) };
  if (r === "passed") return { ...base, state: "done", status: "Passed", detail: sentence([took, ...extra]) };
  // Scanning off (test servers): downloaded and hashed only.
  return { ...base, state: "done", status: "Hashed", detail: sentence([took, ...extra]) };
}

/** A link to one release on the Releases page (it scrolls there). */
function ReleaseLink({ slug, release, children }: { slug?: string; release: Release; children: ReactNode }) {
  if (!slug) return <>{children}</>;
  return <Link to={`/publish/${slug}/releases#release-${release.id}`}>{children}</Link>;
}

/**
 * What the maker can do about a hold, from its kinds (holds.rs): Slop Check for a missing Defender
 * check it can run, Scan again only for a scanner's hold with re-runs left, and always writing to
 * staff or sending a new version. Exported for tests.
 */
export function holdActions(hold: Release["hold"], address: boolean): string {
  const kinds = hold?.kinds ?? [];
  // Slop Check can run (a GitHub repository): staff have nothing to do until the maker writes.
  if (kinds.length === 1 && kinds[0] === "defender_missing" && !hold?.needs_staff) return ": run Slop Check, and it goes on by itself.";
  const scan = !address && kinds.includes("scan") && (hold?.rescans_left ?? 0) > 0;
  return scan ? ": scan again, write to SLAPorSLOP staff, or send a new version." : ": write to SLAPorSLOP staff there, or send a new version.";
}

/** What holds a release, from its maker's hold summary: who has to do something. */
function holdStep(app: ProgressApp, latest: Release, address: boolean): Step {
  const hold = latest.hold;
  const missingCheck = hold?.kinds.length === 1 && hold.kinds[0] === "defender_missing" && !hold.needs_staff;
  return {
    title: "Checks",
    state: "problem",
    stateLabel: missingCheck ? "Waiting for Microsoft Defender's check" : "On hold",
    detail: latest.progress?.summary ?? latest.status_reason ?? "On hold.",
    more: (
      <>
        <ReleaseLink slug={app.slug} release={latest}>
          See what holds version {latest.version}
        </ReleaseLink>
        {holdActions(hold, address)}
      </>
    ),
  };
}

/** The Publish step: done once it's published, the maker's to press once it passed. */
function publishStep(app: ProgressApp, latest: Release, published: Release | undefined, address: boolean): Step {
  // A draft isn't in the store: players get nothing until its maker puts it there.
  const draft = app.status === "draft";
  const players = draft
    ? address
      ? app.stage === "coming_soon"
        ? "Play opens its address once the game is in the store and in Beta"
        : "players can play it once you put the game in the store"
      : "players can get it once you put the app in the store"
    : address
      ? "Play opens its address"
      : app.stage === "coming_soon"
        ? "players can get it once you move the app to Beta"
        : "players get it";
  if (latest.status === "approved") {
    return {
      title: "Publish",
      state: "done",
      detail: `You published version ${latest.version}${latest.published_at ? ` on ${formatDate(latest.published_at)}` : ""}.`,
      more: address ? latest.progress?.summary : undefined,
    };
  }
  // Something must be fixed before it can be published (publish.rs): the way on, not Publish.
  if (latest.status === "ready" && publishBlocked(latest)) {
    const n = latest.must_fix?.length ?? 0;
    return {
      title: "Publish",
      state: "active",
      stateLabel: "Fix first",
      detail: (
        <>
          Version {latest.version} passed its checks, but {n === 1 ? "1 thing" : `${n} things`} must be fixed before you can
          publish it. <Link to={`/publish/${app.slug}/fixes`}>See what to fix</Link>, or{" "}
          <ReleaseLink slug={app.slug} release={latest}>
            ask SLAPorSLOP staff
          </ReleaseLink>{" "}
          if you think one is wrong.
        </>
      ),
      more: latest.progress?.summary,
    };
  }
  if (latest.status === "ready") {
    return {
      title: "Publish",
      state: "active",
      stateLabel: "Waiting for you",
      detail: (
        <>
          Version {latest.version} passed every check.{" "}
          <ReleaseLink slug={app.slug} release={latest}>
            Publish it
          </ReleaseLink>
          {draft ? "; " : " and "}
          {players}
          {address && !draft && app.stage === "coming_soon" ? " once the game is in Beta" : ""}.
        </>
      ),
      more: latest.progress?.summary,
    };
  }
  if (published) {
    return {
      title: "Publish",
      state: "done",
      detail: draft
        ? `Version ${published.version} is published: ${players}.`
        : `Version ${published.version} is published${address ? ": Play keeps opening its address" : ": players keep it"} until you publish a newer one.`,
    };
  }
  return {
    title: "Publish",
    state: "waiting",
    detail: address
      ? "Once the address passes the checks, you publish it and Play opens it."
      : "Once a release passes the checks, you publish it: SLAPorSLOP signs it, and players get it.",
  };
}

/** A link to the fixes SLAPorSLOP's checks found, ready to copy for an AI agent (Releases page). */
function FixesLink({ slug }: { slug?: string }) {
  if (!slug) return null;
  return (
    <>
      {" "}
      <Link to={`/publish/${slug}/releases#fixes`}>See what to fix</Link> (with "Copy for your AI"), or ask your AI agent to
      run <code>get_fixes</code>.
    </>
  );
}

/** The files step while SLAPorSLOP checks them: which file, which step, and an honest estimate. */
function checkingStep(latest: Release, server: Server, now: number): Step {
  const files = latest.files;
  const total = files.length;
  const items = files.map((f) => fileItem(f, latest.status, server, now));
  const checks = files.map((f) => fileCheck(f, latest.status));
  const checked = checks.filter((c) => c === "checked").length;
  const index = checks.indexOf("checking");
  const per = latest.progress?.seconds_per_file ?? null;
  const fileWord = `${total} file${total === 1 ? "" : "s"}`;
  if (index < 0 && checked === 0) {
    return {
      title: "Files checked",
      state: "active",
      stateLabel: "Waiting its turn",
      detail:
        `SLAPorSLOP checks one release at a time. Then its ${fileWord} are downloaded and scanned, one at a time.`,
      more: per ? `Each file takes ${about(per)} here.` : undefined,
      items,
    };
  }
  const current = index >= 0 ? files[index] : undefined;
  const started =
    latest.progress?.started_at ??
    files
      .map((f) => f.check_started_at)
      .filter((t): t is string => Boolean(t))
      .sort()[0];
  const parts: string[] = [];
  if (per) parts.push(`${about(per).replace(/^about/, "About")} per file`);
  if (started) parts.push(`${per ? "started" : "Started"} ${ago(started, now)}`);
  let more = parts.length ? `${parts.join("; ")}.` : "";
  if (per && checked < total) {
    const elapsed = current?.check_started_at ? Math.max(0, (now - Date.parse(current.check_started_at)) / 1000) : 0;
    const afterThis = Math.max(0, total - checked - 1) * per;
    const thisLeft = per - elapsed;
    if (thisLeft > 0 || !current) {
      more += ` ${about(afterThis + Math.max(0, thisLeft)).replace(/^about/, "About").replace(/^under/, "Under")} left.`;
    } else {
      more += ` This file is taking longer than usual: big programs take longest, up to ${LONGEST_SCAN_MINUTES} minutes a file${
        afterThis > 0 ? `, then ${about(afterThis)} for the rest` : ""
      }.`;
    }
  }
  return {
    title: "Files checked",
    state: "active",
    stateLabel: `${checked} of ${total} done`,
    detail: current
      ? `Checking file ${index + 1} of ${total}: ${fileLabel(current)} — ${STAGE_LABEL[current.check_stage ?? "downloading"]}`
      : checked === total
        ? `All ${fileWord} are checked; finishing up.`
        : `${checked} of ${total} files checked; starting the next.`,
    more: more.trim() || undefined,
    items,
  };
}

/** The steps, from the app's own data. Exported for tests. */
export function progressSteps(app: ProgressApp, releases: Release[], server: Server, now: number = Date.now()): Step[] {
  const comingSoon = app.stage === "coming_soon";
  // The release that matters: the newest one. One withdrawn before it was published never counts.
  releases = releases.filter((r) => r.status !== "withdrawn");
  const latest = releases[0];
  const approved = releases.find((r) => r.status === "approved");
  const steps: Step[] = [
    app.server_for
      ? {
          title: "Dedicated server",
          state: "done",
          detail: "Saved. Once it's in the store, players get it on its game's store page.",
        }
      : {
          title: "Store page",
          state: "done",
          detail: "Your store page is saved.",
        },
  ];

  if (app.plays_in_browser) {
    steps.push(...addressSteps(app, releases));
  } else if (comingSoon) {
    steps.push({
      title: "Release",
      state: "skipped",
      detail: "Coming soon pages go in the store without a build. Submit a release when you move to Beta.",
    });
  } else if (!latest) {
    steps.push({
      title: "Release",
      state: "waiting",
      detail: "Submit a release below, or set up Slop Build to send them automatically.",
    });
  } else {
    steps.push({
      title: "Release",
      state: "done",
      detail: `Version ${latest.version} received${
        latest.source === "slop_build" ? " from Slop Build" : latest.source === "github_release" ? " from your GitHub release" : ""
      }.`,
    });

    const files = latest.files;
    const fetching = latest.status === "fetching";
    if (fetching) {
      steps.push(checkingStep(latest, server, now));
    } else if (latest.status === "failed") {
      steps.push({
        title: "Files checked",
        state: "problem",
        detail: (
          <>
            SLAPorSLOP couldn't check the files: {latest.status_reason?.replace(/\.$/, "") ?? "no reason given"}. Fix it and submit the
            release again.
            <FixesLink slug={app.slug} />
          </>
        ),
        items: files.map((f) => fileItem(f, latest.status, server, now)),
      });
    } else {
      steps.push({
        title: "Files checked",
        state: "done",
        detail: `${files.length} file${files.length === 1 ? "" : "s"} downloaded and hashed${server.scanning ? ", and scanned" : ""}.`,
        items: files.map((f) => fileItem(f, latest.status, server, now)),
      });
    }

    const unsigned = files.filter((f) => !f.provenance && !f.maker_key);
    if (fetching) {
      steps.push({
        title: "Maker signature",
        state: "waiting",
        detail: "Checked with each file, above.",
      });
    } else if (unsigned.length === 0) {
      steps.push({
        title: "Maker signature",
        state: "done",
        detail: files.some((f) => f.maker_key)
          ? "Every file is signed by your build or your registered key."
          : "Every file is signed by your build.",
      });
    } else if (!server.maker_signatures_required) {
      steps.push({
        title: "Maker signature",
        state: "skipped",
        detail: "Not required on this server (private test setup).",
      });
    } else {
      steps.push({
        title: "Maker signature",
        state: "problem",
        detail: unsigned[0].provenance_problem ?? "A file isn't signed by your build.",
      });
    }

    const scans = files.map((f) => f.scan_result);
    const onHold = files.filter((f) => held(f.scan_result) && !f.cleared);
    if (fetching || scans.some((s) => s === null)) {
      steps.push({
        title: "Scan",
        state: "waiting",
        detail: fetching ? "Each file is scanned as it's checked, above." : "Runs once the files are checked.",
      });
    } else if (!server.scanning || scans.every((s) => s === "skipped")) {
      steps.push({
        title: "Scan",
        state: "skipped",
        detail: "Automatic scanning isn't running on this server yet.",
      });
    } else if (onHold.length > 0 && latest.status === "awaiting_review") {
      steps.push({
        title: "Scan",
        state: "problem",
        stateLabel: "On hold",
        detail: `${onHold.length === 1 ? `${fileLabel(onHold[0])} is` : `${onHold.length} files are`} on hold: ${
          onHold[0].scan_detail?.replace(/\.$/, "") ?? "a scanner found something"
        }.`,
        more:
          "Scanners make mistakes. SLAPorSLOP staff look at it, and you can scan again or tell them why it's a false positive. If the finding is real, fix the build and submit a new version.",
      });
    } else if (onHold.length > 0) {
      // Turned down, or failed for another reason, with a file still flagged.
      steps.push({
        title: "Scan",
        state: "problem",
        stateLabel: "Flagged",
        detail: `${onHold.length === 1 ? `${fileLabel(onHold[0])} was` : `${onHold.length} files were`} flagged: ${
          onHold[0].scan_detail?.replace(/\.$/, "") ?? "a scanner found something"
        }.`,
      });
    } else {
      const cleared = files.some((f) => held(f.scan_result) && f.cleared);
      steps.push({
        title: "Scan",
        state: "done",
        detail: cleared
          ? "Flagged, then cleared by SLAPorSLOP staff as a false positive."
          : scans.includes("note")
            ? "Passed, with notes: see the release below."
            : "Passed.",
      });
    }

    const source = latest.repo_scan;
    const notes = sourceNotes(source, false);
    if (fetching) {
      steps.push({
        title: "Source check",
        state: "waiting",
        detail: "Runs once the files are checked.",
      });
    } else if (!source) {
      steps.push({
        title: "Source check",
        state: "active",
        detail: "SLAPorSLOP is reading the source at this release's commit.",
      });
    } else if (source.status === "not_checked") {
      steps.push({
        title: "Source check",
        state: "skipped",
        detail: source.detail,
      });
    } else if (source.status === "failed") {
      steps.push({
        title: "Source check",
        state: "active",
        detail: `SLAPorSLOP couldn't read the source (${source.detail}); it tries again every hour.`,
      });
    } else if (notes > 0) {
      // Notes, not failures: information for the maker, and logged. A dedicated server's code
      // loading is one too: the rule on outside code doesn't cover it.
      const server = (isDedicatedServer(app) || source.dedicated_server) && (source.code_loading?.length ?? 0) > 0;
      steps.push({
        title: "Source check",
        state: "done",
        stateLabel: "Done, with notes",
        detail: (
          <>
            {notes} note{notes === 1 ? "" : "s"} about the licence{server ? ", selling or code loading" : " or selling"} in the app:
            see the release below.
            {server &&
              " A dedicated server may load mods, maps and plugins from other places: the Content Policy's rule on outside code doesn't cover it, so that's a note, not a problem."}
            <FixesLink slug={app.slug} />
          </>
        ),
      });
    } else {
      steps.push({
        title: "Source check",
        state: "done",
        detail: "The licence file matches, and there are no signs of selling in the app.",
      });
    }

    steps.push(...outcomeSteps(app, latest, approved, false));
  }

  steps.push(storeStep(app, releases, approved));
  return steps;
}

/** The checks' outcome (passed, on hold, turned down, failed, withdrawn), then Publish. */
function outcomeSteps(app: ProgressApp, latest: Release, published: Release | undefined, address: boolean): Step[] {
  const what = address ? "the address" : "the files";
  switch (latest.status) {
    case "fetching":
      return [
        { title: "Checks", state: "waiting", detail: `They end once ${what} and the source are checked.` },
        publishStep(app, latest, published, address),
      ];
    case "awaiting_review":
      return [holdStep(app, latest, address), publishStep(app, latest, published, address)];
    case "rejected":
      return [
        {
          title: "Checks",
          state: "problem",
          stateLabel: "Turned down",
          detail: (
            <>
              Turned down by SLAPorSLOP staff: {latest.status_reason?.replace(/\.$/, "") ?? "no reason given"}. Fix it and submit a new
              version, or{" "}
              <ReleaseLink slug={app.slug} release={latest}>
                ask SLAPorSLOP staff
              </ReleaseLink>{" "}
              about it.
              <FixesLink slug={app.slug} />
            </>
          ),
        },
        publishStep(app, latest, published, address),
      ];
    case "failed":
      return [
        {
          title: "Checks",
          state: "problem",
          stateLabel: "Failed",
          detail: (
            <>
              Version {latest.version} didn't pass. Fix it and submit a new version, or{" "}
              <ReleaseLink slug={app.slug} release={latest}>
                ask SLAPorSLOP staff
              </ReleaseLink>{" "}
              if you think the checks got it wrong.
            </>
          ),
        },
        publishStep(app, latest, published, address),
      ];
    case "revoked":
      return [
        { title: "Checks", state: "done", detail: `Version ${latest.version} passed SLAPorSLOP's checks.` },
        {
          title: "Publish",
          state: "problem",
          stateLabel: "Withdrawn",
          detail: `Withdrawn by SLAPorSLOP: ${latest.status_reason?.replace(/\.$/, "") ?? "no reason given"}.${
            address ? " Play opens nothing until you publish a new address: submit one on the Releases page." : ""
          }`,
        },
      ];
    default:
      // ready or approved: it passed.
      return [
        {
          title: "Checks",
          state: "done",
          detail:
            latest.hold?.status === "cleared"
              ? `Version ${latest.version} passed: SLAPorSLOP staff accepted its hold.`
              : `Version ${latest.version} passed SLAPorSLOP's checks.`,
        },
        publishStep(app, latest, published, address),
      ];
  }
}

/** Putting the app in the store: the maker's step the first time (a Coming soon page, or once a
 *  release is published); appeals and going public stay with SLAPorSLOP staff. */
function storeStep(app: ProgressApp, releases: Release[], published: Release | undefined): Step {
  const comingSoon = app.stage === "coming_soon";
  // The API leaves `status` out once an app is listed.
  const listed = app.status === undefined || app.status === "listed";
  // Listed, but SLAPorSLOP withdrew the address Play opened: players find it, and can't play it.
  const play = app.plays_in_browser ? wherePlayGoes(releases) : undefined;
  if (app.private) {
    return {
      title: "In the store",
      state: "skipped",
      stateLabel: "Private",
      detail: "A private app is shared, not listed. To put it in the store, ask to make it public on Manage.",
    };
  }
  if (listed && play && !comingSoon && !play.plays) {
    return {
      title: "In the store",
      state: "problem",
      stateLabel: "Play opens nothing",
      detail: `Players can find it, but Play opens nothing until you publish ${play.withdrawn ? "a new" : "its"} play address.`,
    };
  }
  if (listed) {
    return {
      title: "In the store",
      state: "done",
      detail: app.server_for ? "Players can get it on its game's store page." : "Players can find it.",
    };
  }
  if (app.status === "delisted") {
    return { title: "In the store", state: "problem", detail: "Taken out of the store by SLAPorSLOP staff." };
  }
  if (comingSoon || published) {
    return {
      title: "In the store",
      state: "active",
      stateLabel: "Waiting for you",
      detail: app.server_for
        ? "Ready: put it in the store when you want players to find it on its game's page."
        : "Ready: put it in the store when you want players to find it.",
      action: "list",
    };
  }
  return {
    title: "In the store",
    state: "waiting",
    detail: app.plays_in_browser ? "Once you've published its play address." : "Once you've published a release.",
  };
}

/** The source check's notes that apply: for a browser game, not the save folder, code loading or
 *  going online (a site is online by nature; the Content Policy's rules on builds and outside code
 *  don't cover it). */
function sourceNotes(source: Release["repo_scan"], browser: boolean): number {
  return (
    (source?.licence?.problems.length ?? 0) +
    (source?.monetization?.length ?? 0) +
    (browser ? 0 : (source?.code_loading?.length ?? 0) + (source?.saves?.length ?? 0) + (source?.online_undeclared ? 1 : 0))
  );
}

/** A browser game's steps between its store page and the store: its play address, its checks,
 *  what SLAPorSLOP can't check, the source check and a person's review. */
function addressSteps(app: ProgressApp, releases: Release[]): Step[] {
  const latest = releases[0];
  const { plays, withdrawn } = wherePlayGoes(releases);
  if (!latest) {
    return [
      app.stage === "coming_soon"
        ? {
            title: "Play address",
            state: "skipped",
            detail:
              "Coming soon pages go in the store without a play address. Add it, and publish it, before you move to Beta.",
          }
        : { title: "Play address", state: "waiting", detail: "Add your game's play address on the Releases page." },
    ];
  }
  // Said wherever a new address is under way after SLAPorSLOP withdrew the one Play opened.
  const opensNothing = withdrawn && (
    <>
      {" "}
      Play opens nothing meanwhile: SLAPorSLOP withdrew version {withdrawn.version}
      {withdrawn.status_reason ? ` (${withdrawn.status_reason.replace(/\.$/, "")})` : ""}.
    </>
  );
  const steps: Step[] = [
    {
      title: "Play address",
      state: "done",
      detail: `Version ${latest.version}: ${latest.play_address ?? "an address"} received.`,
    },
  ];
  const fetching = latest.status === "fetching";
  const items = addressCheckItems(latest.address_checks, latest.status).map(
    (i): SubItem => ({ key: i.key, label: i.label, state: i.state, status: i.status, detail: i.detail }),
  );
  if (fetching) {
    steps.push({
      title: "Address checked",
      state: "active",
      stateLabel: "Checking",
      detail: "SLAPorSLOP is checking it: https, a public site, the bad-link lists, and that the page loads. It usually takes seconds.",
      items,
      itemName: "check",
    });
  } else if (latest.status === "failed") {
    steps.push({
      title: "Address checked",
      state: "problem",
      detail: (
        <>
          SLAPorSLOP couldn't check the address: {latest.status_reason?.replace(/\.$/, "") ?? "no reason given"}. Fix it and submit the
          address again.
          <FixesLink slug={app.slug} />
        </>
      ),
      items,
      itemName: "check",
    });
  } else {
    steps.push({
      title: "Address checked",
      state: "done",
      detail: "It passed SLAPorSLOP's checks.",
      items,
      itemName: "check",
    });
  }
  steps.push({
    title: "What SLAPorSLOP can't check",
    state: "skipped",
    stateLabel: "Not checked",
    detail:
      "Your game runs on your website. SLAPorSLOP checked the address, not what the site serves, so it never says your game is scanned or built from source.",
  });

  const source = latest.repo_scan;
  const notes = sourceNotes(source, true);
  if (fetching) {
    steps.push({ title: "Source check", state: "waiting", detail: "Runs once the address is checked." });
  } else if (latest.status === "failed") {
    steps.push({ title: "Source check", state: "waiting", detail: "Runs once the address passes its checks." });
  } else if (!source) {
    steps.push({ title: "Source check", state: "active", detail: "SLAPorSLOP is reading the source at this release's commit." });
  } else if (source.status === "not_checked") {
    steps.push({ title: "Source check", state: "skipped", detail: source.detail });
  } else if (source.status === "failed") {
    steps.push({
      title: "Source check",
      state: "active",
      detail: `SLAPorSLOP couldn't read the source (${source.detail}); it tries again every hour.`,
    });
  } else if (notes > 0) {
    steps.push({
      title: "Source check",
      state: "done",
      stateLabel: "Done, with notes",
      detail: (
        <>
          {notes} note{notes === 1 ? "" : "s"} about the licence or selling in the game: see the release below.
          <FixesLink slug={app.slug} />
        </>
      ),
    });
  } else {
    steps.push({
      title: "Source check",
      state: "done",
      detail: "The licence file matches, and there are no signs of selling in the game's source.",
    });
  }

  // Play goes to one address only: an older published one it no longer opens doesn't count.
  const outcome = outcomeSteps(app, latest, plays, true);
  // Play opens nothing meanwhile, when SLAPorSLOP withdrew the address it opened.
  if (opensNothing && latest.status !== "approved" && latest.status !== "revoked") {
    const last = outcome[outcome.length - 1];
    outcome[outcome.length - 1] = {
      ...last,
      more: (
        <>
          {last.more}
          {opensNothing}
        </>
      ),
    };
  }
  if (withdrawn && withdrawn !== latest && latest.status !== "fetching" && latest.status !== "awaiting_review" && latest.status !== "ready") {
    // The address Play opened was withdrawn, and nothing newer is published: nothing to play.
    outcome[outcome.length - 1] = {
      title: "Publish",
      state: "problem",
      stateLabel: "Withdrawn",
      detail: `Version ${withdrawn.version} was withdrawn by SLAPorSLOP: ${
        withdrawn.status_reason?.replace(/\.$/, "") ?? "no reason given"
      }. Play opens nothing until you publish a new address: submit one on the Releases page.`,
    };
  }
  steps.push(...outcome);
  return steps;
}

/** The step an app is stuck on or waiting for: the first one that needs attention, else the
 *  first one under way or not reached yet. Once every step is done, the last one. */
export function currentStep(steps: Step[]): Step {
  return (
    steps.find((s) => s.state === "problem") ??
    steps.find((s) => s.state === "active" || s.state === "waiting") ??
    steps[steps.length - 1]
  );
}

/** How often to look again: every 10 seconds while files are checked (each takes minutes, so
 *  that's plenty), every minute while it's on hold (staff, a re-scan or Defender's check may move
 *  it on) or passed while its source is still being read, otherwise not at all. */
export function refreshEvery(releases: Release[] | undefined): number | null {
  const latest = releases?.find((r) => r.status !== "withdrawn");
  if (latest?.status === "fetching") return 10_000;
  if (latest?.status === "awaiting_review") return 60_000;
  if (latest?.status === "ready" && (!latest.repo_scan || latest.repo_scan.status === "failed")) return 60_000;
  return null;
}

export { ICON as STEP_ICON, STATE_LABEL as STEP_LABEL };

/** A step's files, and how each one's check went or is going. */
export function StepItems({ items, label }: { items: SubItem[]; label: string }) {
  return (
    <ul className="step-files" aria-label={label}>
      {items.map((i) => (
        <li key={i.key} className={`step-file ${i.state}`}>
          <span className="step-icon" aria-hidden="true">
            {ICON[i.state]}
          </span>
          <div>
            <b>{i.label}</b> <span className="step-state">{i.status}</span>
            {i.detail && <div className="muted small">{i.detail}</div>}
          </div>
        </li>
      ))}
    </ul>
  );
}

/** "Put it in the store": a draft goes in the store the first time, by its maker (manage.rs
 *  `list_app`), signed in; never an AI agent. Asks first: only SLAPorSLOP staff take it out again. */
export function ListInStore({ slug, onListed }: { slug: string; onListed?: () => void }) {
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [done, setDone] = useState(false);
  const { opener, close } = useConfirmStep(confirm, setConfirm);
  // While SLAPorSLOP is in beta, going public may need the invite code first (invites.rs).
  const invite = useInviteGate();
  const list = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await invite.guard(() => api.listApp(slug));
      setConfirm(false);
      setDone(true);
      onListed?.();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  if (done) {
    return (
      <p className="notice small" role="status">
        It's in the store.
      </p>
    );
  }
  return (
    <div className="list-in-store">
      {!confirm ? (
        <button ref={opener} type="button" className="btn small slime" onClick={() => setConfirm(true)}>
          Put it in the store…
        </button>
      ) : (
        <div className="stage-confirm" role="group" aria-label="Put it in the store">
          <b>Put it in the store now?</b>
          <p className="small">
            Anyone can find its page and get what you've published. Taking it out again is for SLAPorSLOP staff.
          </p>
          <div className="row-actions">
            <button type="button" className="btn small slime" autoFocus disabled={busy} onClick={() => void list()}>
              {busy ? "Putting it in…" : "Yes, put it in the store"}
            </button>
            <button type="button" className="btn small ghost" disabled={busy} onClick={close}>
              Not yet
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {invite.prompt}
    </div>
  );
}

export function PublishProgress(props: { app: ProgressApp; releases: Release[]; server: Server; onListed?: () => void }) {
  // While files are checked, "started 4 minutes ago" moves on between refreshes.
  const checking = props.releases.some((r) => r.status === "fetching");
  const now = useNow(checking ? 15_000 : null);
  const steps = progressSteps(props.app, props.releases, props.server, now);
  return (
    <section className="panel panel-pad progress" aria-labelledby="progress-title">
      <h2 id="progress-title">Publishing progress</h2>
      <ol className="steps">
        {steps.map((s) => {
          const items = s.items ?? [];
          // Files that all went fine fold away; anything under way, wrong or not checked shows. An
          // address check a test setup doesn't make ("Not checked") isn't wrong, so it folds too.
          const open = items.some((i) => i.state !== "done" && (s.itemName !== "check" || i.state !== "skipped"));
          const each = s.itemName === "check" ? "each check" : "each file";
          return (
            <li key={s.title} className={`step ${s.state}`}>
              <span className="step-icon" aria-hidden="true">
                {ICON[s.state]}
              </span>
              <div>
                <b>{s.title}</b> <span className="step-state">{s.stateLabel ?? STATE_LABEL[s.state]}</span>
                <div className="muted small">{s.detail}</div>
                {s.more && <div className="muted small step-more">{s.more}</div>}
                {s.action === "list" && props.app.slug && <ListInStore slug={props.app.slug} onListed={props.onListed} />}
                {items.length > 0 &&
                  (open ? (
                    <StepItems items={items} label={`${s.title}: ${each}`} />
                  ) : (
                    <details className="step-details">
                      <summary className="small">{each.charAt(0).toUpperCase() + each.slice(1)}</summary>
                      <StepItems items={items} label={`${s.title}: ${each}`} />
                    </details>
                  ))}
              </div>
            </li>
          );
        })}
      </ol>
    </section>
  );
}
