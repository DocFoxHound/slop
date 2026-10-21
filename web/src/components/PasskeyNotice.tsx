// The deadline for publishers and creatives to add a passkey (crates/slop-api/src/passkeys.rs).

import { Link } from "react-router";

import { formatDate } from "../format";
import { useSession } from "../session";

/** Shown on publishing and creative pages until the account has a passkey. */
export function PasskeyNotice() {
  const { me } = useSession();
  if (!me || me.has_passkey || !me.passkey_due_at) return null;
  const now = Date.now();
  const grace = me.passkey_grace_until && Date.parse(me.passkey_grace_until) > now ? me.passkey_grace_until : null;
  const overdue = Date.parse(me.passkey_due_at) <= now;
  return (
    <div className={`notice small ${overdue && !grace ? "warn" : ""}`} role="note">
      {grace ? (
        <>
          You can publish and manage payouts without a passkey until <b>{formatDate(grace)}</b>.{" "}
        </>
      ) : overdue ? (
        <>Publishing and changing where money goes need a passkey now. </>
      ) : (
        <>
          From <b>{formatDate(me.passkey_due_at)}</b>, publishing and changing where money goes need a passkey, so someone
          who gets into your sign-in elsewhere can't do either.{" "}
        </>
      )}
      <Link to="/account#sign-ins">Add a passkey</Link>
    </div>
  );
}
