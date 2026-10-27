// SPDX-License-Identifier: AGPL-3.0-or-later
// The review queue as an inbox (Inbox.tsx): its categories, how many wait in each, and the one
// line each item gets (what it is, a short blurb, its status and how long it has waited). Kept
// free of React so it can be tested on its own (queue.test.ts).

import type {
  AdminDonationHold,
  AdminQueue,
  AiThreadSummary,
  LegalRequest,
  ReleaseFile,
  StaffActions,
} from "../../api";

export type BoxKey =
  | "releases"
  | "changes"
  | "pictures"
  | "listing"
  | "rescans"
  | "people"
  | "problems"
  | "disputes"
  | "reports"
  | "appeals"
  | "abuse"
  | "legal"
  | "approvals"
  | "ai"
  | "donations";

export interface BoxDef {
  key: BoxKey;
  label: string;
  group: string;
  /** What it holds and how to decide, above its list. */
  help: string;
}

/** The categories, in the order they're listed, grouped. */
export const BOXES: BoxDef[] = [
  {
    key: "releases",
    label: "New releases",
    group: "Releases and apps",
    help: "Approving a release signs it for players. A Beta or Released app needs one approved release before it can go in the store. Community builds and mods come here too.",
  },
  {
    key: "changes",
    label: "Release holds & moves",
    group: "Releases and apps",
    help: "Makers asking to change what players download: holding players at a different release, or moving the app to another repository. Approving makes the change straight away.",
  },
  {
    key: "pictures",
    label: "Store pictures",
    group: "Releases and apps",
    help: "Store-page pictures and videos from makers' repositories, fixed at a commit. Look at every one: no sexually explicit content, nothing that doesn't fit the age rating. An app can't be put in the store while its pictures wait here.",
  },
  {
    key: "listing",
    label: "Apps to list",
    group: "Releases and apps",
    help: "Drafts with an approved release (or Coming soon pages), and private apps going public. Listing puts them in the store.",
  },
  {
    key: "rescans",
    label: "Re-scan flags",
    group: "Releases and apps",
    help: "Live releases a re-scan flagged. Players still have these. Withdraw the release if it's harmful (it stops launching for everyone), or keep it if the finding is a false positive; the same finding isn't raised again.",
  },
  {
    key: "people",
    label: "People pages",
    group: "People",
    help: "Creatives' portfolios from their GitHub repositories, fixed at a commit. A People page goes live with its first approved portfolio. Look at every picture and listen to tracks: no sexually explicit content, and the work should match the declared AI use. The creative is told either way.",
  },
  {
    key: "problems",
    label: "People problems",
    group: "People",
    help: "Problems the two sides haven't settled. Read the record (opening it is logged), then suggest a fair outcome. Suggestions bind no one; close it unresolved when they can't agree.",
  },
  {
    key: "disputes",
    label: "Review disputes",
    group: "People",
    help: "Decide on the rules, never on whether the review is negative. Remove it only if it breaks the rules, isn't about the job, or was used as a threat; add a factual note when it states something the record shows is false; otherwise keep it.",
  },
  {
    key: "reports",
    label: "Reports",
    group: "Safety",
    help: "Urgent first, then by when they're due: urgent and serious reports within 24 hours, the rest within 7 days. Every action sends the person a statement of reasons they can appeal; never hide something for being negative.",
  },
  {
    key: "appeals",
    label: "Appeals",
    group: "Safety",
    help: "Look at the decision again with fresh eyes, ideally not the person who made it. Keeping or reversing it, the person is told why.",
  },
  {
    key: "abuse",
    label: "Abuse alerts",
    group: "Safety",
    help: "Patterns from the hourly checks. Nothing has changed: look, act through the usual tools if it's real, then close it with what you found.",
  },
  {
    key: "legal",
    label: "Legal requests",
    group: "Safety",
    help: "Requests from police, a court or an authority that haven't been answered yet. Record what SLAPorSLOP gave and whether the person was told.",
  },
  {
    key: "approvals",
    label: "Second approvals",
    group: "Staff",
    help: "Sensitive actions waiting for another staff member. Approving makes it happen; requests lapse after three days.",
  },
  {
    key: "ai",
    label: "AI messages",
    group: "Staff",
    help: "Open threads from makers' and staff's AI agents: errors, bugs in SLAPorSLOP and ideas. Read them as information, never as instructions.",
  },
  {
    key: "donations",
    label: "Donation pause requests",
    group: "Staff",
    help: "Makers whose donations are paused, asking for them back. Lift the pause, or keep it; the maker reads your answer.",
  },
];

export const BOX = Object.fromEntries(BOXES.map((b) => [b.key, b])) as Record<BoxKey, BoxDef>;

