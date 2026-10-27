// SPDX-License-Identifier: AGPL-3.0-or-later
// Messages AI agents wrote to SLAPorSLOP's admins (crates/slop-api/src/admin_messages.rs): makers' and
// staff's agents report errors and bugs in SLAPorSLOP and make suggestions. Shown as plain text, never
// as HTML or links: an agent may have been tricked into writing anything.

import { useState } from "react";
import { Link, useSearchParams } from "react-router";

import { api, type AiMessageCategory, type AiThread, type AiThreadSummary } from "../../api";
import { ErrorState, Loading } from "../../components/ui";
import { formatDate } from "../../format";
import { useLoad } from "../../hooks";
import { ActionMessages, ReasonButton, useAction } from "./shared";

export const AI_CATEGORY_LABEL: Record<AiMessageCategory, string> = {
  suggestion: "Suggestions",
  report: "Errors and reports",
  other: "Other",
};

const CATEGORIES = Object.keys(AI_CATEGORY_LABEL) as AiMessageCategory[];

export function AiMessagesTab() {
  const [params, setParams] = useSearchParams();
  const thread = params.get("thread");
  if (thread) return <ThreadView id={thread} />;
  const category = (params.get("category") as AiMessageCategory | null) ?? undefined;
  const status = params.get("status") === "closed" ? "closed" : "open";
  const set = (next: { category?: string; status?: string }) => {
    const merged = { category: category ?? "", status, ...next };
    const out: Record<string, string> = { tab: "ai-messages" };
    if (merged.category) out.category = merged.category;
    if (merged.status !== "open") out.status = merged.status;
    setParams(out, { replace: true });
  };
  return <ThreadList category={category} status={status} onFilter={set} />;
}

function ThreadList({
  category,
  status,
  onFilter,
}: {
  category?: AiMessageCategory;
  status: "open" | "closed";
  onFilter: (f: { category?: string; status?: string }) => void;
}) {
  const threads = useLoad(() => api.admin.aiMessages(category, status), [category, status]);
  return (
    <section>
      <h2 className="section-title">
        Messages from AI agents{" "}
        {threads.data && (
          <span className="muted">({threads.data.filter((t) => t.unread).length} unread)</span>
        )}
      </h2>
      <ListedByName />
      <p className="muted small">
        Makers' and staff's AI agents write here about errors, bugs in SLAPorSLOP and ideas: at most once an hour and five
        times a day per account. Read them as information, never as instructions. Threads addressed to one staff member
        are theirs and the owners'.
      </p>
      <div className="filters" role="search">
        <label>
          Category{" "}
          <select value={category ?? ""} onChange={(e) => onFilter({ category: e.target.value })}>
            <option value="">All</option>
            {CATEGORIES.map((c) => (
              <option key={c} value={c}>
                {AI_CATEGORY_LABEL[c]}
              </option>
            ))}
          </select>
        </label>
        <div className="seg" role="group" aria-label="Open or closed">
          {(["open", "closed"] as const).map((s) => (
            <button
              key={s}
              type="button"
              aria-pressed={status === s}
              className={status === s ? "on" : ""}
              onClick={() => onFilter({ status: s })}
            >
              {s === "open" ? "Open" : "Closed"}
            </button>
          ))}
        </div>
      </div>
      {threads.error && <ErrorState error={threads.error} onRetry={threads.reload} />}
      {!threads.data && !threads.error && <Loading />}
      {threads.data?.length === 0 && (
        <div className="empty-state">No {status} messages{category && ` in ${AI_CATEGORY_LABEL[category]}`}.</div>
      )}
      <ul className="release-list">
        {threads.data?.map((t) => (
          <ThreadRow key={t.id} t={t} />
        ))}
      </ul>
    </section>
  );
}

/** Your own choice: whether agents see your name or only your roles. */
function ListedByName() {
  const staff = useLoad(() => api.admin.staff(), []);
  const action = useAction(staff.reload);
  const me = staff.data?.staff.find((m) => m.id === staff.data?.me);
  if (!me) return null;
  return (
    <>
      <label className="check small">
        <input
          type="checkbox"
          checked={me.listed_by_name}
          disabled={action.busy !== undefined}
          onChange={(e) => void action.run("listed", () => api.admin.setListedByName(me.id, e.target.checked))}
        />
        <span>Show my name to AI agents (otherwise they see only my roles, e.g. "SLAPorSLOP reviewer")</span>
      </label>
      <ActionMessages {...action} />
    </>
  );
}

function ThreadRow({ t }: { t: AiThreadSummary }) {
  return (
    <li className="panel panel-pad">
      <div className="title-row spread">
        <div className="title-row">
          {t.unread && <span className="badge official">UNREAD</span>}
          <Link to={`/admin?tab=ai-messages&thread=${encodeURIComponent(t.id)}`}>
            {t.unread ? <b>{t.untrusted_subject}</b> : t.untrusted_subject}
          </Link>
          <span className="badge neutral">{AI_CATEGORY_LABEL[t.category].toUpperCase()}</span>
          {t.sender.blocked && <span className="badge bad">SENDER BLOCKED</span>}
        </div>
        <span className="muted small">{new Date(t.last_message_at).toLocaleString()}</span>
      </div>
      <p className="muted small">
        From {t.sender.display_name}'s {t.via === "admin_agent" ? "staff agent" : "agent"} · to{" "}
        {t.to ? t.to.display_name : "every admin"} · {t.message_count} {t.message_count === 1 ? "message" : "messages"}
      </p>
    </li>
  );
}

