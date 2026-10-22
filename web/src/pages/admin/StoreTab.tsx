// SPDX-License-Identifier: AGPL-3.0-or-later
// The front page for staff: the Featured carousel (featured.rs), shelves staff make
// (shelves.rs), and hiding the automatic shelves. Every change is in the audit log.

import { useState } from "react";
import { Link } from "react-router";

import { api, ApiError, type AdminFeatured, type AdminShelves } from "../../api";
import { ErrorState, Loading } from "../../components/ui";
import { useLoad } from "../../hooks";
import { ActionMessages, useAction } from "./shared";

export function StoreTab() {
  return (
    <>
      <FeaturedEditor />
      <ShelvesEditor />
    </>
  );
}

/** "cozy-farm" from an address or a link to the app. */
const slugOf = (text: string) => text.trim().replace(/^.*\/app\//, "").replace(/[/?#].*$/, "");

/** The front page's Featured carousel (featured.rs): apps in order, each with one line. */
function FeaturedEditor() {
  const current = useLoad(() => api.admin.featured(), []);
  const [draft, setDraft] = useState<AdminFeatured[]>();
  const [slug, setSlug] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const list = draft ?? current.data ?? [];
  const edit = (next: AdminFeatured[]) => setDraft(next);
  const move = (i: number, by: number) => {
    const next = [...list];
    const [item] = next.splice(i, 1);
    next.splice(i + by, 0, item);
    edit(next);
  };
  const save = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await api.admin.setFeatured(
        list.map((f) => ({ slug: f.slug, blurb: f.blurb })),
      );
      setDraft(undefined);
      current.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <details className="panel panel-pad" open>
      <summary>
        <b>Featured on the front page</b>{" "}
        <span className="muted">({current.data?.length ?? "…"} of 8)</span>
      </summary>
      <p className="muted small">
        Shown in this order at the top of the store. Only apps in the store
        show; the line under each title says why it's featured (up to 160
        characters). Changes are logged.
      </p>
      {current.error && (
        <ErrorState error={current.error} onRetry={current.reload} />
      )}
      <ol className="featured-edit">
        {list.map((f, i) => (
          <li key={f.slug}>
            <div className="title-row">
              <Link to={`/app/${f.slug}`}>
                <b>{f.title || f.slug}</b>
              </Link>
              {!f.listed && f.title && (
                <span className="badge warn">NOT IN THE STORE: HIDDEN</span>
              )}
              <button
                className="btn small ghost"
                disabled={i === 0}
                onClick={() => move(i, -1)}
                aria-label={`Move ${f.slug} up`}
              >
                ↑
              </button>
              <button
                className="btn small ghost"
                disabled={i === list.length - 1}
                onClick={() => move(i, 1)}
                aria-label={`Move ${f.slug} down`}
              >
                ↓
              </button>
              <button
                className="btn small ghost danger"
                onClick={() => edit(list.filter((_, j) => j !== i))}
              >
                Remove
              </button>
            </div>
            <input
              aria-label={`Why ${f.slug} is featured`}
              value={f.blurb}
              maxLength={160}
              placeholder="Why it's featured, e.g. A cozy roguelike that just hit 1.0"
              onChange={(e) =>
                edit(
                  list.map((x, j) =>
                    j === i ? { ...x, blurb: e.target.value } : x,
                  ),
                )
              }
            />
          </li>
        ))}
      </ol>
      <form
        className="reason-form"
        onSubmit={(e) => {
          e.preventDefault();
          const s = slugOf(slug);
          if (s && !list.some((f) => f.slug === s))
            edit([...list, { slug: s, title: "", blurb: "", listed: true }]);
          setSlug("");
        }}
      >
        <input
          aria-label="App address to feature"
          value={slug}
          placeholder="App address or link"
          onChange={(e) => setSlug(e.target.value)}
        />
        <button
          className="btn small"
          disabled={!slug.trim() || list.length >= 8}
        >
          Add
        </button>
        <button
          type="button"
          className="btn small slime"
          disabled={busy || !draft}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save"}
        </button>
      </form>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </details>
  );
}
type ShelfDraft = { slug: string; title: string; blurb: string; on_front: boolean; apps: Array<{ slug: string; title: string; listed: boolean }> };

