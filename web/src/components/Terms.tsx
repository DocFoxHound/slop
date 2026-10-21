// SPDX-License-Identifier: AGPL-3.0-or-later
// Agreements that stand in front of an action (crates/slop-api/src/legal.rs): the Publisher
// Agreement before a first app, the People Terms before a creative profile or a hire. When the
// API answers "terms_required", the page shows the document, the user accepts it, and the action
// runs again; nothing they typed is lost. "passkey_required" works the same way
// (PasskeyRequired.tsx).

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router";

import { api, ApiError, type LegalDocument, type LegalKind } from "../api";
import { formatDate } from "../format";
import { renderDocument } from "../markdown";
import { PasskeyRequired } from "./PasskeyRequired";

interface Pending {
  kind: LegalKind;
  resolve: () => void;
  reject: (err: unknown) => void;
}

/**
 * `guard(fn)` runs `fn`; if it needs an agreement accepted first, `prompt` shows it, and `fn`
 * runs again once it's accepted. Cancelling rejects with the original error.
 */
export function useTermsGate(): { guard: <T>(fn: () => Promise<T>) => Promise<T>; prompt: ReactNode } {
  const [pending, setPending] = useState<Pending>();
  const [passkey, setPasskey] = useState<{ add: boolean; resolve: () => void; reject: () => void }>();
  const guard = useCallback(async <T,>(fn: () => Promise<T>): Promise<T> => {
    for (;;) {
      try {
        return await fn();
      } catch (err) {
        if (err instanceof ApiError && err.code === "passkey_required") {
          const add = err.field === "add";
          await new Promise<void>((resolve, reject) => setPasskey({ add, resolve, reject: () => reject(err) }));
          continue;
        }
        if (!(err instanceof ApiError) || err.code !== "terms_required" || !err.field) throw err;
        const kind = err.field as LegalKind;
        await new Promise<void>((resolve, reject) => setPending({ kind, resolve, reject: () => reject(err) }));
      }
    }
  }, []);
  if (passkey) {
    return {
      guard,
      prompt: (
        <PasskeyRequired
          add={passkey.add}
          onDone={() => {
            setPasskey(undefined);
            passkey.resolve();
          }}
          onCancel={() => {
            setPasskey(undefined);
            passkey.reject();
          }}
        />
      ),
    };
  }
  const prompt = pending ? (
    <AcceptDocument
      kind={pending.kind}
      onAccepted={() => {
        setPending(undefined);
        pending.resolve();
      }}
      onCancel={() => {
        setPending(undefined);
        pending.reject(undefined);
      }}
    />
  ) : null;
  return { guard, prompt };
}

/** A document to accept, in a dialog: what changed, the full text, and one checkbox. */
export function AcceptDocument({
  kind,
  version,
  onAccepted,
  onCancel,
}: {
  kind: LegalKind;
  /** A version announced for later; otherwise the one in effect. */
  version?: string;
  onAccepted: () => void;
  onCancel: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [doc, setDoc] = useState<LegalDocument>();
  const [checked, setChecked] = useState(false);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    dialog.current?.showModal();
    api.legal.document(kind, version).then(setDoc, (e: unknown) => setError(e instanceof ApiError ? e.message : String(e)));
  }, [kind, version]);

  const accept = async () => {
    if (!doc) return;
    setBusy(true);
    try {
      await api.legal.accept(kind, doc.version);
      onAccepted();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <dialog ref={dialog} className="dialog wide" aria-label={doc?.title ?? "Agreement"} onCancel={onCancel}>
      <div className="form">
        <h2>{doc ? `Before you continue: the ${doc.title}` : "Loading…"}</h2>
        {doc && (
          <>
            <p className="small muted">
              Version {doc.version} · {doc.in_effect ? `in effect since ${formatDate(doc.effective_at)}` : `takes effect on ${formatDate(doc.effective_at)}`}
              {doc.versions.length > 1 && <> · What changed: {doc.summary}</>}
            </p>
            <div
              className="legal-scroll doc"
              tabIndex={0}
              aria-label={`${doc.title}, full text`}
              dangerouslySetInnerHTML={{ __html: renderDocument(doc.body_md) }}
            />
            <p className="small">
              <Link to={`/legal/${kind}`} target="_blank">
                Open it on its own page
              </Link>
            </p>
            <label className="check">
              <input type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
              <span>I've read and accept the {doc.title}.</span>
            </label>
          </>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <div className="row-actions">
          <button type="button" className="btn slime" disabled={!doc || !checked || busy} onClick={() => void accept()}>
            {busy ? "Saving…" : "Accept and continue"}
          </button>
          <button type="button" className="btn ghost" onClick={onCancel}>
            Not now
          </button>
        </div>
      </div>
    </dialog>
  );
}
