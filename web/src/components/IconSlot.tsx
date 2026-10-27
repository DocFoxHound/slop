// SPDX-License-Identifier: AGPL-3.0-or-later
// The Icon slot at the top of the store-page editor's pictures (MediaEditor): the app's icon at
// the sizes players see it, whether it's in the store, waiting for review, missing or refused,
// and how to add one. Like every picture it lives in the maker's repository (`"icon"` in
// .slop/media.json, crates/slop-api/src/media.rs), so "choosing" an icon means naming it there.
// The crop helper only makes a file for the maker to download and commit: SLAPorSLOP never stores it.

import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";

import type { AppDetail, MediaOverview, RepoPicture } from "../api";
import { isDesktop } from "../platform";
import { AppIcon } from "./AppIcon";

/** The line that names the icon in .slop/media.json. */
export const ICON_LINE = `"icon": "icon.png",`;
const ICON_MIN = 256;
const ICON_BEST = 512;

export type IconState = "approved" | "pending" | "missing" | "problem";

/** Where the icon is: refused at the last read, waiting for review, in the store, or missing. */
export function iconState(m: Pick<MediaOverview, "live" | "pending" | "icon_problem">): IconState {
  if (m.icon_problem) return "problem";
  const live = m.live?.icon;
  const pending = m.pending?.icon;
  if (pending && pending.sha256 !== live?.sha256) return "pending";
  if (live) return "approved";
  return "missing";
}

const STATE_BADGE: Record<IconState, { label: string; badge: string }> = {
  approved: { label: "APPROVED", badge: "official" },
  pending: { label: "IN REVIEW", badge: "soon" },
  missing: { label: "NO ICON YET", badge: "neutral" },
  problem: { label: "PROBLEM", badge: "bad" },
};

export function IconSlot({ app, overview: m }: { app: AppDetail; overview: MediaOverview }) {
  const state = iconState(m);
  const icon = (state === "pending" ? m.pending?.icon : m.live?.icon) ?? null;
  // What players see meanwhile: the shown thumbnail's crop, or the letter.
  const thumbnail = m.live?.thumbnail?.url ?? null;
  const shown = icon?.url ?? null;
  // Pictures the maker could crop an icon from: the newest set's thumbnail and screenshots.
  const working = m.pending ?? m.live;
  const pictures = useMemo(() => {
    const out: Array<{ label: string; picture: RepoPicture }> = [];
    if (working?.thumbnail) out.push({ label: `Thumbnail (${working.thumbnail.path})`, picture: working.thumbnail });
    working?.gallery.forEach((item, i) => {
      if (item.kind === "image") out.push({ label: `Screenshot ${i + 1} (${item.path})`, picture: item });
    });
    return out;
  }, [working]);
  const { label, badge } = STATE_BADGE[state];

  return (
    <section className="icon-slot" aria-labelledby="icon-slot-title">
      <div className="title-row">
        <h3 id="icon-slot-title">Icon</h3>
        <span className={`badge ${badge}`}>{label}</span>
      </div>
      <div className="icon-previews" aria-label={shown ? "Your icon at the sizes players see it" : "What players see without an icon"}>
        {[32, 64, 128].map((size) => (
          <figure key={size}>
            <AppIcon title={app.title} slug={app.slug} iconUrl={shown} thumbnailUrl={shown ? null : thumbnail} size={size} />
            <figcaption className="muted small">{size} px</figcaption>
          </figure>
        ))}
      </div>
      {state === "approved" && icon && (
        <p className="muted small">
          <code>{icon.path}</code>, {icon.width}×{icon.height}: in players' libraries, on store tiles and on your app's page.
          {m.pending && !m.pending.icon && " Your newest pictures, waiting for review, have no icon: once they're approved, it goes."}
        </p>
      )}
      {state === "pending" && icon && (
        <p className="muted small">
          <code>{icon.path}</code>, {icon.width}×{icon.height}, is waiting for review with your other new pictures.
          {m.live?.icon ? " The store keeps showing the current one meanwhile." : ""}
        </p>
      )}
      {state === "problem" && (
        <p className="icon-problem small">
          <b>The icon couldn't be used:</b> {m.icon_problem} Fix it in your repository and check again.
        </p>
      )}
      {state === "missing" && (
        <p className="small">
          No icon yet. That's fine for now: it never holds anything up. Until you add one, players see{" "}
          {thumbnail ? "a square from the middle of your thumbnail" : "your app's first letter"}.
        </p>
      )}
      {state !== "approved" && (
        <div className="icon-howto">
          <p className="muted small">
            To add one, put a square picture in your repository at <code>.slop/icon.png</code>: PNG, JPEG or WebP,{" "}
            {ICON_BEST}×{ICON_BEST} is best ({ICON_MIN} to 1024 pixels a side, up to 4 MB). Make it from your app's own
            art, such as its logo or a character, simple enough to read at 32 pixels. Then add this line to{" "}
            <code>{m.manifest_path}</code>, push, and press <b>Check my repository now</b>:
          </p>
          <pre className="code-block">{ICON_LINE}</pre>
          <p className="muted small">
            It's reviewed with your other pictures. See{" "}
            <Link to="/publish/guide#pictures-and-videos">Pictures and videos</Link> in How to publish.
          </p>
          {!isDesktop && pictures.length > 0 && <IconCropper pictures={pictures} slug={app.slug} />}
        </div>
      )}
    </section>
  );
}

