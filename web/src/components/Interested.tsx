// SPDX-License-Identifier: AGPL-3.0-or-later
// The Interested button for Coming soon and Beta apps (DESIGN §5, §6, §8): it puts the app in
// your library and tells you when it moves to beta or is released, and, under the bell, when its
// maker posts news (the checkbox under it switches that off for this app; News.tsx).

import { useState } from "react";
import { Link } from "react-router";

import { api, ApiError, type AppSummary } from "../api";
import { useSession } from "../session";
import { NewsSwitch } from "./News";
import { InlineIcon } from "./StoreIcons";

export function InterestedButton({
  app,
  isInterested,
  isMine,
  news,
  onNews,
  onMarked,
}: {
  app: Pick<AppSummary, "slug" | "stage" | "interested">;
  isInterested: boolean;
  isMine: boolean;
  /** Whether the maker's news is on for this mark (the checkbox under it). */
  news: boolean;
  onNews: (on: boolean) => void;
  /** After the mark was added or taken back. */
  onMarked?: (on: boolean) => void;
}) {
  const { me } = useSession();
  const [on, setOn] = useState(isInterested);
  const [count, setCount] = useState(app.interested);
  const [error, setError] = useState<string>();
  if (app.stage === "released") return null;
  const label = `${count.toLocaleString()} ${count === 1 ? "person is" : "people are"} interested`;
  if (isMine || me === undefined) return <p className="muted small interested-count">
        <InlineIcon name="star-on" /> {label}
      </p>;
  if (!me) {
    return (
      <p className="small interested-count">
        <InlineIcon name="star-on" /> {label} · <Link to={`/signin?return_to=${encodeURIComponent(`/app/${app.slug}`)}`}>Sign in</Link> to hear when
        it's out and to get news from its maker
      </p>
    );
  }
  const toggle = async () => {
    setError(undefined);
    try {
      if (on) await api.interested.remove(app.slug);
      else await api.interested.add(app.slug);
      setCount((c) => c + (on ? -1 : 1));
      setOn(!on);
      onMarked?.(!on);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <div className="interested">
      <button type="button" className={`btn ${on ? "slime" : ""}`} aria-pressed={on} onClick={() => void toggle()}>
        {on ? (
          <>
            <InlineIcon name="star-on" /> Interested
          </>
        ) : (
          <>
            <InlineIcon name="star" /> I'm interested
          </>
        )}
      </button>
      <p className="muted small">
        {label}.{" "}
        {on
          ? `It's in your library; we'll tell you when it ${app.stage === "coming_soon" ? "reaches beta or is released" : "is released"}.`
          : "Mark it to hear when it's out and to get news from its maker."}
      </p>
      {on && <NewsSwitch slug={app.slug} on={news} onChange={onNews} />}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
