// SPDX-License-Identifier: AGPL-3.0-or-later
// Linking a sign-in to the account (`/v1/auth/<provider>/link`): Link GitHub, Refresh picture.

import { useEffect, useState, type ReactNode } from "react";

import { api, type SignInProvider } from "../api";
import { siteOrigin } from "../browsing";
import { isDesktop, openExternal } from "../platform";

declare const __SLOP_SITE_URL__: string;

/**
 * A link that signs in to `provider` and links it to the account, then comes back to `returnTo`.
 * On the website it's an ordinary link: the server sends the browser to the provider and back. The
 * desktop app's pages are its own, so that address would open the app's "Page not found": there it
 * opens in the browser, on the site the app is for, and `onReturn` runs when the app is next in
 * front (to load what changed).
 */
export function LinkSignIn({
  provider,
  returnTo,
  className = "btn small",
  onReturn,
  children,
}: {
  provider: SignInProvider;
  returnTo?: string;
  className?: string;
  onReturn?: () => void;
  children: ReactNode;
}) {
  const [opened, setOpened] = useState(false);
  useEffect(() => {
    if (!opened || !onReturn) return;
    window.addEventListener("focus", onReturn);
    return () => window.removeEventListener("focus", onReturn);
  }, [opened, onReturn]);
  const href = api.linkUrl(provider, returnTo);
  if (!isDesktop) {
    return (
      <a className={className} href={href}>
        {children}
      </a>
    );
  }
  const origin = siteOrigin(window.location, typeof __SLOP_SITE_URL__ === "string" ? __SLOP_SITE_URL__ : "");
  return (
    <>
      <button
        type="button"
        className={className}
        onClick={() => {
          setOpened(true);
          void openExternal(origin + href);
        }}
      >
        {children}
      </button>
      {opened && (
        <span className="muted small" role="status">
          Opened in your browser: finish there, then come back.
        </span>
      )}
    </>
  );
}
