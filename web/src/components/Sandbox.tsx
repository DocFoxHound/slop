// SPDX-License-Identifier: AGPL-3.0-or-later
// Sandbox mode in the SLAPorSLOP app (DESIGN §6): an app runs in a box where it can use only its own
// files, mods and save folder, plus the screen, sound and controllers. Players turn it on per
// app; SLAPorSLOP suggests it for apps that go online. It limits what a compromised app can reach;
// it isn't a guarantee, so it's never called "safe".

import { useEffect, useState } from "react";
import { Link } from "react-router";

import { api } from "../api";
import { useChosen, useLoad } from "../hooks";
import { desktop, isDesktop, type Launched, type SandboxStatus, type SaveSyncOutcome } from "../platform";

/** What the store knows about an app, for sandbox suggestions. */
export interface SandboxHint {
  /** It goes online on servers SLAPorSLOP can't check (or has networking code it doesn't declare). */
  suggested: boolean;
  /** Its store page says it doesn't go online, so blocking the internet is offered. */
  offline: boolean;
  /** Its mods that go online: once one is added, sandbox mode is suggested too (DESIGN §9). */
  onlineMods?: string[];
}

export function useSandboxStatus() {
  return useLoad(() => desktop.sandboxStatus(), []);
}

/** Whether the player has made a choice for this app (or chose "every app"). */
export function decided(status: SandboxStatus | undefined, slug: string): boolean {
  return Boolean(status && (status.settings.all || slug in status.settings.apps));
}

export function isOn(status: SandboxStatus | undefined, slug: string): boolean {
  if (!status) return false;
  const app = status.settings.apps[slug];
  return app ? app.on : status.settings.all;
}

/** Asked the first time a suggested app is played. */
export function SandboxPrompt({ title, onChoose }: { title: string; onChoose: (on: boolean) => void }) {
  return (
    <div className="notice sandbox-prompt" role="dialog" aria-label="Sandbox mode">
      <p>
        <b>{title}</b> plays online, on servers SLAPorSLOP can't check. In <b>sandbox mode</b> it can only use its own files and
        saves, not the rest of your computer. Your saves are the same in both modes. Some games don't work in it; you can
        change this any time.
      </p>
      <div className="actions">
        <button className="btn ghost" onClick={() => onChoose(false)}>
          Run normally
        </button>
        <button className="btn slime" onClick={() => onChoose(true)}>
          Use sandbox mode
        </button>
      </div>
    </div>
  );
}

/** The per-app switch, with "Block internet" for apps that say they don't go online. */
export function SandboxSwitch({
  slug,
  status,
  hint,
  onChanged,
}: {
  slug: string;
  status: SandboxStatus;
  hint?: SandboxHint;
  onChanged: () => void;
}) {
  const [error, setError] = useState<string>();
  const [on, chooseOn] = useChosen(status.available ? isOn(status, slug) : false);
  const blockedNow = status.available ? (status.settings.apps[slug]?.block_network ?? false) : false;
  const [blocked, chooseBlocked] = useChosen(blockedNow);
  if (!status.available) {
    return hint?.suggested ? <p className="muted small">Sandbox mode: {status.reason}.</p> : null;
  }
  const set = (nextOn: boolean, block: boolean) => {
    chooseOn(nextOn);
    chooseBlocked(block);
    void desktop
      .setSandbox(slug, nextOn, block)
      .then(onChanged)
      .catch((err: unknown) => {
        chooseOn(undefined);
        chooseBlocked(undefined);
        setError(err instanceof Error ? err.message : String(err));
      });
  };
  return (
    <div className="sandbox-switch small">
      <label className="check">
        <input type="checkbox" checked={on} onChange={(e) => set(e.target.checked, blocked)} /> Sandbox mode
        {hint?.suggested && !on && <span className="badge warn">SUGGESTED</span>}
      </label>
      {on && hint?.offline && (
        <label className="check">
          <input type="checkbox" checked={blocked} onChange={(e) => set(true, e.target.checked)} /> Block internet
        </label>
      )}
      {on && <div className="muted">Runs with only its own files and saves. Saves are shared with normal mode.</div>}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/** What SLAPorSLOP did before starting the app. */
export function LaunchNotices({ launched, slug, title }: { launched: Launched; slug: string; title: string }) {
  const unapproved = launched.unapproved_mods ?? [];
  return (
    <>
      <SaveSyncNotice slug={slug} title={title} result={launched.save_sync ?? null} />
      {unapproved.length > 0 && (
        <div className="notice small" role="status">
          SLAPorSLOP moved {unapproved.length === 1 ? "something" : `${unapproved.length} things`} that{" "}
          {unapproved.length === 1 ? "isn't a mod" : "aren't mods"} from SLAPorSLOP out of {title}'s mods folder (
          {unapproved.slice(0, 3).join(", ")}
          {unapproved.length > 3 && ", …"}). They're kept in SLAPorSLOP's quarantine folder. Only mods published on SLAPorSLOP can be
          used, offline or online: a mod has to be published and approved, and follow the{" "}
          <Link to="/publish/guide#online-play-and-mods">guidelines</Link>, so everyone you play with gets the same checked
          files. Made it yourself? <Link to={`/app/${slug}/branch?kind=addon`}>Publish it for {title}</Link>.
        </div>
      )}
      {launched.quarantined.length > 0 && (
        <div className="notice small" role="status">
          Before starting {title}, SLAPorSLOP moved {launched.quarantined.length === 1 ? "a program" : `${launched.quarantined.length} programs`}{" "}
          out of its save folder ({launched.quarantined.slice(0, 3).join(", ")}
          {launched.quarantined.length > 3 && ", …"}). Saves are for data only. They're kept in SLAPorSLOP's quarantine folder.
        </div>
      )}
      {launched.copy_skipped && (
        <div className="notice small" role="status">
          {title}'s saves are too big to copy before a sandboxed session, so there's no copy to go back to this time.
        </div>
      )}
    </>
  );
}

/** Shown when an app closes with an error right after starting in sandbox mode. */
export function useSandboxFailed(slug: string): [boolean, () => void] {
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void desktop
      .onSandboxFailed((e) => {
        if (e.slug === slug) setFailed(true);
      })
      .then((u) => (unlisten = u));
    return () => unlisten?.();
  }, [slug]);
  return [failed, () => setFailed(false)];
}

