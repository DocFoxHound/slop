// The Interested button for Coming soon and Beta apps (DESIGN §5, §6, §8): it puts the app in
// your library and tells you when it moves to beta or is released.

import { useState } from "react";
import { Link } from "react-router";

import { api, ApiError, type AppSummary } from "../api";
import { useSession } from "../session";

export function InterestedButton({
  app,
  isInterested,
  isMine,
}: {
  app: Pick<AppSummary, "slug" | "stage" | "interested">;
  isInterested: boolean;
  isMine: boolean;
}) {
  const { me } = useSession();
  const [on, setOn] = useState(isInterested);
  const [count, setCount] = useState(app.interested);
  const [error, setError] = useState<string>();
  if (app.stage === "released") return null;
  const label = `${count.toLocaleString()} ${count === 1 ? "person is" : "people are"} interested`;
  if (isMine || me === undefined) return <p className="muted small interested-count">★ {label}</p>;
  if (!me) {
    return (
      <p className="small interested-count">
        ★ {label} · <Link to={`/signin?return_to=${encodeURIComponent(`/app/${app.slug}`)}`}>Sign in</Link> to hear when
        it's out
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
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <div className="interested">
      <button type="button" className={`btn ${on ? "slime" : ""}`} aria-pressed={on} onClick={() => void toggle()}>
        {on ? "★ Interested" : "☆ I'm interested"}
      </button>
      <p className="muted small">
        {label}.{" "}
        {on
          ? `It's in your library; we'll tell you when it ${app.stage === "coming_soon" ? "reaches beta or is released" : "is released"}.`
          : "Mark it to hear when it's out."}
      </p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
