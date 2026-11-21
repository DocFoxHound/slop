// SPDX-License-Identifier: AGPL-3.0-or-later
// Someone's profile address (pages/Profile.tsx at /u/{id}), with a Copy button: what people send
// each other to be added as friends or to a team.

import { useState } from "react";

import { displayAddress, siteOrigin } from "../browsing";

declare const __SLOP_SITE_URL__: string;

/** The full address of a profile, as it works in a browser (also from the desktop app). */
export function profileUrl(id: string): string {
  return `${origin()}/u/${id}`;
}

const origin = () => siteOrigin(window.location, typeof __SLOP_SITE_URL__ === "string" ? __SLOP_SITE_URL__ : "");

export function ProfileLink({ id, label }: { id: string; label: string }) {
  const url = profileUrl(id);
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard not allowed: the address is selectable in the box.
    }
  };
  return (
    <div className="login-link profile-link">
      <input readOnly value={displayAddress(origin(), `/u/${id}`)} aria-label={label} onFocus={(e) => e.currentTarget.select()} />
      <button type="button" className="btn small" onClick={() => void copy()}>
        {copied ? "Copied" : "Copy link"}
      </button>
    </div>
  );
}