/** Loads a picture so a canvas can read it back (the host must allow it, as GitHub's does). */
function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("couldn't load the picture"));
    img.src = url;
  });
}

/** The square to cut from a w×h picture: as big as fits, `at` (0–1) along its long side. */
export function cropSquare(w: number, h: number, at: number): { x: number; y: number; side: number } {
  const side = Math.min(w, h);
  const spare = Math.max(w, h) - side;
  const offset = Math.round(spare * Math.min(1, Math.max(0, at)));
  return w >= h ? { x: offset, y: 0, side } : { x: 0, y: offset, side };
}

/** Draws the square at `at` of `img`, scaled to `out`×`out`. */
function crop(img: HTMLImageElement, at: number, out: number): HTMLCanvasElement | null {
  const { x, y, side } = cropSquare(img.naturalWidth, img.naturalHeight, at);
  const canvas = document.createElement("canvas");
  canvas.width = out;
  canvas.height = out;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, x, y, side, side, 0, 0, out, out);
  return canvas;
}

/**
 * "Use a crop of this picture as the icon", within the repository model: the browser cuts a
 * square from one of the maker's own pictures and offers it as a file to commit. Nothing is sent
 * to SLAPorSLOP.
 */
function IconCropper({ pictures, slug }: { pictures: Array<{ label: string; picture: RepoPicture }>; slug: string }) {
  const [open, setOpen] = useState(false);
  const [chosen, setChosen] = useState(0);
  const [at, setAt] = useState(0.5);
  const [img, setImg] = useState<HTMLImageElement>();
  const [preview, setPreview] = useState<string>();
  const [error, setError] = useState<string>();
  const picture = pictures[Math.min(chosen, pictures.length - 1)].picture;
  const side = Math.min(picture.width, picture.height);
  const out = Math.min(side, ICON_BEST);
  const tooSmall = side < ICON_MIN;

  useEffect(() => {
    if (!open || tooSmall) return;
    let live = true;
    setImg(undefined);
    setError(undefined);
    loadImage(picture.url).then(
      (i) => live && setImg(i),
      () => live && setError("This picture couldn't be loaded here. Make the icon in an image editor instead."),
    );
    return () => {
      live = false;
    };
  }, [open, picture.url, tooSmall]);

  useEffect(() => {
    if (!img) return;
    try {
      setPreview(crop(img, at, out)?.toDataURL("image/png"));
    } catch {
      // The host didn't allow reading the picture back.
      setPreview(undefined);
      setError("Your browser can't crop this picture here. Make the icon in an image editor instead.");
    }
  }, [img, at, out]);

  const download = () => {
    const canvas = img && crop(img, at, out);
    if (!canvas) return;
    try {
      canvas.toBlob((blob) => {
        if (!blob) return;
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = "icon.png";
        a.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
      }, "image/png");
    } catch {
      setError("Your browser can't crop this picture here. Make the icon in an image editor instead.");
    }
  };

  if (!open)
    return (
      <button type="button" className="btn small" onClick={() => setOpen(true)}>
        Use a crop of one of my pictures as the icon
      </button>
    );
  return (
    <div className="icon-cropper">
      <p className="muted small">
        SLAPorSLOP can cut a square from one of your pictures for you to download. It isn't stored on SLAPorSLOP: add the file to
        your repository as <code>.slop/icon.png</code>, add the line above, push, and check again.
      </p>
      <label>
        Picture
        <select value={chosen} onChange={(e) => setChosen(Number(e.target.value))}>
          {pictures.map((p, i) => (
            <option key={p.picture.url} value={i}>
              {p.label}
            </option>
          ))}
        </select>
      </label>
      {tooSmall ? (
        <p className="small">
          This picture is {picture.width}×{picture.height}: too small for a {ICON_MIN}×{ICON_MIN} icon. Pick another.
        </p>
      ) : (
        <>
          {picture.width !== picture.height && (
            <label>
              Where to cut ({picture.width > picture.height ? "left to right" : "top to bottom"})
              <input type="range" min={0} max={100} value={Math.round(at * 100)} onChange={(e) => setAt(Number(e.target.value) / 100)} />
            </label>
          )}
          {error && <p className="small icon-problem">{error}</p>}
          {preview && (
            <div className="icon-previews" aria-label="The crop at the sizes players see it">
              {[32, 64, 128].map((size) => (
                <figure key={size}>
                  <img src={preview} alt={size === 128 ? `Crop for ${slug}'s icon` : ""} width={size} height={size} className="app-icon-crop" />
                  <figcaption className="muted small">{size} px</figcaption>
                </figure>
              ))}
            </div>
          )}
          <div className="row-actions">
            <button type="button" className="btn small slime" disabled={!preview} onClick={download}>
              Download icon.png ({out}×{out})
            </button>
            <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
              Close
            </button>
          </div>
        </>
      )}
    </div>
  );
}
