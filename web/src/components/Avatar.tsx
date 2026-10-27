// SPDX-License-Identifier: AGPL-3.0-or-later
// A person's picture: the one they chose from a linked sign-in (crates/slop-api/src/avatars.rs,
// served by SLAPorSLOP itself), or the first letter of their name on a coloured square. The size comes
// from the class (`avatar` in its places in styles.css, `p-avatar` on People); it never shrinks.

import { useEffect, useState } from "react";

export function initial(name: string): string {
  const first = name.trim().charAt(0);
  return first ? first.toUpperCase() : "?";
}

export function Avatar({
  name,
  url,
  className = "avatar",
}: {
  name: string;
  url?: string | null;
  /** The size and shape: "avatar" (its context sets the size), "avatar big", "p-avatar"… */
  className?: string;
}) {
  // A picture that doesn't load (removed meanwhile, offline) falls back to the letter.
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [url]);
  return (
    <span className={className} aria-hidden="true">
      {url && !failed ? (
        <img src={url} alt="" loading="lazy" decoding="async" onError={() => setFailed(true)} />
      ) : (
        initial(name)
      )}
    </span>
  );
}