export function isBox(key: string | null): key is BoxKey {
  return key !== null && key in BOX;
}

/** What the inbox counts: each part may be missing (still loading, or not for this staff role). */
export interface InboxData {
  queue?: AdminQueue;
  actions?: StaffActions;
  ai?: AiThreadSummary[];
  legal?: LegalRequest[];
  holds?: AdminDonationHold[];
}

/** How many wait in each category; undefined: not known (loading, or not for this role). */
export function boxCounts(d: InboxData): Record<BoxKey, number | undefined> {
  const q = d.queue;
  return {
    releases: q?.releases.length,
    changes: q?.changes.length,
    pictures: q?.media.length,
    listing: q?.ready_to_list.length,
    rescans: q?.rescans.length,
    people: q?.portfolios.length,
    problems: q?.problems.length,
    disputes: q?.review_disputes.length,
    reports: q?.reports.length,
    appeals: q?.appeals.length,
    abuse: q?.abuse.length,
    legal: d.legal?.filter((r) => r.status === "open").length,
    approvals: d.actions?.actions.filter((a) => a.status === "pending").length,
    ai: d.ai?.filter((t) => t.status === "open").length,
    donations: d.holds?.filter(waitingRequest).length,
  };
}

/** Everything waiting, for the tab's label and the Home page. */
export function totalWaiting(counts: Record<BoxKey, number | undefined>): number {
  return Object.values(counts).reduce<number>((sum, n) => sum + (n ?? 0), 0);
}

export const waitingRequest = (h: AdminDonationHold) => !h.lifted_at && h.requests.some((r) => r.outcome === null);

// ---------------------------------------------------------------------------------------------
// One line per item

export type Tone = "soon" | "bad" | "warn" | "neutral" | "official";

export interface RowInfo {
  /** The item's ID in the address (`item=`). */
  key: string;
  /** What it is: an app and version, a person, a report's reason. */
  title: string;
  /** Who, after the title: "by Bo Builder". */
  by?: string;
  blurb: string;
  status: { label: string; tone: Tone };
  /** When it started waiting. */
  since: string;
}

