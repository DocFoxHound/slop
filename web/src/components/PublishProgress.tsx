// Where an app is on its way into the store, step by step, for its maker.
// Built from the app, its releases and what this server checks (GET /v1/server).

import type { ReactNode } from "react";

import type { AppSummary, Release } from "../api";

type State = "done" | "active" | "problem" | "waiting" | "skipped";

export interface Step {
  title: string;
  state: State;
  detail: ReactNode;
}

const ICON: Record<State, string> = { done: "✓", active: "…", problem: "!", waiting: "○", skipped: "–" };
const STATE_LABEL: Record<State, string> = {
  done: "Done",
  active: "In progress",
  problem: "Needs attention",
  waiting: "Not yet",
  skipped: "Not checked",
};

/** The steps, from the app's own data. Exported for tests. */
export function progressSteps(
  app: Pick<AppSummary, "stage" | "status">,
  releases: Release[],
  server: { scanning: boolean; maker_signatures_required: boolean },
): Step[] {
  const comingSoon = app.stage === "coming_soon";
  // The release that matters: the newest one, unless an approved one is newer than a failure.
  const latest = releases[0];
  const approved = releases.find((r) => r.status === "approved");
  // The API leaves `status` out once an app is listed.
  const listed = app.status === undefined || app.status === "listed";
  const steps: Step[] = [{ title: "Store page", state: "done", detail: "Your store page is saved as a draft." }];

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
    steps.push({ title: "Release", state: "done", detail: `Version ${latest.version} received.` });

    const files = latest.files;
    if (latest.status === "fetching") {
      steps.push({ title: "Files checked", state: "active", detail: "Slop is downloading each file once to record its size and hash." });
    } else if (latest.status === "failed") {
      steps.push({ title: "Files checked", state: "problem", detail: latest.status_reason ?? "Slop couldn't check the files." });
    } else {
      steps.push({ title: "Files checked", state: "done", detail: `${files.length} file${files.length === 1 ? "" : "s"} hashed.` });
    }

    const unsigned = files.filter((f) => !f.provenance);
    if (latest.status === "fetching") {
      steps.push({ title: "Maker signature", state: "waiting", detail: "Checked once the files are." });
    } else if (unsigned.length === 0) {
      steps.push({ title: "Maker signature", state: "done", detail: "Every file is signed by your build." });
    } else if (!server.maker_signatures_required) {
      steps.push({ title: "Maker signature", state: "skipped", detail: "Not required on this server (private test setup)." });
    } else {
      steps.push({
        title: "Maker signature",
        state: "problem",
        detail: unsigned[0].provenance_problem ?? "A file isn't signed by your build.",
      });
    }

    const scans = files.map((f) => f.scan_result);
    if (latest.status === "fetching" || scans.some((s) => s === null)) {
      steps.push({ title: "Scan", state: "waiting", detail: "Runs once the files are checked." });
    } else if (!server.scanning || scans.every((s) => s === "skipped")) {
      steps.push({ title: "Scan", state: "skipped", detail: "Automatic scanning isn't running on this server yet." });
    } else if (scans.some((s) => s === "rejected" || s === "blocked" || s === "on_hold")) {
      steps.push({ title: "Scan", state: "problem", detail: files.find((f) => f.scan_detail)?.scan_detail ?? "A scan found a problem." });
    } else {
      steps.push({ title: "Scan", state: "done", detail: "Passed." });
    }

    if (latest.status === "approved" || (approved && latest.status !== "awaiting_review")) {
      steps.push({ title: "Review", state: "done", detail: `Version ${(approved ?? latest).version} approved and signed by Slop.` });
    } else if (latest.status === "awaiting_review") {
      steps.push({ title: "Review", state: "active", detail: "Waiting for a Slop reviewer. This is done by a person, so it can take a while." });
    } else if (latest.status === "rejected") {
      steps.push({ title: "Review", state: "problem", detail: `Not approved: ${latest.status_reason ?? "no reason given"}. Fix it and submit a new version.` });
    } else if (latest.status === "revoked") {
      steps.push({ title: "Review", state: "problem", detail: `Withdrawn by Slop: ${latest.status_reason ?? "no reason given"}.` });
    } else {
      steps.push({ title: "Review", state: "waiting", detail: "Comes after the checks above." });
    }
  }

  if (listed) {
    steps.push({ title: "In the store", state: "done", detail: "Players can find it." });
  } else if (app.status === "delisted") {
    steps.push({ title: "In the store", state: "problem", detail: "Taken out of the store by Slop staff." });
  } else if (comingSoon || approved) {
    steps.push({ title: "In the store", state: "active", detail: "Ready: waiting for Slop staff to put it in the store." });
  } else {
    steps.push({ title: "In the store", state: "waiting", detail: "After a release is approved." });
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

export { ICON as STEP_ICON, STATE_LABEL as STEP_LABEL };

export function PublishProgress(props: {
  app: Pick<AppSummary, "stage" | "status">;
  releases: Release[];
  server: { scanning: boolean; maker_signatures_required: boolean };
}) {
  const steps = progressSteps(props.app, props.releases, props.server);
  return (
    <section className="panel panel-pad progress" aria-labelledby="progress-title">
      <h2 id="progress-title">Publishing progress</h2>
      <ol className="steps">
        {steps.map((s) => (
          <li key={s.title} className={`step ${s.state}`}>
            <span className="step-icon" aria-hidden="true">
              {ICON[s.state]}
            </span>
            <div>
              <b>{s.title}</b> <span className="step-state">{STATE_LABEL[s.state]}</span>
              <div className="muted small">{s.detail}</div>
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}