function ShelvesEditor() {
  const data = useLoad(() => api.admin.shelves(), []);
  const action = useAction(data.reload);
  const [adding, setAdding] = useState(false);
  if (data.error) return <ErrorState error={data.error} onRetry={data.reload} />;
  if (!data.data) return <Loading />;
  const d: AdminShelves = data.data;
  const move = (i: number, by: number) => {
    const order = d.shelves.map((s) => s.slug);
    const [one] = order.splice(i, 1);
    order.splice(i + by, 0, one);
    void action.run("order", () => api.admin.orderShelves(order));
  };
  return (
    <>
      <section>
        <h2 className="section-title">Shelves staff made</h2>
        <p className="muted small">
          Each shelf shows on the front page after Featured, in this order, and has its own page at /shelf/its-address.
          Only apps in the store show; a shelf with none left doesn't.
        </p>
        {d.shelves.length === 0 && <div className="empty-state">No shelves yet.</div>}
        {d.shelves.map((s, i) => (
          <div key={s.slug} className="title-row">
            <div className="row-actions">
              <button
                className="btn small ghost"
                disabled={i === 0 || action.busy !== undefined}
                onClick={() => move(i, -1)}
                aria-label={`Move ${s.title} up`}
              >
                ↑
              </button>
              <button
                className="btn small ghost"
                disabled={i === d.shelves.length - 1 || action.busy !== undefined}
                onClick={() => move(i, 1)}
                aria-label={`Move ${s.title} down`}
              >
                ↓
              </button>
            </div>
            <ShelfForm key={JSON.stringify(s)} initial={s} onSaved={data.reload} />
          </div>
        ))}
        {adding ? (
          <ShelfForm
            initial={{ slug: "", title: "", blurb: "", on_front: true, apps: [] }}
            isNew
            onSaved={() => {
              setAdding(false);
              data.reload();
            }}
            onCancel={() => setAdding(false)}
          />
        ) : (
          <button className="btn small" onClick={() => setAdding(true)}>
            New shelf
          </button>
        )}
        <ActionMessages {...action} />
      </section>
      <section>
        <h2 className="section-title">Automatic shelves</h2>
        <p className="muted small">The front page fills these by itself. Hide one if it's being misused.</p>
        <ul className="plain">
          {d.built_in.map((b) => (
            <li key={b.key}>
              <label className="check">
                <input
                  type="checkbox"
                  checked={!b.hidden}
                  disabled={action.busy !== undefined}
                  onChange={() => void action.run(b.key, () => api.admin.hideFrontShelf(b.key, !b.hidden))}
                />
                <span>
                  {b.title} {b.hidden && <span className="badge warn">HIDDEN</span>}
                </span>
              </label>
            </li>
          ))}
        </ul>
      </section>
    </>
  );
}

