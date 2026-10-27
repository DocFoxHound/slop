// SPDX-License-Identifier: AGPL-3.0-or-later
// SLAPorSLOP's terms and policies for staff (crates/slop-api/src/legal.rs): every version, and
// publishing a new one from here. A new version is announced at once and takes effect 15 days
// later; until then it can be withdrawn. Publishing is sensitive (two people, once SLAPorSLOP requires
// two). Versions published here live in the database: copy the file text into
// crates/slop-api/legal/ so the repository keeps up.

import { useState } from "react";
import { Link } from "react-router";

import { api, type AdminLegalDocument } from "../../api";
import { ErrorState, Loading } from "../../components/ui";
import { formatDate } from "../../format";
import { useLoad } from "../../hooks";
import { renderDocument } from "../../markdown";
import { ActionMessages, ReasonButton, useAction } from "./shared";

export function LegalDocsTab() {
  const docs = useLoad(() => api.admin.legalDocuments(), []);
  if (docs.error) return <ErrorState error={docs.error} onRetry={docs.reload} />;
  if (!docs.data) return <Loading />;
  return (
    <>
      <p className="muted">
        A new version is announced to everyone straight away and takes effect 15 days later; people asked to accept it
        (Terms, Publisher Agreement, People Terms) are asked then. Until it takes effect you can withdraw it.
      </p>
      {docs.data.map((d) => (
        <LegalDoc key={`${d.kind}-${d.latest_version}`} doc={d} onDone={docs.reload} />
      ))}
    </>
  );
}

function LegalDoc({ doc, onDone }: { doc: AdminLegalDocument; onDone: () => void }) {
  const [editing, setEditing] = useState(false);
  const [body, setBody] = useState(doc.latest_body_md);
  const [summary, setSummary] = useState("");
  const [preview, setPreview] = useState(false);
  const action = useAction(() => {
    setEditing(false);
    setSummary("");
    onDone();
  });
  const now = Date.now();
  const upcoming = doc.versions.find((v) => new Date(v.effective_at).getTime() > now);
  const current = doc.versions.find((v) => new Date(v.effective_at).getTime() <= now);
  return (
    <section className="panel panel-pad">
      <div className="title-row spread">
        <h2>{doc.title}</h2>
        <Link to={`/legal/${doc.kind}`} className="small">
          Public page
        </Link>
      </div>
      <p className="muted small">
        {doc.accepted ? "People accept it." : "People read it; changes are announced."} In effect:{" "}
        {current ? (
          <>
            {current.version} since {formatDate(current.effective_at)}
          </>
        ) : (
          "none"
        )}
        .
      </p>
      {upcoming && (
        <div className="notice" role="status">
          <b>Announced:</b> {upcoming.version}, in effect {formatDate(upcoming.effective_at)}: {upcoming.summary}
          {upcoming.published_by ? (
            <>
              {" "}
              <span className="muted small">(published from here by {upcoming.published_by})</span>
              <div className="row-actions">
                <ReasonButton
                  label="Withdraw it"
                  placeholder="Why, for the record"
                  danger
                  disabled={action.busy !== undefined}
                  onSubmit={(reason) =>
                    void action.run("withdraw", () => api.admin.withdrawLegal(doc.kind, upcoming.version, reason))
                  }
                />
              </div>
            </>
          ) : (
            <span className="muted small"> (from the file in the repository: change the file to withdraw it)</span>
          )}
        </div>
      )}
      {doc.file_text && (
        <details>
          <summary className="small">The newest version as a file, for crates/slop-api/legal/{doc.kind}.md</summary>
          <textarea readOnly rows={8} value={doc.file_text} aria-label={`${doc.title} as a file`} />
        </details>
      )}
      {!upcoming &&
        (editing ? (
          <form
            className="form"
            onSubmit={(e) => {
              e.preventDefault();
              void action.run("publish", () => api.admin.publishLegal(doc.kind, summary.trim(), body));
            }}
          >
            <div className="seg" role="tablist" aria-label="Edit or preview">
              <button type="button" role="tab" aria-selected={!preview} className={preview ? "" : "on"} onClick={() => setPreview(false)}>
                Edit
              </button>
              <button type="button" role="tab" aria-selected={preview} className={preview ? "on" : ""} onClick={() => setPreview(true)}>
                Preview
              </button>
            </div>
            {preview ? (
              <article className="doc" dangerouslySetInnerHTML={{ __html: renderDocument(body) }} />
            ) : (
              <label>
                The whole text (Markdown)
                <textarea value={body} rows={18} required onChange={(e) => setBody(e.target.value)} />
              </label>
            )}
            <label>
              What changed, in plain words (everyone is told this)
              <input value={summary} maxLength={300} required onChange={(e) => setSummary(e.target.value)} />
            </label>
            <div className="row-actions">
              <button
                className="btn small slime"
                disabled={action.busy !== undefined || !summary.trim() || body === doc.latest_body_md}
              >
                Publish with 15 days' notice
              </button>
              <button type="button" className="btn small ghost" onClick={() => setEditing(false)}>
                Cancel
              </button>
            </div>
          </form>
        ) : (
          <button className="btn small" onClick={() => setEditing(true)}>
            Write a new version
          </button>
        ))}
      <ActionMessages {...action} />
      <details>
        <summary className="small">All versions ({doc.versions.length})</summary>
        <ul className="plain small">
          {doc.versions.map((v) => (
            <li key={v.version}>
              <Link to={`/legal/${doc.kind}?version=${encodeURIComponent(v.version)}`}>{v.version}</Link>: {v.summary}{" "}
              <span className="muted">
                · published {formatDate(v.published_at)}, in effect {formatDate(v.effective_at)}
                {v.published_by ? ` · from the panel by ${v.published_by}` : " · from the repository"}
              </span>
            </li>
          ))}
        </ul>
      </details>
    </section>
  );
}
