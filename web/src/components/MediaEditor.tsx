// SPDX-License-Identifier: AGPL-3.0-or-later
// The maker's pictures and videos, on the store-page editor. They live in the maker's repository
// (.slop/media.json, crates/slop-api/src/media.rs); every new set waits for review before the
// store shows it. The maker puts the gallery in order here, which isn't reviewed.

import { useState } from "react";
import { Link } from "react-router";

import { api, ApiError, mediaKey, type AppDetail, type MediaItem, type MediaOverview, type MediaSet } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { ErrorState, ExternalLink, Loading } from "./ui";

const EXAMPLE = `{
  "thumbnail": "thumbnail.png",
  "gallery": [
    { "image": "screenshots/title.png", "caption": "The title screen" },
    { "image": "screenshots/level-1.png" },
    { "youtube": "https://www.youtube.com/watch?v=…", "poster": "screenshots/trailer.png" }
  ]
}`;

export function MediaEditor({ app }: { app: AppDetail }) {
  const loaded = useLoad(() => api.media(app.slug), [app.slug]);
  const [overview, setOverview] = useState<MediaOverview>();
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();
  const m = overview ?? loaded.data;

  const run = async (what: string, action: () => Promise<MediaOverview>) => {
    setBusy(what);
    setError(undefined);
    try {
      setOverview(await action());
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(undefined);
    }
  };

  if (loaded.error) return <ErrorState error={loaded.error} onRetry={loaded.reload} />;
  if (!m) return <Loading />;

  // The order applies to whatever is newest: the set waiting for review, else the shown one.
  const working = m.pending ?? m.live;
  const move = (i: number, by: number) => {
    if (!working) return;
    const keys = working.gallery.map(mediaKey);
    const [key] = keys.splice(i, 1);
    keys.splice(i + by, 0, key);
    void run("order", () => api.setMediaOrder(app.slug, keys));
  };

  return (
    <section className="panel panel-pad form media-editor">
      <h2>Pictures &amp; videos</h2>
      <p className="muted small">
        Your thumbnail (the store tile), screenshots and videos come from your repository, not from uploads. Put the
        pictures in a <code>.slop</code> folder and list them in <code>{m.manifest_path}</code>:
      </p>
      <pre className="code-block">{EXAMPLE}</pre>
      <p className="muted small">
        Paths are inside <code>.slop/</code>. PNG, JPEG, WebP or GIF, up to 8 MB each; the thumbnail is best at 920×430
        (at least 460×215), screenshots at least 320×180. Up to 20 screenshots and videos. Videos are YouTube links; the{" "}
        <code>poster</code> is optional. See <Link to="/publish/guide">How to publish</Link>.
      </p>
      <p className="muted small">
        Every new set of pictures is reviewed before it shows (and before the app first goes in the store). Slop also
        reads them again with each release. Changing the order below isn't reviewed.
      </p>

      {!m.supported && <p className="notice">Pictures from the repository only work for GitHub repositories so far.</p>}

      <div className="row-actions">
        <button className="btn slime" disabled={Boolean(busy) || !m.supported} onClick={() => void run("check", () => api.checkMedia(app.slug))}>
          {busy === "check" ? "Reading your repository…" : "Check my repository now"}
        </button>
        {m.checked_at && <span className="muted small">Last read {formatDate(m.checked_at)}</span>}
      </div>
      {m.result === "unchanged" && <p className="notice">Nothing new: these pictures are already shown or waiting for review.</p>}
      {m.result === "none" && (
        <p className="notice">
          There's no <code>{m.manifest_path}</code> on your default branch yet.
        </p>
      )}
      {m.result === "pending" && <p className="notice">Found new pictures. They're waiting for review.</p>}
      {m.problem && (
        <p className="form-error" role="alert">
          {m.problem}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      {m.rejected && (
        <div className="notice">
          <b>These pictures were turned down:</b> {m.rejected.status_reason} Fix them in your repository and check again.
        </div>
      )}
      {m.pending && <SetView set={m.pending} label="Waiting for review" badge="soon" onMove={move} busy={Boolean(busy)} />}
      {m.live ? (
        <SetView set={m.live} label="In the store" badge="official" onMove={m.pending ? undefined : move} busy={Boolean(busy)} />
      ) : (
        !m.pending && <p className="muted small">No pictures in the store yet.</p>
      )}
    </section>
  );
}

function SetView({
  set,
  label,
  badge,
  onMove,
  busy,
}: {
  set: MediaSet;
  label: string;
  badge: string;
  onMove?: (i: number, by: number) => void;
  busy: boolean;
}) {
  return (
    <div className="media-set">
      <div className="title-row">
        <span className={`badge ${badge}`}>{label.toUpperCase()}</span>
        <span className="muted small">
          from commit <code>{set.commit_sha.slice(0, 7)}</code> · {formatDate(set.created_at)}
        </span>
      </div>
      {set.thumbnail ? (
        <div className="thumb-preview">
          <img src={set.thumbnail.url} alt="Thumbnail" className="thumb-img" />
        </div>
      ) : (
        <p className="muted small">No thumbnail; the store uses a colour and your app's initial.</p>
      )}
      {set.gallery.length > 0 && (
        <>
          {onMove && <p className="muted small">Put them in the order players see them:</p>}
          <ol className="gallery-edit">
            {set.gallery.map((item, i) => (
              <li key={mediaKey(item)}>
                <ItemThumb item={item} />
                <div className="small">
                  <b>{item.kind === "image" ? item.path : "YouTube video"}</b>
                  {item.kind === "video" && (
                    <>
                      {" "}
                      <ExternalLink href={`https://www.youtube.com/watch?v=${item.youtube_id}`}>{item.youtube_id}</ExternalLink>
                    </>
                  )}
                  {item.caption && <div className="muted">{item.caption}</div>}
                </div>
                {onMove && (
                  <div className="row-actions">
                    <button
                      type="button"
                      className="btn small"
                      disabled={busy || i === 0}
                      onClick={() => onMove(i, -1)}
                      aria-label={`Move item ${i + 1} earlier`}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      className="btn small"
                      disabled={busy || i === set.gallery.length - 1}
                      onClick={() => onMove(i, 1)}
                      aria-label={`Move item ${i + 1} later`}
                    >
                      ↓
                    </button>
                  </div>
                )}
              </li>
            ))}
          </ol>
        </>
      )}
    </div>
  );
}

export function ItemThumb({ item }: { item: MediaItem }) {
  const src = item.kind === "image" ? item.url : item.poster?.url;
  return (
    <div className="ge-thumb">
      {src ? <img src={src} alt="" loading="lazy" /> : <span className="play-card" />}
      {item.kind === "video" && (
        <span className="play-mark small" aria-hidden="true">
          ▶
        </span>
      )}
    </div>
  );
}