export function SandboxFailedNotice({
  slug,
  title,
  onRunNormally,
  onTurnOff,
  onClose,
}: {
  slug: string;
  title: string;
  onRunNormally: () => void;
  onTurnOff: () => void;
  onClose: () => void;
}) {
  const [reported, setReported] = useState(false);
  return (
    <div className="notice" role="alert">
      <p>
        <b>{title}</b> closed right after starting in sandbox mode. Some apps don't work in it, often because they save
        outside their save folder. Outside the sandbox it can reach your files like any program, so only run it
        normally if you trust it.
      </p>
      <div className="actions">
        <button className="btn small" onClick={onRunNormally}>
          Run normally this time
        </button>
        <button className="btn small" onClick={onTurnOff}>
          Turn sandbox mode off for it
        </button>
        <button
          className="btn small ghost"
          disabled={reported}
          onClick={() => void api.sandboxReport(slug, true).then(() => setReported(true), () => setReported(false))}
        >
          {reported ? "Thanks, reported" : "Tell SLAPorSLOP it doesn't work in sandbox mode"}
        </button>
        <button className="btn small ghost" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}

/** What save sync did, when there's something to say or choose. */
export function SaveSyncNotice({ slug, title, result }: { slug: string; title: string; result: SaveSyncOutcome | null }) {
  const [done, setDone] = useState<string>();
  if (!result || done === "hidden") return null;
  if (result.outcome === "fetched") {
    return (
      <div className="notice small" role="status">
        Got {title}'s saves from your other computer. This computer's previous saves are kept under Saves.
      </div>
    );
  }
  if (result.outcome === "too_big") {
    return (
      <div className="notice small" role="status">
        {title}'s saves are over 20 MB, so they stay on this computer only.
      </div>
    );
  }
  if (result.outcome !== "both_changed") return null;
  const keep = (which: "here" | "other") =>
    void desktop.resolveSaveConflict(slug, which).then(() => setDone(which === "here" ? "kept here" : "took other"));
  return (
    <div className="notice" role="alert">
      {done ? (
        <p>{done === "kept here" ? "This computer's saves are synced now." : "You have the other computer's saves now; this computer's are kept under Saves."}</p>
      ) : (
        <>
          <p>
            {title}'s saves changed on this computer and on another one since they last synced. Which do you want to keep?
            Nothing has been overwritten.
          </p>
          <div className="row-actions">
            <button type="button" className="btn small" onClick={() => keep("here")}>
              This computer's
            </button>
            <button type="button" className="btn small" onClick={() => keep("other")}>
              The other computer's
            </button>
            <button type="button" className="btn small ghost" onClick={() => setDone("hidden")}>
              Decide later
            </button>
          </div>
        </>
      )}
    </div>
  );
}

/** Save sync results after an app closes (in the SLAPorSLOP app). */
export function SaveSyncEvents() {
  const [last, setLast] = useState<{ slug: string; result: SaveSyncOutcome }>();
  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    void desktop.onSavesSynced((e) => setLast(e)).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);
  if (!last || !["both_changed", "too_big"].includes(last.result.outcome)) return null;
  return <SaveSyncNotice slug={last.slug} title={last.slug} result={last.result} />;
}