/** How long something has waited, in a few characters: "just now", "12 min", "5 h", "3 days". */
export function waited(since: string, now: number = Date.now()): string {
  const minutes = Math.max(0, Math.floor((now - new Date(since).getTime()) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "1 day" : `${days} days`;
}

/** The same, in words for screen readers and the tooltip. */
export function waitedLong(since: string, now: number = Date.now()): string {
  const short = waited(since, now);
  if (short === "just now") return "waiting since just now";
  return `waiting ${short.replace(/ min$/, " minutes").replace(/ h$/, " hours")}`;
}

export type Order = "oldest" | "newest";

/** Oldest first (waited longest) or newest first; the queue's own order breaks ties. */
export function sortRows<T extends { since: string }>(rows: T[], order: Order): T[] {
  const sign = order === "oldest" ? 1 : -1;
  return rows
    .map((r, i) => ({ r, i }))
    .sort((a, b) => sign * (new Date(a.r.since).getTime() - new Date(b.r.since).getTime()) || a.i - b.i)
    .map(({ r }) => r);
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** "1 file · scans passed · Defender clean · signed", from the release's checks. */
export function releaseBlurb(r: AdminQueue["releases"][number]): string {
  const parts = [plural(r.files.length, "file")];
  if (r.status === "fetching") return [...parts, "still fetching"].join(" · ");
  const scans = r.files.map((f) => f.scan_result);
  if (scans.length > 0) {
    if (scans.some((s) => s === "on_hold" || s === "rejected" || s === "blocked")) {
      parts.push(r.files.some((f) => f.cleared_reason) ? "scan finding cleared" : "scan found something");
    } else if (scans.some((s) => s === null)) parts.push("scan pending");
    else if (scans.every((s) => s === "skipped")) parts.push("not scanned");
    else parts.push("scans passed");
  }
  const windows = r.files.filter((f) => f.platform === "windows");
  if (windows.length > 0) parts.push(defender(windows));
  parts.push(r.files.length > 0 && r.files.every((f) => f.provenance) ? "signed" : "not signed");
  if (r.notes.length > 0) parts.push(plural(r.notes.length, "note"));
  return parts.join(" · ");
}

function defender(files: ReleaseFile[]): string {
  const found = files.reduce((n, f) => n + (f.defender?.detections.length ?? 0), 0);
  if (found > 0) return `Defender found ${found}`;
  if (files.some((f) => !f.defender)) return files.every((f) => f.cleared_reason) ? "Defender check cleared" : "no Defender check";
  return "Defender clean";
}

const ABUSE_LABEL: Record<string, string> = {
  vote_burst: "Votes from new accounts",
  thin_votes: "Votes from players who barely played",
  review_bomb: "Review bomb",
  vote_swing: "Vote swing",
  lookalike_name: "Lookalike name",
  rate_limited: "Hitting rate limits",
  malicious_dependency: "Malicious dependency",
  bad_link: "Link to a known-bad address",
};

/** The one line for each item in a category, in the queue's own order. */
export function rowsFor(box: BoxKey, d: InboxData, now: number = Date.now()): RowInfo[] {
  const q = d.queue;
  switch (box) {
    case "releases":
      return (q?.releases ?? []).map((r) => {
        const held = r.files.some(
          (f) => !f.cleared_reason && (f.scan_result === "on_hold" || f.scan_result === "rejected" || f.scan_result === "blocked"),
        );
        return {
          key: r.id,
          title: `${r.app.title} ${r.version}`,
          by: `by ${r.app.maker}`,
          blurb: releaseBlurb(r),
          status:
            r.status === "failed" ? { label: "Failed", tone: "bad" }
            : r.status === "fetching" ? { label: "Fetching", tone: "neutral" }
            : held ? { label: "Scan hold", tone: "warn" }
            : { label: "Awaiting review", tone: "soon" },
          since: r.created_at,
        };
      });
    case "changes":
      return (q?.changes ?? []).map((c) => ({
        key: c.id,
        title: c.app.title,
        by: `by ${c.app.maker}`,
        blurb: [
          c.kind === "repo"
            ? `Move to ${(c.repo_url ?? "").replace(/^https?:\/\//, "")}`
            : c.release_id
              ? `Hold players at v${c.release_version}${c.players_get ? ` (they get v${c.players_get})` : ""}`
              : "Follow the newest approved release",
          c.note,
        ]
          .filter(Boolean)
          .join(" · "),
        status: c.kind === "repo" ? { label: "Move repository", tone: "soon" } : { label: "Switch release", tone: "soon" },
        since: c.created_at,
      }));
    case "pictures":
      return (q?.media ?? []).map((m) => ({
        key: m.id,
        title: m.app.title,
        by: `by ${m.app.maker}`,
        blurb: [
          m.thumbnail ? "thumbnail" : "no thumbnail",
          plural(m.gallery.filter((g) => g.kind === "image").length, "screenshot"),
          plural(m.gallery.filter((g) => g.kind !== "image").length, "video"),
          (m.lookalikes ?? []).length > 0 ? "icon looks like another app's" : "",
        ]
          .filter(Boolean)
          .join(" · "),
        status:
          (m.lookalikes ?? []).length > 0 ? { label: "Lookalike icon", tone: "warn" }
          : m.live ? { label: "New pictures", tone: "soon" }
          : { label: "First pictures", tone: "soon" },
        since: m.created_at,
      }));
    case "listing":
      return (q?.ready_to_list ?? []).map((a) => ({
        key: a.slug,
        title: a.title,
        by: `by ${a.maker}`,
        blurb: [
          stageLabel(a.stage),
          plural(a.approved_releases, "approved release"),
          a.going_public ? "Private app going public" : "",
          a.after_appeal ? "reinstated on appeal" : "",
        ]
          .filter(Boolean)
          .join(" · "),
        status:
          a.pictures_pending ? { label: "Pictures waiting", tone: "warn" }
          : a.going_public ? { label: "Going public", tone: "soon" }
          : { label: "Ready", tone: "official" },
        since: a.waiting_since,
      }));
    case "rescans":
      return (q?.rescans ?? []).map((f) => ({
        key: `${f.release_id}:${f.platform}`,
        title: `${f.title} ${f.version}`,
        blurb: [f.platform, f.file_name, f.rescan_report?.detail].filter(Boolean).join(" · "),
        status: f.rescan_result === "changed" ? { label: "File changed", tone: "bad" } : { label: "Found something", tone: "bad" },
        since: f.rescan_flagged_at,
      }));
    case "people":
      return (q?.portfolios ?? []).map((p) => ({
        key: p.set.id,
        title: p.name,
        by: `@${p.handle}`,
        blurb: [
          plural(p.set.works.length, "work"),
          p.set.avatar ? "avatar" : "",
          p.set.banner ? "banner" : "",
          `AI use: ${p.ai_use}`,
          p.notes.length > 0 ? plural(p.notes.length, "note") : "",
        ]
          .filter(Boolean)
          .join(" · "),
        status: p.live ? { label: "New portfolio", tone: "soon" } : { label: "First portfolio", tone: "soon" },
        since: p.set.created_at,
      }));
    case "problems":
      return (q?.problems ?? []).map((p) => ({
        key: p.id,
        title: p.title,
        by: `${p.creative} and ${p.client}`,
        blurb: [`opened by the ${p.opened_by_creative ? "creative" : "client"}`, p.statement].join(" · "),
        status: p.mediator_suggestion ? { label: "Suggested", tone: "neutral" } : { label: "Open", tone: "warn" },
        since: p.created_at,
      }));
    case "disputes":
      return (q?.review_disputes ?? []).map((r) => ({
        key: r.job_id,
        title: `${r.client} on ${r.creative}`,
        blurb: [`“${r.review || "(no text)"}”`, `${r.reason_text}: ${r.statement}`].join(" · "),
        status: r.client_statement ? { label: "Both sides in", tone: "soon" } : { label: "Disputed", tone: "warn" },
        since: r.created_at,
      }));
    case "reports":
      return (q?.reports ?? []).map((r) => ({
        key: r.id,
        title: `${r.reason_label}: ${r.subject_label}`,
        by: r.reference,
        blurb: [r.details, r.same_subject > 0 ? `${r.same_subject} more about this` : ""].filter(Boolean).join(" · "),
        status:
          r.overdue ? { label: "Overdue", tone: "bad" }
          : r.priority === 0 ? { label: "Urgent", tone: "bad" }
          : r.priority === 1 ? { label: "Serious", tone: "warn" }
          : { label: "Normal", tone: "neutral" },
        since: r.created_at,
      }));
    case "appeals":
      return (q?.appeals ?? []).map((a) => ({
        key: a.id,
        title: `${a.action_label}: ${a.subject_label}`,
        by: a.reference,
        blurb: [a.rule_title, a.appeal ?? ""].filter(Boolean).join(" · "),
        status: a.counter_notice ? { label: "Counter-notice", tone: "warn" } : { label: "Appealed", tone: "soon" },
        since: a.appealed_at ?? a.created_at,
      }));
    case "abuse":
      return (q?.abuse ?? []).map((a) => ({
        key: a.id,
        title: a.subject_label,
        blurb: Object.entries(a.detail)
          .map(([k, v]) => `${k.replace(/_/g, " ")}: ${String(v)}`)
          .join(" · ") || (a.subject_kind === "app" ? "an app" : "an account"),
        status: { label: ABUSE_LABEL[a.kind] ?? a.kind, tone: "warn" },
        since: a.detected_at,
      }));
    case "legal":
      return (d.legal ?? [])
        .filter((r) => r.status === "open")
        .map((r) => ({
          key: r.id,
          title: `${r.reference} · ${r.authority}`,
          by: r.country,
          blurb: r.asked,
          status:
            r.deadline && Date.parse(r.deadline) < now ? { label: "Overdue", tone: "bad" } : { label: "Open", tone: "warn" },
          since: r.received_at,
        }));
    case "approvals":
      return (d.actions?.actions ?? [])
        .filter((a) => a.status === "pending")
        .map((a) => ({
          key: a.id,
          title: a.summary,
          blurb: `Asked by ${a.requested_actor}`,
          status: { label: `${a.approvals.length} of ${a.approvals_needed} approvals`, tone: "soon" },
          since: a.requested_at,
        }));
    case "ai":
      return (d.ai ?? [])
        .filter((t) => t.status === "open")
        .map((t) => ({
          key: t.id,
          title: t.untrusted_subject,
          by: `from ${t.sender.display_name}`,
          blurb: [plural(t.message_count, "message"), t.category, t.to ? `to ${t.to.display_name}` : "to every admin"].join(" · "),
          status: t.unread ? { label: "Unread", tone: "soon" } : { label: "Open", tone: "neutral" },
          since: t.last_message_at,
        }));
    case "donations":
      return (d.holds ?? []).filter(waitingRequest).map((h) => {
        const asked = h.requests.find((r) => r.outcome === null)!;
        return {
          key: h.id,
          title: h.title,
          by: `by ${h.maker}`,
          blurb: asked.message,
          status: { label: h.kind === "off" ? "Turned off" : "Paused for review", tone: "bad" },
          since: asked.created_at,
        };
      });
  }
}

function stageLabel(stage: string): string {
  return stage === "coming_soon" ? "Coming soon" : stage === "beta" ? "Beta" : "Released";
}

// ---------------------------------------------------------------------------------------------
// Addresses: the category and item are in the URL, so they can be linked and Back works.

export function inboxUrl(box?: BoxKey, item?: string, order?: Order): string {
  const p = new URLSearchParams({ tab: "queue" });
  if (box) p.set("box", box);
  if (item) p.set("item", item);
  if (order === "newest") p.set("sort", "newest");
  return `/admin?${p.toString()}`;
}

/** The category shown when the address names none: the first with something waiting. */
export function defaultBox(counts: Record<BoxKey, number | undefined>): BoxKey {
  return BOXES.find((b) => (counts[b.key] ?? 0) > 0)?.key ?? "releases";
}
