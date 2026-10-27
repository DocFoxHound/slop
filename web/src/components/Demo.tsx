// SPDX-License-Identifier: AGPL-3.0-or-later
// Browser demos (crates/slop-api/src/demos.rs): a new game's web build, playable in the browser
// for its first days, from a separate origin.

import { useState } from "react";

import { api, ApiError } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";

/** On the store page: play the demo now (opens on the play origin). */
export function DemoButton({ demo }: { demo: { url: string; ends_at: string } | null }) {
  if (!demo) return null;
  return (
    <p className="demo-button">
      <a className="btn slime" href={demo.url} target="_blank" rel="noreferrer">
        ▶ Play the demo in your browser
      </a>
      <span className="muted small"> Free to try until {formatDate(demo.ends_at)}.</span>
    </p>
  );
}

/** On the Manage page: offer a browser demo while the game is new. */
export function DemoPanel({ slug }: { slug: string }) {
  const status = useLoad(() => api.demo.status(slug), [slug]);
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string>();
  const s = status.data;
  if (!s || !s.available || (!s.eligible && !s.demo)) return null;
  const run = async (action: () => Promise<void>) => {
    setError(undefined);
    try {
      await action();
      status.reload();
      // It's fetched and scanned in the background.
      setTimeout(status.reload, 4000);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't do that.");
    }
  };
  const d = s.demo;
  return (
    <section className="panel panel-pad" aria-labelledby="demo-title">
      <h2 id="demo-title">Browser demo</h2>
      <p className="muted small">
        For its first {s.days} days, a new game can be played in the browser from its store page, free, with nothing to
        install. Attach a web build (a .zip with index.html, like Godot's Web export) to a release on your repository.
        SLAPorSLOP scans it and hosts it until {s.eligible_until ? formatDate(s.eligible_until) : "its days are up"}, then
        deletes it. Demos can't go online.
      </p>
      {d && (
        <p>
          <span className="badge">{d.status.toUpperCase()}</span>{" "}
          {d.status === "live" && s.play_url && (
            <>
              <a href={s.play_url} target="_blank" rel="noreferrer">
                Open it
              </a>{" "}
              · {d.plays} plays · until {formatDate(d.ends_at)}
            </>
          )}
          {d.status === "fetching" && "Downloading and scanning…"}
          {d.status === "failed" && d.detail}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {s.eligible && d?.status !== "live" && (
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void run(() => api.demo.start(slug, url.trim()));
          }}
        >
          <label>
            The web build's address on your release page
            <input type="url" value={url} required placeholder="https://github.com/you/game/releases/download/v1.0.0/web.zip" onChange={(e) => setUrl(e.target.value)} />
          </label>
          <button className="btn small slime" disabled={!url.trim()}>
            Host the demo
          </button>
        </form>
      )}
      {d?.status === "live" && (
        <button type="button" className="btn small ghost" onClick={() => void run(() => api.demo.end(slug))}>
          End the demo now
        </button>
      )}
    </section>
  );
}
