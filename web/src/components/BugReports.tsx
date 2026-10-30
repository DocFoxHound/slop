// SPDX-License-Identifier: AGPL-3.0-or-later
// Bug reports (crates/slop-api/src/bug_reports.rs): the crash prompt in the SLAPorSLOP app, the
// player's form on an app's page, and the maker's list and feedback per release.

import { useEffect, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";

import { api, ApiError, type BugReport, type ReleaseFeedback } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { desktop, detectOs, isDesktop } from "../platform";
import { InlineIcon } from "./StoreIcons";

type Crash = { slug: string; code: number | null; log: string };

/** In the SLAPorSLOP app: an app closed with an error. Offers a bug report; sends nothing itself. */
export function CrashPrompt() {
  const [crash, setCrash] = useState<Crash>();
  const navigate = useNavigate();
  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    void desktop.onAppCrashed((e) => setCrash(e)).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);
  if (!crash) return null;
  return (
    <div className="notice crash-prompt" role="alert">
      <p>
        <b>{crash.slug}</b> closed with an error{crash.code !== null ? ` (code ${crash.code})` : ""}. Tell its maker? You'll
        see exactly what's sent first.
      </p>
      <div className="row-actions">
        <button
          type="button"
          className="btn small slime"
          onClick={() => {
            navigate(`/app/${crash.slug}#bug-reports`, { state: { crash } });
            setCrash(undefined);
          }}
        >
          Report it
        </button>
        <button type="button" className="btn small ghost" onClick={() => setCrash(undefined)}>
          No thanks
        </button>
      </div>
    </div>
  );
}

const STATUS: Record<BugReport["status"], string> = {
  open: "OPEN",
  fixed: "FIXED",
  wont_fix: "WON'T FIX",
  duplicate: "DUPLICATE",
};