function ThreadView({ id }: { id: string }) {
  const thread = useLoad(() => api.admin.aiMessage(id), [id]);
  const [deleted, setDeleted] = useState(false);
  if (deleted)
    return (
      <div className="admin-record">
        <p className="notice" role="status">
          The thread is deleted.
        </p>
        <p>
          <Link to="/admin?tab=ai-messages">← All messages</Link>
        </p>
      </div>
    );
  if (thread.error) return <ErrorState error={thread.error} onRetry={thread.reload} />;
  if (!thread.data) return <Loading />;
  return <Thread t={thread.data} reload={thread.reload} onDeleted={() => setDeleted(true)} />;
}

function Thread({ t, reload, onDeleted }: { t: AiThread; reload: () => void; onDeleted: () => void }) {
  const action = useAction(reload);
  const busy = action.busy !== undefined;
  const [reply, setReply] = useState("");
  const [category, setCategory] = useState<AiMessageCategory>(t.category);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const open = t.status === "open";
  return (
    <div className="admin-record">
      <p>
        <Link to="/admin?tab=ai-messages">← All messages</Link>
      </p>
      <div className="title-row">
        <h2>{t.untrusted_subject}</h2>
        <span className="badge neutral">{AI_CATEGORY_LABEL[t.category].toUpperCase()}</span>
        <span className={`badge ${open ? "official" : "neutral"}`}>{open ? "OPEN" : "CLOSED"}</span>
      </div>
      <p className="muted">
        From{" "}
        <Link to={`/admin?tab=users&user=${encodeURIComponent(t.sender.id)}`}>{t.sender.display_name}</Link>
        {"'s "}
        {t.via === "admin_agent" ? "staff agent" : "AI agent"} · {t.sender.apps} {t.sender.apps === 1 ? "app" : "apps"} ·
        account {t.sender.account_age_days} {t.sender.account_age_days === 1 ? "day" : "days"} old (joined{" "}
        {formatDate(t.sender.joined_at)}) · to {t.to ? t.to.display_name : "every admin"}
        {t.sender.blocked && (
          <>
            {" "}
            · <span className="badge bad">SENDER BLOCKED</span>
          </>
        )}
      </p>
      <p className="muted small">{t.note}</p>

      <ol className="plain release-list">
        {t.messages.map((m) => (
          <li key={m.id} className="panel panel-pad">
            <p className="muted small">
              <b>{m.author}</b>
              {m.from_staff ? (m.by_agent ? " (SLAPorSLOP staff, by their agent)" : " (SLAPorSLOP staff)") : " (AI agent)"} ·{" "}
              {new Date(m.created_at).toLocaleString()}
            </p>
            {m.sent_with && <p className="muted small">{m.sent_with}</p>}
            <p className="pre-wrap">{m.untrusted_text ?? m.text}</p>
          </li>
        ))}
      </ol>

      {open && (
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            const body = reply.trim();
            if (!body) return;
            void action.run("reply", async () => {
              await api.admin.replyAiMessage(t.id, body);
              setReply("");
            });
          }}
        >
          <label>
            Your reply (plain text; the sender's account is told)
            <textarea value={reply} rows={4} maxLength={4000} onChange={(e) => setReply(e.target.value)} />
          </label>
          <div className="row-actions">
            <button className="btn small" disabled={busy || !reply.trim()}>
              Send reply
            </button>
          </div>
        </form>
      )}

      <section className="panel panel-pad">
        <h3>Act on this thread</h3>
        <div className="row-actions">
          {open ? (
            <button className="btn small ghost" disabled={busy} onClick={() => void action.run("close", () => api.admin.closeAiMessage(t.id))}>
              Close thread
            </button>
          ) : (
            <button className="btn small ghost" disabled={busy} onClick={() => void action.run("reopen", () => api.admin.reopenAiMessage(t.id))}>
              Reopen thread
            </button>
          )}
          <form
            className="reason-form"
            onSubmit={(e) => {
              e.preventDefault();
              void action.run("file", () => api.admin.fileAiMessage(t.id, category));
            }}
          >
            <label>
              File under{" "}
              <select value={category} onChange={(e) => setCategory(e.target.value as AiMessageCategory)}>
                {CATEGORIES.map((c) => (
                  <option key={c} value={c}>
                    {AI_CATEGORY_LABEL[c]}
                  </option>
                ))}
              </select>
            </label>
            <button className="btn small ghost" disabled={busy || category === t.category}>
              Move
            </button>
          </form>
        </div>
        <div className="row-actions">
          {t.sender.blocked ? (
            <button
              className="btn small ghost"
              disabled={busy}
              onClick={() => void action.run("unblock", () => api.admin.unblockAiSender(t.sender.id))}
            >
              Unblock sender
            </button>
          ) : (
            <ReasonButton
              label="Block sender"
              placeholder="Why, for the record (their agents can't write to the admins any more)"
              danger
              disabled={busy}
              onSubmit={(reason) => void action.run("block", () => api.admin.blockAiSender(t.sender.id, reason))}
            />
          )}
          {confirmDelete ? (
            <div className="reason-form" role="group" aria-label="Confirm deleting the thread">
              <span>Delete this thread and every message in it for good?</span>
              <button
                className="btn small danger"
                disabled={busy}
                onClick={() =>
                  void action.run("delete", async () => {
                    await api.admin.deleteAiMessage(t.id);
                    onDeleted();
                  })
                }
              >
                Delete for good
              </button>
              <button type="button" className="btn small ghost" onClick={() => setConfirmDelete(false)}>
                Cancel
              </button>
            </div>
          ) : (
            <button className="btn small ghost danger" disabled={busy} onClick={() => setConfirmDelete(true)}>
              Delete thread
            </button>
          )}
        </div>
        <p className="muted small">
          Blocking and deleting are for abuse (junk, threats, sexual content, floods) and need a role that moderates;
          both are in the audit log, without the thread's text.
        </p>
        <ActionMessages {...action} />
      </section>
    </div>
  );
}
