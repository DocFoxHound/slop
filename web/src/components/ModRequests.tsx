// A running game asking for mods (Slop Link, DESIGN §9): a server or lobby needs them. The
// player sees each one and says yes or no; on yes the Slop app fetches each from Slop, checks it
// against Slop's signature, adds it to the game, and tells the game. Servers only name mods.

import { useEffect, useRef, useState } from "react";

import { api, type AppUpdate } from "../api";
import { desktop, isDesktop, type ModRequest } from "../platform";

type Known = NonNullable<AppUpdate["mods"]>[number];

/** Mounted once in the layout; shows requests one at a time. */
export function ModRequests() {
  const [queue, setQueue] = useState<ModRequest[]>([]);
  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    void desktop.onModRequest((r) => setQueue((q) => [...q, r])).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);
  const current = queue[0];
  if (!current) return null;
  return <RequestDialog key={current.id} request={current} onDone={() => setQueue((q) => q.slice(1))} />;
}

function RequestDialog({ request, onDone }: { request: ModRequest; onDone: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [info, setInfo] = useState<{ title: string; mods: Known[]; installed: string[] }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => {
    ref.current?.showModal();
    void Promise.all([
      api.libraryUpdates([request.game]).catch(() => [] as AppUpdate[]),
      desktop.installed(),
      desktop.installedMods(request.game),
    ]).then(([updates, apps, added]) =>
      setInfo({
        title: apps.find((a) => a.slug === request.game)?.title ?? request.game,
        mods: updates[0]?.mods ?? [],
        installed: added.map((m) => m.slug),
      }),
    );
  }, [request.game]);

  const answer = async (accept: boolean) => {
    setBusy(true);
    setError(undefined);
    try {
      await desktop.answerModRequest(request.id, accept);
      ref.current?.close();
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setBusy(false);
    }
  };

  const rows = request.mods.map((w) => ({ wanted: w, known: info?.mods.find((m) => m.slug === w.slug) }));
  const addable = rows.filter((r) => r.known && !info?.installed.includes(r.wanted.slug));
  const anyKnown = rows.some((r) => r.known);
  const online = rows.some((r) => r.known?.online);
  const title = info?.title ?? "A game";
  return (
    <dialog ref={ref} className="dialog" aria-label={`${title} wants to add mods`} onCancel={(e) => e.preventDefault()}>
      <h2>{title} wants to add mods</h2>
      <p className="muted small">
        {request.from ? (
          <>
            For <b>{request.from}</b> (the game's words).{" "}
          </>
        ) : null}
        Only mods published and approved on Slop can be added. Each comes from Slop and is checked against Slop's
        signature, never from the server.
      </p>
      {!info ? (
        <p className="muted">Checking…</p>
      ) : (
        <ul className="plain mod-request-list">
          {rows.map(({ wanted, known }) => (
            <li key={wanted.slug}>
              {known ? (
                <>
                  <b>{known.title}</b> <span className="muted small">by {known.author}</span>
                  {wanted.version && <span className="muted small"> · asks for {wanted.version}</span>}
                  {info.installed.includes(wanted.slug) && <span className="badge official">ADDED</span>}
                  {known.code && <span className="badge neutral">CONTAINS CODE</span>}
                  {known.online && <span className="badge warn">GOES ONLINE</span>}
                  {known.note && <div className="muted small">{known.note}</div>}
                </>
              ) : (
                <>
                  <code>{wanted.slug}</code> <span className="badge bad">NOT ON SLOP</span>
                  <div className="muted small">
                    Not an approved mod for {title}, so it can't be added. Mods must be published on Slop and follow its
                    guidelines.
                  </div>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      {online && (
        <p className="small">
          A mod here goes online, on servers Slop can't check. We suggest playing {title} in <b>sandbox mode</b>.
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <div className="actions">
        {error ? (
          <button
            className="btn ghost"
            onClick={() => {
              ref.current?.close();
              onDone();
            }}
          >
            Close
          </button>
        ) : (
          <>
            <button className="btn ghost" disabled={busy} onClick={() => void answer(false)}>
              No
            </button>
            <button className="btn slime" disabled={busy || !info || !anyKnown} onClick={() => void answer(true)}>
              {busy
                ? "Adding…"
                : addable.length === 0
                  ? "OK"
                  : `Add ${addable.length === 1 ? "1 mod" : `${addable.length} mods`}`}
            </button>
          </>
        )}
      </div>
    </dialog>
  );
}
