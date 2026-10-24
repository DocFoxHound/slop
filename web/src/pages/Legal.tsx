// SPDX-License-Identifier: AGPL-3.0-or-later
// Slop's legal documents (crates/slop-api/legal/): every version kept, changes announced 15 days
// ahead, and each user's acceptance recorded (crates/slop-api/src/legal.rs).

import { useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { api, type LegalKind } from "../api";
import { ErrorState, Loading } from "../components/ui";
import { AcceptDocument } from "../components/Terms";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { renderDocument } from "../markdown";
import { useSession } from "../session";

export function LegalIndex() {
  useDocumentTitle("Terms and policies");
  const index = useLoad(() => api.legal.index(), []);
  if (index.error) return <ErrorState error={index.error} onRetry={index.reload} />;
  if (!index.data) return <Loading />;
  return (
    <div className="page narrow">
      <h1 className="page-title">Terms and policies</h1>
      <p className="muted">
        Changes are announced on Slop at least 15 days before they take effect. Every earlier version stays readable here.
      </p>
      <ul className="legal-index panel">
        {index.data.map((d) => (
          <li key={d.kind}>
            <Link to={`/legal/${d.kind}`}>
              <b>{d.title}</b>
            </Link>
            <div className="muted small">
              {d.current ? `Version ${d.current.version}, in effect since ${formatDate(d.current.effective_at)}` : "Not published yet"}
              {d.accepted ? " · you accept this one" : " · for reading"}
            </div>
            {d.upcoming && (
              <div className="small warn">
                Changes on {formatDate(d.upcoming.effective_at)}: {d.upcoming.summary}
              </div>
            )}
          </li>
        ))}
      </ul>
      <p className="small">
        <Link to="/transparency">Transparency report</Link>: what authorities asked Slop for, and how Slop handled reports,
        each year.
      </p>
    </div>
  );
}

export function LegalPage() {
  const { kind = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const version = params.get("version") ?? undefined;
  const { me } = useSession();
  const doc = useLoad(() => api.legal.document(kind, version), [kind, version]);
  const notices = useLoad(() => (me ? api.legal.notices() : Promise.resolve([])), [me?.id]);
  // Where they stand with it, even when nothing asked them yet (a first Publisher Agreement).
  const mine = useLoad(
    () => (me && doc.data?.accepted ? api.legal.mine(doc.data.kind) : Promise.resolve(undefined)),
    [me?.id, doc.data?.kind, doc.data?.accepted],
  );
  const [accepting, setAccepting] = useState<string>();
  useDocumentTitle(doc.data?.title);

  if (doc.error) return <ErrorState error={doc.error} onRetry={doc.reload} />;
  if (!doc.data) return <Loading />;
  const d = doc.data;
  const toAccept = notices.data?.filter((n) => n.kind === d.kind && n.action === "accept") ?? [];
  // The version in effect, not yet accepted, and not already offered by a notice.
  const current = mine.data?.kind === d.kind ? mine.data : undefined;
  const notYet =
    current?.version && !current.accepted && !toAccept.some((n) => n.version === current.version) ? current.version : undefined;
  const returnTo = `/legal/${d.kind}`;

  return (
    <div className="page narrow legal-page">
      <p className="small">
        <Link to="/legal">← Terms and policies</Link>
      </p>
      <h1 className="page-title">{d.title}</h1>
      <p className="muted small">
        Version {d.version} ·{" "}
        {d.in_effect ? `in effect since ${formatDate(d.effective_at)}` : `takes effect on ${formatDate(d.effective_at)}`}
        {d.versions.length > 1 && <> · {d.summary}</>}
      </p>
      {toAccept.map((n) => (
        <div className="notice" key={n.version}>
          {n.in_effect ? (
            <>The {n.title} changed on {formatDate(n.effective_at)}. </>
          ) : (
            <>The {n.title} changes on {formatDate(n.effective_at)}. </>
          )}
          {n.summary}{" "}
          <button type="button" className="btn small slime" onClick={() => setAccepting(n.version)}>
            Review and accept version {n.version}
          </button>
        </div>
      ))}
      {d.accepted && me === null && (
        <p className="notice">
          <Link to={`/signin?return_to=${encodeURIComponent(returnTo)}`}>Sign in to accept the {d.title}</Link>.
        </p>
      )}
      {notYet && (
        <div className="notice">
          You haven't accepted the {d.title} yet.{" "}
          <button type="button" className="btn small slime" onClick={() => setAccepting(notYet)}>
            Review and accept version {notYet}
          </button>
        </div>
      )}
      {current?.accepted && current.version && toAccept.length === 0 && (
        <p className="small muted">You've accepted version {current.version}, the one in effect.</p>
      )}
      {accepting && (
        <AcceptDocument
          kind={d.kind as LegalKind}
          version={accepting}
          onAccepted={() => {
            setAccepting(undefined);
            notices.reload();
            mine.reload();
          }}
          onCancel={() => setAccepting(undefined)}
        />
      )}
      <article className="panel panel-pad doc" dangerouslySetInnerHTML={{ __html: renderDocument(d.body_md) }} />
      {d.versions.length > 1 && (
        <>
          <h2 className="section-title">All versions</h2>
          <ul className="legal-versions">
            {d.versions.map((v) => (
              <li key={v.version}>
                <button
                  type="button"
                  className="link-btn"
                  aria-current={v.version === d.version ? "page" : undefined}
                  onClick={() => setParams({ version: v.version })}
                >
                  {v.version}
                </button>{" "}
                <span className="muted small">
                  {new Date(v.effective_at) > new Date() ? "takes effect" : "in effect from"} {formatDate(v.effective_at)}: {v.summary}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      <p className="muted small">
        SHA-256 of this text: <code>{d.sha256.slice(0, 16)}…</code>
      </p>
    </div>
  );
}
