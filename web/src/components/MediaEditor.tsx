// The maker's thumbnail and gallery, on the store-page editor. Each change saves straight away
// (crates/slop-api/src/media.rs); like the rest of the store page, it's live as soon as it's saved.

import { useRef, useState, type FormEvent } from "react";

import { api, ApiError, type AppDetail, type GalleryItem } from "../api";
import { Art } from "./ui";

/** Matches the server's MAX_IMAGE_BYTES and MAX_GALLERY. */
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_GALLERY = 12;
const ACCEPT = "image/png,image/jpeg,image/webp,image/gif";

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("Couldn't read that file."));
    reader.readAsDataURL(file);
  });
}

function checkFile(file: File): string | undefined {
  if (!ACCEPT.split(",").includes(file.type)) return "Pick a PNG, JPEG, WebP or GIF picture.";
  if (file.size > MAX_BYTES) return "Pictures can be up to 8 MB.";
  return undefined;
}

function useBusy() {
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();
  const run = async (what: string, action: () => Promise<void>) => {
    setBusy(what);
    setError(undefined);
    try {
      await action();
    } catch (err) {
      setError(err instanceof ApiError || err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(undefined);
    }
  };
  return { busy, error, setError, run };
}

export function MediaEditor({ app }: { app: AppDetail }) {
  return (
    <>
      <ThumbnailEditor app={app} />
      <GalleryEditor app={app} />
    </>
  );
}

function ThumbnailEditor({ app }: { app: AppDetail }) {
  const [thumbnail, setThumbnail] = useState(app.thumbnail_url);
  const { busy, error, setError, run } = useBusy();
  const input = useRef<HTMLInputElement>(null);

  const upload = (file: File | undefined) => {
    if (!file) return;
    const problem = checkFile(file);
    if (problem) return setError(problem);
    void run("upload", async () => {
      const { thumbnail_url } = await api.setThumbnail(app.slug, await readAsDataUrl(file));
      setThumbnail(thumbnail_url);
    });
    if (input.current) input.current.value = "";
  };

  return (
    <section className="panel panel-pad form media-editor">
      <h2>Thumbnail</h2>
      <p className="muted small">
        The picture on your app's tile in the store. Best at <b>920×430</b> (at least 460×215); other shapes are cropped
        from the middle. PNG, JPEG, WebP or GIF, up to 8 MB. Saved as soon as you pick it.
      </p>
      <div className="thumb-row">
        <div className="thumb-preview">
          <Art app={{ ...app, thumbnail_url: thumbnail }} withName={!thumbnail} />
        </div>
        <div className="row-actions">
          <label className="btn slime file-btn">
            {busy === "upload" ? "Uploading…" : thumbnail ? "Replace" : "Upload a thumbnail"}
            <input
              ref={input}
              type="file"
              accept={ACCEPT}
              disabled={Boolean(busy)}
              onChange={(e) => upload(e.target.files?.[0])}
            />
          </label>
          {thumbnail && (
            <button
              type="button"
              className="btn"
              disabled={Boolean(busy)}
              onClick={() =>
                void run("remove", async () => {
                  await api.removeThumbnail(app.slug);
                  setThumbnail(null);
                })
              }
            >
              Remove
            </button>
          )}
        </div>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

function GalleryEditor({ app }: { app: AppDetail }) {
  const [items, setItems] = useState<GalleryItem[]>(app.gallery);
  const [video, setVideo] = useState("");
  const { busy, error, setError, run } = useBusy();
  const input = useRef<HTMLInputElement>(null);
  const full = items.length >= MAX_GALLERY;

  const uploadAll = (files: FileList | null) => {
    if (!files?.length) return;
    const list = Array.from(files).slice(0, MAX_GALLERY - items.length);
    const problem = list.map(checkFile).find(Boolean);
    if (problem) return setError(problem);
    void run("upload", async () => {
      for (const file of list) {
        setItems(await api.addToGallery(app.slug, { kind: "image", image: await readAsDataUrl(file) }));
      }
    });
    if (input.current) input.current.value = "";
  };
  const addVideo = (e: FormEvent) => {
    e.preventDefault();
    void run("video", async () => {
      setItems(await api.addToGallery(app.slug, { kind: "video", url: video }));
      setVideo("");
    });
  };
  const move = (i: number, by: number) => {
    const next = [...items];
    const [item] = next.splice(i, 1);
    next.splice(i + by, 0, item);
    setItems(next);
    void run("order", async () => setItems(await api.reorderGallery(app.slug, next.map((x) => x.id))));
  };

  return (
    <section className="panel panel-pad form media-editor">
      <h2>
        Screenshots &amp; videos{" "}
        <span className="muted small">
          ({items.length}/{MAX_GALLERY})
        </span>
      </h2>
      <p className="muted small">
        Shown as a carousel at the top of your app's page, in this order. Screenshots up to 1920×1080 (bigger ones are
        scaled down), at least 320×180. Videos are YouTube links; the page only loads YouTube when someone presses play.
      </p>

      {items.length > 0 && (
        <ol className="gallery-edit">
          {items.map((item, i) => (
            <li key={item.id}>
              <div className="ge-thumb">
                <img src={item.url} alt="" loading="lazy" />
                {item.kind === "video" && (
                  <span className="play-mark small" aria-hidden="true">
                    ▶
                  </span>
                )}
              </div>
              <CaptionField
                item={item}
                index={i}
                disabled={Boolean(busy)}
                onSave={(caption) =>
                  void run("caption", async () => setItems(await api.captionGalleryItem(app.slug, item.id, caption)))
                }
              />
              <div className="row-actions">
                <button type="button" className="btn small" disabled={Boolean(busy) || i === 0} onClick={() => move(i, -1)} aria-label={`Move item ${i + 1} earlier`}>
                  ↑
                </button>
                <button
                  type="button"
                  className="btn small"
                  disabled={Boolean(busy) || i === items.length - 1}
                  onClick={() => move(i, 1)}
                  aria-label={`Move item ${i + 1} later`}
                >
                  ↓
                </button>
                <button
                  type="button"
                  className="btn small"
                  disabled={Boolean(busy)}
                  onClick={() => void run("remove", async () => setItems(await api.removeFromGallery(app.slug, item.id)))}
                  aria-label={`Remove item ${i + 1}`}
                >
                  Remove
                </button>
              </div>
            </li>
          ))}
        </ol>
      )}

      <div className="row-actions">
        <label className={`btn slime file-btn ${full ? "disabled" : ""}`}>
          {busy === "upload" ? "Uploading…" : "Add screenshots"}
          <input ref={input} type="file" accept={ACCEPT} multiple disabled={Boolean(busy) || full} onChange={(e) => uploadAll(e.target.files)} />
        </label>
      </div>
      <form className="video-add" onSubmit={addVideo}>
        <label>
          Add a YouTube video
          <input
            type="url"
            value={video}
            placeholder="https://www.youtube.com/watch?v=…"
            disabled={full}
            onChange={(e) => setVideo(e.target.value)}
          />
        </label>
        <button className="btn" disabled={Boolean(busy) || full || !video.trim()}>
          {busy === "video" ? "Adding…" : "Add video"}
        </button>
      </form>
      {full && <p className="muted small">The gallery is full. Remove something to add more.</p>}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

function CaptionField({
  item,
  index,
  disabled,
  onSave,
}: {
  item: GalleryItem;
  index: number;
  disabled: boolean;
  onSave: (caption: string) => void;
}) {
  const [caption, setCaption] = useState(item.caption ?? "");
  const changed = caption.trim() !== (item.caption ?? "");
  return (
    <form
      className="ge-caption"
      onSubmit={(e) => {
        e.preventDefault();
        onSave(caption);
      }}
    >
      <label>
        <span className="muted small">
          {item.kind === "video" ? "Video" : "Screenshot"} {index + 1} caption (optional)
        </span>
        <input value={caption} maxLength={200} onChange={(e) => setCaption(e.target.value)} placeholder="e.g. The first boss" />
      </label>
      {changed && (
        <button className="btn small" disabled={disabled}>
          Save caption
        </button>
      )}
    </form>
  );
}
