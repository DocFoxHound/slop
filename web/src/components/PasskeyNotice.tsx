// SPDX-License-Identifier: AGPL-3.0-or-later
// A gentle suggestion to add a passkey, on publishing and creative pages. Passkeys are optional
// (crates/slop-api/src/passkeys.rs): nothing waits for one, so this never blocks or warns.

import { Link } from "react-router";

import { useSession } from "../session";

/** Shown on publishing and creative pages until the account has a passkey. Optional. */
export function PasskeyNotice() {
  const { me } = useSession();
  if (!me || me.has_passkey) return null;
  return (
    <p className="muted small" role="note">
      Optional: a <Link to="/account/security#passkeys">passkey</Link> lets you sign in quickly, and nobody can phish it.
    </p>
  );
}
