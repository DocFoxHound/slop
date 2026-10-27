// SPDX-License-Identifier: AGPL-3.0-or-later
// Where an app is on its way into the store, step by step, for its maker.
// Built from the app, its releases (with each file's check, crates/slop-api/src/release_progress.rs)
// and what this server checks (GET /v1/server).

import type { ReactNode } from "react";
import { Link } from "react-router";

import type { AppSummary, FileCheckStage, Release, ReleaseFile } from "../api";
import { useNow } from "../hooks";

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
};

/** The scanner's longest run on one file (crates/slop-api/src/scan.rs). */
const LONGEST_SCAN_MINUTES = 30;

type Server = { scanning: boolean; maker_signatures_required: boolean };
type ProgressApp = Pick<AppSummary, "stage" | "status"> & { slug?: string };

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
  if (held(r) && f.cleared_reason) {
    return {
      ...base,
      state: "done",
      status: "Cleared by a reviewer",
      detail: sentence([`found: ${f.scan_detail ?? "a scanner finding"}; cleared: ${f.cleared_reason}`, ...extra]),
    };
  }
  if (held(r)) {
    return {
      ...base,
      state: "problem",
      status: "On hold",
      detail: sentence([f.scan_detail ?? "the scanner found something a person needs to look at", ...extra]),
    };
  }
  if (r === "note") return { ...base, state: "done", status: "Passed, with a note", detail: sentence([f.scan_detail, took, ...extra]) };
  if (r === "passed") return { ...base, state: "done", status: "Passed", detail: sentence([took, ...extra]) };
  // Scanning off (test servers): downloaded and hashed only.
  return { ...base, state: "done", status: "Hashed", detail: sentence([took, ...extra]) };
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
  // The release that matters: the newest one, unless an approved one is newer than a failure.
  // One withdrawn before review never counts.
  releases = releases.filter((r) => r.status !== "withdrawn");
  const latest = releases[0];
  const approved = releases.find((r) => r.status === "approved");
  // The API leaves `status` out once an app is listed.
  const listed = app.status === undefined || app.status === "listed";
  const steps: Step[] = [
    {
      title: "Store page",
      state: "done",
      detail: "Your store page is saved as a draft.",
    },
  ];

  if (comingSoon) {
    steps.push({
      title: "Release",
      state: "skipped",
      detail: "Coming soon pages go in the store without a build. Submit a release when you move to beta.",
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
    const onHold = files.filter((f) => held(f.scan_result) && !f.cleared_reason);
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
    } else if (onHold.length > 0) {
      steps.push({
        title: "Scan",
        state: "problem",
        stateLabel: "On hold",
        detail: `${onHold.length === 1 ? `${fileLabel(onHold[0])} is` : `${onHold.length} files are`} on hold: ${
          onHold[0].scan_detail?.replace(/\.$/, "") ?? "a scanner found something to look at"
        }.`,
        more:
          "Scanners make mistakes, so a person at SLAPorSLOP looks before anything else happens. If it's a false positive, they clear it and tell you why. If the finding is real, fix the build and submit a new version.",
      });
    } else {
      steps.push({
        title: "Scan",
        state: "done",
        detail: scans.includes("note") ? "Passed, with notes for the reviewer." : "Passed.",
      });
    }

    const source = latest.repo_scan;
    const notes =
      (source?.licence?.problems.length ?? 0) +
      (source?.monetization?.length ?? 0) +
      (source?.code_loading?.length ?? 0) +
      (source?.saves?.length ?? 0) +
      (source?.online_undeclared ? 1 : 0);
    const waitingForPerson = latest.status === "awaiting_review";
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
        detail: `SLAPorSLOP couldn't read the source (${source.detail}); it tries again within the hour.`,
      });
    } else if (notes > 0) {
      // Notes, not failures: once a person approves the release, they've looked at them.
      steps.push({
        title: "Source check",
        state: waitingForPerson ? "problem" : "done",
        stateLabel: waitingForPerson ? "Notes for you and the reviewer" : undefined,
        detail: (
          <>
            {notes} note{notes === 1 ? "" : "s"} about the licence or selling in the app, for you and the reviewer: see the release
            below.
            {waitingForPerson && <FixesLink slug={app.slug} />}
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

    if (latest.status === "approved" || (approved && !waitingForPerson && !fetching)) {
      const r = latest.status === "approved" ? latest : (approved ?? latest);
      steps.push({
        title: "Review by a person",
        state: "done",
        detail: `Version ${r.version} ${r.auto_approved ? "approved automatically as an update" : "approved"} and signed by SLAPorSLOP.`,
      });
    } else if (waitingForPerson) {
      const why = latest.status_reason?.replace(/^Waiting for a person: /, "").replace(/\.$/, "");
      steps.push({
        title: "Review by a person",
        state: "active",
        stateLabel: "Waiting for a person",
        detail:
          onHold.length > 0
            ? "Waiting for review by a person at SLAPorSLOP, who looks at the file on hold first."
            : "Waiting for review by a person at SLAPorSLOP. It usually takes a day or two.",
        more: (
          <>
            {why && <>Why a person: {why}. </>}
            What happens next: they look at the checks above and the app itself. If it's approved, SLAPorSLOP signs it and players can
            get it; if something needs changing, you're told what. Either way you get a notification (under 🔔, and by email).
            Nothing to do meanwhile.
          </>
        ),
      });
    } else if (latest.status === "rejected") {
      steps.push({
        title: "Review by a person",
        state: "problem",
        stateLabel: "Not approved",
        detail: (
          <>
            Not approved: {latest.status_reason?.replace(/\.$/, "") ?? "no reason given"}. Fix it and submit a new version.
            <FixesLink slug={app.slug} />
          </>
        ),
      });
    } else if (latest.status === "revoked") {
      steps.push({
        title: "Review by a person",
        state: "problem",
        detail: `Withdrawn by SLAPorSLOP: ${latest.status_reason ?? "no reason given"}.`,
      });
    } else {
      steps.push({
        title: "Review by a person",
        state: "waiting",
        detail: approved
          ? "Comes after the checks above. Your approved version stays live meanwhile."
          : "Comes after the checks above: a person at SLAPorSLOP reviews each app's first release.",
      });
    }
  }

  if (listed) {
    steps.push({
      title: "In the store",
      state: "done",
      detail: "Players can find it.",
    });
  } else if (app.status === "delisted") {
    steps.push({
      title: "In the store",
      state: "problem",
      detail: "Taken out of the store by SLAPorSLOP staff.",
    });
  } else if (comingSoon || approved) {
    steps.push({
      title: "In the store",
      state: "active",
      detail: "Ready: waiting for SLAPorSLOP staff to put it in the store.",
    });
  } else {
    steps.push({
      title: "In the store",
      state: "waiting",
      detail: "After a release is approved.",
    });
  }
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
 *  that's plenty) and while SLAPorSLOP then reads the source (it starts once the files are in, so the
 *  release is waiting for review before its source check shows), every minute while a person
 *  reviews it, otherwise not at all. */
export function refreshEvery(releases: Release[] | undefined): number | null {
  const latest = releases?.find((r) => r.status !== "withdrawn");
  if (latest?.status === "fetching") return 10_000;
  if (latest?.status === "awaiting_review" && !latest.repo_scan) return 10_000;
  if (latest?.status === "awaiting_review") return 60_000;
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

export function PublishProgress(props: { app: ProgressApp; releases: Release[]; server: Server }) {
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
          // Files that all went fine fold away; anything under way or wrong shows.
          const open = items.some((i) => i.state !== "done");
          return (
            <li key={s.title} className={`step ${s.state}`}>
              <span className="step-icon" aria-hidden="true">
                {ICON[s.state]}
              </span>
              <div>
                <b>{s.title}</b> <span className="step-state">{s.stateLabel ?? STATE_LABEL[s.state]}</span>
                <div className="muted small">{s.detail}</div>
                {s.more && <div className="muted small step-more">{s.more}</div>}
                {items.length > 0 &&
                  (open ? (
                    <StepItems items={items} label={`${s.title}: each file`} />
                  ) : (
                    <details className="step-details">
                      <summary className="small">Each file</summary>
                      <StepItems items={items} label={`${s.title}: each file`} />
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
