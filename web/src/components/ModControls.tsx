// Adding mods to installed games (DESIGN §9). A mod is a .zip of files for its game to load;
// the Slop app checks it against Slop's signed manifest like any build, keeps it beside the game,
// and tells the game where its mods are when it starts (SLOP_MODS). Nothing in a mod is started
// by Slop.

import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router";

import { desktop, isDesktop, type InstalledMod } from "../platform";

/** On a mod's own page. */
export function ModControls({ slug, game }: { slug: string; game: { slug: string; title: string } }) {
  const [state, setState] = useState<{ gameInstalled: boolean; added: boolean }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const reload = useCallback(async () => {
    const apps = await desktop.installed();
    const gameInstalled = apps.some((a) => a.slug === game.slug);
    const mods = gameInstalled ? await desktop.installedMods(game.slug) : [];
    setState({ gameInstalled, added: mods.some((m) => m.slug === slug) });
  }, [game.slug, slug]);

  useEffect(() => {
    if (isDesktop) void reload();
  }, [reload]);

  if (!isDesktop) {
    return (
      <p className="muted">
        A mod for <Link to={`/app/${game.slug}`}>{game.title}</Link>. Add it to the game from your library in the Slop
        app, which checks it against Slop's signature first.
      </p>
    );
  }
  if (!state) return <div className="install-controls muted">Checking…</div>;

  const act = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="install-controls">
      {!state.gameInstalled ? (
        <p className="muted">
          A mod for <Link to={`/app/${game.slug}`}>{game.title}</Link>. Install the game first, then add its mods.
        </p>
      ) : state.added ? (
        <>
          <p>
            ✓ Added to <Link to={`/app/${game.slug}`}>{game.title}</Link>. The game loads it next time it starts.
          </p>
          <div className="row-actions">
            <button className="btn small" disabled={busy} onClick={() => void act(() => desktop.installMod(slug))}>
              Update
            </button>
            <button className="btn small ghost danger" disabled={busy} onClick={() => void act(() => desktop.uninstallMod(slug))}>
              Remove
            </button>
          </div>
        </>
      ) : (
        <>
          <button className="btn big slime" disabled={busy} onClick={() => void act(() => desktop.installMod(slug))}>
            {busy ? "Adding…" : `Add to ${game.title}`}
          </button>
          <p className="muted small">Checked against Slop's signature, then handed to the game when it starts.</p>
        </>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/** In the library, under an installed game: its mods to add or remove. */
export function GameMods({
  game,
  available,
  onChanged,
}: {
  game: string;
  available: Array<{ slug: string; title: string; author: string; note: string | null }>;
  onChanged?: () => void;
}) {
  const [added, setAdded] = useState<InstalledMod[]>();
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();

  const reload = useCallback(() => desktop.installedMods(game).then(setAdded), [game]);
  useEffect(() => void reload(), [reload]);

  if (!added) return null;
  // Added mods first (including any no longer listed), then the rest.
  const rows = [
    ...added.map((m) => ({ slug: m.slug, title: m.title, author: m.maker, note: null, mod: m })),
    ...available.filter((a) => !added.some((m) => m.slug === a.slug)).map((a) => ({ ...a, mod: undefined })),
  ];
  if (rows.length === 0) return null;

  const toggle = async (slug: string, add: boolean) => {
    setBusy(slug);
    setError(undefined);
    try {
      await (add ? desktop.installMod(slug) : desktop.uninstallMod(slug));
      await reload();
      onChanged?.();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(undefined);
    }
  };

  return (
    <div className="game-mods">
      <h4>Mods</h4>
      <ul className="plain">
        {rows.map((r) => (
          <li key={r.slug}>
            <label className="check">
              <input
                type="checkbox"
                checked={Boolean(r.mod)}
                disabled={busy !== undefined}
                onChange={(e) => void toggle(r.slug, e.target.checked)}
              />{" "}
              <Link to={`/app/${r.slug}`}>{r.title}</Link> <span className="muted small">by {r.author}</span>
            </label>
            {busy === r.slug && <span className="muted small"> {r.mod ? "Removing…" : "Adding…"}</span>}
            {r.mod?.revoked && (
              <div className="small">
                <span className="badge bad">PULLED</span> Slop pulled this mod ({r.mod.revoked}), so the game no longer gets
                it. Remove it.
              </div>
            )}
            {r.note && <div className="muted small">{r.note}</div>}
          </li>
        ))}
      </ul>
      <p className="muted small">Ticked mods are handed to the game when it starts. Each was checked and reviewed by Slop.</p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