/** On an app's page, for players who played it: open reports ("me too") and a form. */
export function BugReports({ slug, version }: { slug: string; version?: string }) {
  const location = useLocation();
  const crash = (location.state as { crash?: Crash } | null)?.crash;
  // "all": the open ones (to say "me too") and the player's own whatever their status, so the
  // maker's answer shows on a report they closed (the API leaves out others' closed ones).
  const list = useLoad(() => api.bugReports.list(slug, "all").catch(() => null), [slug]);
  const [open, setOpen] = useState(Boolean(crash));
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [sendLog, setSendLog] = useState(Boolean(crash?.log));
  const [log, setLog] = useState(crash?.log ?? "");
  const [error, setError] = useState<string>();
  const [sent, setSent] = useState(false);
  if (!list.data || (!list.data.can_report && !list.data.is_maker)) return null;
  const l = list.data;
  const submit = async () => {
    setError(undefined);
    try {
      await api.bugReports.file(slug, {
        title,
        body,
        version,
        platform: ["windows", "linux", "mac"].includes(detectOs()) ? detectOs() : undefined,
        log: sendLog && log.trim() ? log : undefined,
      });
      setSent(true);
      setOpen(false);
      list.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't send the report.");
    }
  };
  return (
    <section className="panel panel-pad" id="bug-reports" aria-labelledby="bugs-title">
      <h2 id="bugs-title">Bug reports</h2>
      {l.is_maker ? (
        <p className="muted small">
          Players' reports are on your <Link to={`/publish/${slug}/manage#bug-reports`}>Manage page</Link>.
        </p>
      ) : (
        <p className="muted small">Something broken? Tell the maker. Only they and SLAPorSLOP staff see the details.</p>
      )}
      {!l.is_maker && l.reports.length > 0 && (
        <ul className="offer-list">
          {l.reports.map((r) => (
            <li key={r.id}>
              <span className="badge">{STATUS[r.status]}</span> {r.title}{" "}
              <span className="faint small">
                {r.me_too > 0 && `${r.me_too + 1} players · `}
                {formatDate(r.created_at)}
              </span>
              {r.mine && r.maker_note && <p className="small">The maker: {r.maker_note}</p>}
              {!r.mine && r.status === "open" && (
                <button
                  type="button"
                  className={`btn small ${r.i_have_it_too ? "slime" : "ghost"}`}
                  aria-pressed={r.i_have_it_too}
                  onClick={() => void api.bugReports.meToo(r.id, !r.i_have_it_too).then(list.reload)}
                >
                  I have this too
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      {sent && <p role="status">Sent to the maker. You'll be told when they answer.</p>}
      {l.can_report && !l.is_maker && !open && (
        <button type="button" className="btn small" onClick={() => setOpen(true)}>
          Report a bug
        </button>
      )}
      {open && l.can_report && (
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <label>
            What's wrong, in a line
            <input value={title} maxLength={120} required onChange={(e) => setTitle(e.target.value)} />
          </label>
          <label>
            What happened, and how to make it happen again
            <textarea value={body} maxLength={4000} rows={4} required onChange={(e) => setBody(e.target.value)} />
          </label>
          {crash?.log && (
            <>
              <label className="check">
                <input type="checkbox" checked={sendLog} onChange={(e) => setSendLog(e.target.checked)} /> Include the
                game's last output lines (below; your home folder is replaced with ~). Edit out anything private.
              </label>
              {sendLog && (
                <label>
                  Output to send
                  <textarea className="code-block" value={log} rows={8} onChange={(e) => setLog(e.target.value)} />
                </label>
              )}
            </>
          )}
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <div className="row-actions">
            <button className="btn small slime" disabled={!title.trim() || !body.trim()}>
              Send to the maker
            </button>
            <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
              Cancel
            </button>
          </div>
        </form>
      )}
    </section>
  );
}

/** A GitHub "new issue" link with the report filled in: the maker files it; SLAPorSLOP writes nothing. */
function issueLink(repoUrl: string, r: BugReport): string | null {
  if (!repoUrl.startsWith("https://github.com/")) return null;
  const log = r.log ? `\n\n<details><summary>Last output</summary>\n\n\`\`\`\n${r.log.slice(-1500)}\n\`\`\`\n</details>` : "";
  const body =
    `${r.body ?? ""}\n\nVersion: ${r.version ?? "?"} · Platform: ${r.platform ?? "?"} · ` +
    `${r.me_too + 1} player${r.me_too ? "s" : ""} on SLAPorSLOP${log}`;
  return `${repoUrl.replace(/\/$/, "")}/issues/new?title=${encodeURIComponent(r.title)}&body=${encodeURIComponent(body)}`;
}

/** The maker's view: every report in full, to triage. */
export function BugReportsPanel({ slug, repoUrl, accepting }: { slug: string; repoUrl: string; accepting: boolean }) {
  const [status, setStatus] = useState("open");
  const list = useLoad(() => api.bugReports.list(slug, status), [slug, status]);
  const [error, setError] = useState<string>();
  const update = async (id: string, change: { status?: BugReport["status"]; maker_note?: string }) => {
    setError(undefined);
    try {
      await api.bugReports.update(id, change);
      list.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't save that.");
    }
  };
  return (
    <section className="panel panel-pad" id="bug-reports" aria-labelledby="maker-bugs-title">
      <h2 id="maker-bugs-title">Bug reports</h2>
      {!accepting && <p className="notice small">You've turned bug reports off on the store page.</p>}
      <label>
        Show{" "}
        <select value={status} onChange={(e) => setStatus(e.target.value)}>
          <option value="open">Open</option>
          <option value="fixed">Fixed</option>
          <option value="wont_fix">Won't fix</option>
          <option value="duplicate">Duplicates</option>
          <option value="all">All</option>
        </select>
      </label>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {list.data?.reports.length === 0 && <p className="muted">None.</p>}
      <ul className="offer-list">
        {list.data?.reports.map((r) => (
          <MakerReport key={r.id} r={r} link={issueLink(repoUrl, r)} onUpdate={update} />
        ))}
      </ul>
    </section>
  );
}

function MakerReport({
  r,
  link,
  onUpdate,
}: {
  r: BugReport;
  link: string | null;
  onUpdate: (id: string, change: { status?: BugReport["status"]; maker_note?: string }) => Promise<void>;
}) {
  const [note, setNote] = useState(r.maker_note ?? "");
  return (
    <li>
      <span className="badge">{STATUS[r.status]}</span> <b>{r.title}</b>{" "}
      <span className="faint small">
        {r.reporter} · {r.version ?? "?"} on {r.platform ?? "?"} · {formatDate(r.created_at)}
        {r.me_too > 0 && ` · ${r.me_too} more with it`}
      </span>
      <p className="pre small">{r.body}</p>
      {r.log && (
        <details>
          <summary className="small">Last output the player sent</summary>
          <pre className="code-block" tabIndex={0}>
            {r.log}
          </pre>
        </details>
      )}
      <div className="form">
        <label>
          Note for the player
          <input value={note} maxLength={2000} onChange={(e) => setNote(e.target.value)} />
        </label>
        <div className="row-actions">
          {(["fixed", "wont_fix", "duplicate", "open"] as const)
            .filter((s) => s !== r.status)
            .map((s) => (
              <button key={s} type="button" className="btn small ghost" onClick={() => void onUpdate(r.id, { status: s, maker_note: note })}>
                {s === "open" ? "Open again" : `Mark ${STATUS[s].toLowerCase()}`}
              </button>
            ))}
          {link && (
            <a className="btn small ghost" href={link} target="_blank" rel="noreferrer">
              Open as a GitHub issue
            </a>
          )}
        </div>
      </div>
    </li>
  );
}

/** How each recent release landed with players. */
export function ReleaseFeedbackPanel({ slug }: { slug: string }) {
  const feedback = useLoad(() => api.bugReports.feedback(slug), [slug]);
  const rows: ReleaseFeedback[] = feedback.data ?? [];
  if (rows.length === 0) return null;
  const phrases = (list: Array<[string, number]>) =>
    list.length ? list.map(([p, n]) => `${p} (${n})`).join(", ") : "nothing repeated yet";
  return (
    <section className="panel panel-pad" aria-labelledby="feedback-title">
      <h2 id="feedback-title">How each release landed</h2>
      <p className="muted small">
        From votes cast on each version, reviews written while it was the newest, and bug reports. Phrases are counted,
        not guessed: each is used in at least two reviews.
      </p>
      <ul className="offer-list">
        {rows.map((f) => (
          <li key={f.version}>
            <b>{f.version}</b> <span className="faint small">approved {formatDate(f.approved_at)}</span>
            <div className="small">
              {f.not_slop_percent !== null ? `${f.not_slop_percent}% Slaps from ${f.votes} votes` : "No votes on it yet"} ·{" "}
              {f.reviews_up} Slaps and {f.reviews_down} Slop reviews · {f.bug_reports} bug report{f.bug_reports === 1 ? "" : "s"}
            </div>
            <div className="small">
              <InlineIcon name="up" /> Slaps say: {phrases(f.praised)}</div>
            <div className="small">
              <InlineIcon name="down" /> Slop says: {phrases(f.criticised)}</div>
          </li>
        ))}
      </ul>
    </section>
  );
}