function ShelfForm({
  initial,
  isNew,
  onSaved,
  onCancel,
}: {
  initial: ShelfDraft;
  isNew?: boolean;
  onSaved: () => void;
  onCancel?: () => void;
}) {
  const [s, setS] = useState(initial);
  const [add, setAdd] = useState("");
  const [dirty, setDirty] = useState(isNew ?? false);
  const action = useAction(() => {
    setDirty(false);
    onSaved();
  });
  const edit = (next: Partial<ShelfDraft>) => {
    setS({ ...s, ...next });
    setDirty(true);
  };
  const moveApp = (i: number, by: number) => {
    const apps = [...s.apps];
    const [one] = apps.splice(i, 1);
    apps.splice(i + by, 0, one);
    edit({ apps });
  };
  const save = () => {
    const body = { title: s.title, blurb: s.blurb, on_front: s.on_front, apps: s.apps.map((a) => a.slug) };
    void action.run("save", () =>
      isNew ? api.admin.createShelf({ ...body, slug: s.slug.trim() }) : api.admin.updateShelf(s.slug, body),
    );
  };
  const label = s.title || "New shelf";
  return (
    <details className="panel panel-pad admin-shelf" open={isNew}>
      <summary>
        <b>{label}</b>{" "}
        <span className="muted small">
          {s.apps.length} apps{!s.on_front && " · off the front page"}
        </span>
      </summary>
      <form
        className="form"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        {!isNew && (
          <p className="muted small">
            Its page: <Link to={`/shelf/${s.slug}`}>/shelf/{s.slug}</Link>
          </p>
        )}
        {isNew && (
          <label>
            Address
            <input
              value={s.slug}
              required
              maxLength={40}
              pattern="[a-z0-9]+(-[a-z0-9]+)*"
              placeholder="cozy-games"
              onChange={(e) => edit({ slug: e.target.value.toLowerCase() })}
            />
          </label>
        )}
        <label>
          Title
          <input value={s.title} required maxLength={60} onChange={(e) => edit({ title: e.target.value })} />
        </label>
        <label>
          What ties them together
          <input value={s.blurb} maxLength={160} onChange={(e) => edit({ blurb: e.target.value })} />
        </label>
        <label className="check small">
          <input type="checkbox" checked={s.on_front} onChange={(e) => edit({ on_front: e.target.checked })} />
          <span>Show on the front page</span>
        </label>
        <ol className="featured-edit">
          {s.apps.map((a, i) => (
            <li key={a.slug} className="title-row">
              <Link to={`/app/${a.slug}`}>{a.title || a.slug}</Link>
              {!a.listed && a.title && <span className="badge warn">NOT IN THE STORE: HIDDEN</span>}
              <button
                type="button"
                className="btn small ghost"
                disabled={i === 0}
                onClick={() => moveApp(i, -1)}
                aria-label={`Move ${a.slug} up`}
              >
                ↑
              </button>
              <button
                type="button"
                className="btn small ghost"
                disabled={i === s.apps.length - 1}
                onClick={() => moveApp(i, 1)}
                aria-label={`Move ${a.slug} down`}
              >
                ↓
              </button>
              <button
                type="button"
                className="btn small ghost danger"
                onClick={() => edit({ apps: s.apps.filter((_, j) => j !== i) })}
              >
                Remove
              </button>
            </li>
          ))}
        </ol>
        <div className="reason-form">
          <input
            aria-label={`App address to add to ${label}`}
            value={add}
            placeholder="App address or link"
            onChange={(e) => setAdd(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                (e.currentTarget.nextElementSibling as HTMLButtonElement | null)?.click();
              }
            }}
          />
          <button
            type="button"
            className="btn small"
            disabled={!add.trim() || s.apps.length >= 24}
            onClick={() => {
              const slug = slugOf(add);
              if (slug && !s.apps.some((a) => a.slug === slug)) edit({ apps: [...s.apps, { slug, title: "", listed: true }] });
              setAdd("");
            }}
          >
            Add app
          </button>
        </div>
        <div className="row-actions">
          <button className="btn small slime" disabled={!dirty || action.busy !== undefined}>
            {isNew ? "Make the shelf" : "Save"}
          </button>
          {onCancel && (
            <button type="button" className="btn small ghost" onClick={onCancel}>
              Cancel
            </button>
          )}
          {!isNew && (
            <button
              type="button"
              className="btn small ghost danger"
              disabled={action.busy !== undefined}
              onClick={() => {
                if (confirm(`Delete the shelf "${s.title}"?`)) void action.run("delete", () => api.admin.deleteShelf(s.slug));
              }}
            >
              Delete shelf
            </button>
          )}
        </div>
        <ActionMessages {...action} />
      </form>
    </details>
  );
}
