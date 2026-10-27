// SPDX-License-Identifier: AGPL-3.0-or-later
// Sign-in, sign-up, approving the desktop app, combining accounts, and the account page's
// sign-ins and passkeys (the page itself is Account.tsx).

import { useEffect, useState, type FormEvent } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams, useSearchParams } from "react-router";

import { api, API_BASE, ApiError, PROVIDER_LABEL, type SignInProvider } from "../api";
import { LinkSignIn } from "../components/LinkSignIn";
import { createPasskey, passkeysSupported, signWithPasskey } from "../passkeys";
import { Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { useSession } from "../session";

const SIGNIN_ERRORS: Record<string, string> = {
  cancelled: "Sign-in was cancelled.",
  provider: "That sign-in didn't work. Please try again.",
  closed: "That account was closed, so its sign-in can't be used on SLAPorSLOP again.",
  link_expired: "Linking was started in another session. Start it again from your account page.",
};

/** Only same-site paths, matching the API's own check. */
function safeReturnTo(value: string | null): string {
  return value && value.startsWith("/") && !value.startsWith("//") && !value.includes("\\") ? value : "/";
}

/**
 * Goes on to `returnTo` after signing in. A server path (`/v1/…`, such as linking another sign-in)
 * is a full page load: the website's router doesn't know it and would show "Page not found".
 */
function goOn(returnTo: string, navigate: (to: string, opts?: { replace?: boolean }) => void, replace = false) {
  if (returnTo.startsWith("/v1/")) {
    if (replace) window.location.replace(API_BASE + returnTo);
    else window.location.assign(API_BASE + returnTo);
  } else {
    navigate(returnTo, { replace });
  }
}

export function SignIn() {
  useDocumentTitle("Sign in");
  const [params] = useSearchParams();
  const returnTo = safeReturnTo(params.get("return_to"));
  const error = params.get("error");

  if (isDesktop) return <DesktopSignIn />;
  return <WebSignIn returnTo={returnTo} error={error} />;
}

function WebSignIn({ returnTo, error }: { returnTo: string; error: string | null }) {
  const providers = useLoad(() => api.providers(), []);
  const [params] = useSearchParams();
  const { me } = useSession();
  const linking = params.get("reason") === "link";
  // Already signed in and on the way to the server (linking another sign-in, say): go straight on.
  const onToServer = !!me && returnTo.startsWith("/v1/");
  useEffect(() => {
    if (onToServer) window.location.replace(API_BASE + returnTo);
  }, [onToServer, returnTo]);
  if (onToServer) return <Loading />;
  const available = (Object.keys(PROVIDER_LABEL) as SignInProvider[]).filter((p) => providers.data?.[p]);
  return (
    <div className="page narrow">
      <div className="panel panel-pad auth-card">
        <h1>Sign in to SLAPorSLOP</h1>
        <p className="muted">SLAPorSLOP never sees or stores a password. You sign in with an account you already have.</p>
        {error && (
          <p className="form-error" role="alert">
            {SIGNIN_ERRORS[error] ?? "Sign-in failed. Please try again."}
          </p>
        )}
        {linking && (
          <p className="notice">
            To link another sign-in, first sign in with one you already use. Then you'll go straight on to linking.
          </p>
        )}
        {providers.data && available.length === 0 && <p className="notice">Sign-in isn't set up on this server yet.</p>}
        <div className="provider-list">
          {available.map((p) => (
            <a key={p} className="btn big provider" href={api.signInUrl(p, returnTo)}>
              Continue with {PROVIDER_LABEL[p]}
            </a>
          ))}
          {passkeysSupported() && <PasskeySignIn returnTo={returnTo} />}
        </div>
        <p className="faint small">
          Already have an account with another sign-in? Sign in with that one, then link this one under Account → Sign-ins.
          SLAPorSLOP never joins accounts just because their emails match.
        </p>
      </div>
    </div>
  );
}

/** Signing in with a passkey added under Account → Sign-ins. */
function PasskeySignIn({ returnTo }: { returnTo: string }) {
  const { refresh } = useSession();
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const go = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const started = await api.passkeys.signInStart();
      const credential = await signWithPasskey(started.options);
      await api.passkeys.signInFinish(started.challenge_id, credential);
      await refresh();
      goOn(returnTo, navigate);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "The passkey wasn't used. Try again, or use another sign-in.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button type="button" className="btn big provider" disabled={busy} onClick={() => void go()}>
        {busy ? "Waiting for your passkey…" : "Sign in with a passkey"}
      </button>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}

/** Account → Sign-in & security: passkeys on this account. Optional: a quicker sign-in, never required. */
export function Passkeys() {
  const keys = useLoad(() => api.passkeys.list(), []);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      keys.reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const add = () =>
    run(async () => {
      const started = await api.passkeys.registerStart();
      const credential = await createPasskey(started.options);
      await api.passkeys.registerFinish(started.challenge_id, name.trim() || "Passkey", credential);
      setName("");
    });
  if (!passkeysSupported()) return null;
  return (
    <section id="passkeys" className="account-panel passkeys" aria-labelledby="passkeys-title">
      <h3 className="account-panel-title" id="passkeys-title">
        Passkeys
      </h3>
      <div className="panel panel-pad">
        <p className="muted small">
          Optional: sign in with Face ID, Touch ID, Windows Hello or a security key instead of a provider. Nobody can phish
          a passkey, and SLAPorSLOP keeps only the public half.
        </p>
        {keys.data && keys.data.length > 0 && (
          <ul className="report-list">
            {keys.data.map((k) => (
              <li key={k.id}>
                <span>
                  <b>{k.name}</b>{" "}
                  <span className="muted small">
                    added {formatDate(k.created_at)}
                    {k.last_used_at && ` · last used ${formatDate(k.last_used_at)}`}
                  </span>
                </span>
                <button className="btn small ghost" disabled={busy} onClick={() => void run(() => api.passkeys.remove(k.id))}>
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
        <form
          className="reason-form"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <input aria-label="Name for the passkey" placeholder="e.g. MacBook, YubiKey" maxLength={60} value={name} onChange={(e) => setName(e.target.value)} />
          <button className="btn small" disabled={busy}>
            {busy ? "Waiting for your device…" : "Add a passkey"}
          </button>
        </form>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}

/** In the desktop app, signing in happens in the separate sign-in window. */
function DesktopSignIn() {
  useEffect(() => {
    void desktop.showLogin();
  }, []);
  return (
    <div className="page narrow">
      <div className="panel panel-pad auth-card">
        <h1>Sign in to SLAPorSLOP</h1>
        <p className="muted">Continue in the sign-in window.</p>
        <button className="btn" onClick={() => void desktop.showLogin()}>
          Show the sign-in window
        </button>
      </div>
    </div>
  );
}

/**
 * Website page the desktop app opens: the signed-in user approves the device, and the browser
 * hands a one-time code back to the app on this computer (docs/SECURITY.md §4.1).
 */
export function DesktopApprove() {
  const { id = "" } = useParams();
  const location = useLocation();
  const { me } = useSession();
  useDocumentTitle("Approve the SLAPorSLOP app");
  const request = useLoad(() => (me ? api.desktopRequest(id) : Promise.resolve(undefined)), [id, me?.id]);
  const [state, setState] = useState<"ask" | "sending" | "done" | "cancelled">("ask");
  const [error, setError] = useState<string>();

  if (me === undefined) return <Loading />;
  if (!me) return <Navigate to={`/signin?return_to=${encodeURIComponent(location.pathname)}`} replace />;
  if (request.error) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad">
          <h1>This sign-in link has expired</h1>
          <p>Go back to the SLAPorSLOP app and choose Sign in again.</p>
        </div>
      </div>
    );
  }
  if (!request.data) return <Loading />;

  const approve = async () => {
    setState("sending");
    try {
      const { redirect_url } = await api.approveDesktop(id);
      setState("done");
      // Hand the one-time code to the app listening on this computer.
      window.location.assign(redirect_url);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setState("ask");
    }
  };

  return (
    <div className="page narrow">
      <div className="panel panel-pad auth-card">
        {state === "cancelled" ? (
          <>
            <h1>Not signed in</h1>
            <p>The app wasn't given access. You can close this tab.</p>
          </>
        ) : state === "done" ? (
          <>
            <h1>Done</h1>
            <p>Go back to the SLAPorSLOP app. You can close this tab.</p>
          </>
        ) : (
          <>
            <h1>Sign in to the SLAPorSLOP app?</h1>
            <p>
              <b>{request.data.device_name}</b> wants to sign in as <b>{me.display_name}</b>.
            </p>
            <p className="notice">
              Only continue if you just chose <b>Sign in</b> in the SLAPorSLOP app on this computer. If you didn't, someone may be
              trying to get into your account: choose <b>Don't allow</b>.
            </p>
            {error && (
              <p className="form-error" role="alert">
                {error}
              </p>
            )}
            <div className="actions">
              <button className="btn ghost" onClick={() => setState("cancelled")}>
                Don't allow
              </button>
              <button className="btn slime" disabled={state === "sending"} onClick={() => void approve()}>
                Allow
              </button>
            </div>
            <p className="faint small">You can sign this device out at any time from your account page.</p>
          </>
        )}
      </div>
    </div>
  );
}

export function SignUp() {
  useDocumentTitle("Create your account");
  const navigate = useNavigate();
  const { refresh } = useSession();
  const info = useLoad(() => api.signupInfo(), []);
  const [name, setName] = useState<string>();
  const [country, setCountry] = useState("");
  const [ageConfirmed, setAgeConfirmed] = useState(false);
  const [acceptTerms, setAcceptTerms] = useState(false);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);

  if (info.error) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad">
          <h1>Sign-up expired</h1>
          <p>
            Your sign-in took too long or was finished in another window. <Link to="/signin">Start again</Link>.
          </p>
        </div>
      </div>
    );
  }
  if (!info.data) return <Loading />;

  const displayName = name ?? info.data.suggested_name;
  const minAge = info.data.countries.find((c) => c.code === country)?.min_age ?? 16;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const res = await api.signup({
        display_name: displayName,
        country_code: country,
        age_confirmed: ageConfirmed,
        accept_terms: acceptTerms,
      });
      await refresh();
      goOn(safeReturnTo(res.return_to), navigate, true);
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    } finally {
      setBusy(false);
    }
  };

  const fieldError = (field: string) =>
    error?.field === field ? (
      <span className="field-error" role="alert">
        {error.message}
      </span>
    ) : null;

  return (
    <div className="page narrow">
      <form className="panel panel-pad auth-card form" onSubmit={(e) => void submit(e)}>
        <h1>Create your SLAPorSLOP account</h1>
        {info.data.email && <p className="muted">Signed in with GitHub as {info.data.email}.</p>}

        <label>
          Display name
          <input value={displayName} maxLength={40} required onChange={(e) => setName(e.target.value)} />
          <span className="hint">Shown on your profile, reviews and votes.</span>
          {fieldError("display_name")}
        </label>

        <label>
          Country
          <select value={country} required onChange={(e) => setCountry(e.target.value)}>
            <option value="" disabled>
              Choose your country
            </option>
            {info.data.countries.map((c) => (
              <option key={c.code} value={c.code}>
                {c.name}
              </option>
            ))}
          </select>
          <span className="hint">SLAPorSLOP is available in the US, the EU and the UK. Some rules depend on where you live.</span>
          {fieldError("country_code")}
        </label>

        <label className="check">
          <input type="checkbox" checked={ageConfirmed} onChange={(e) => setAgeConfirmed(e.target.checked)} />I am {minAge} or
          older.
        </label>
        {fieldError("age_confirmed")}

        <label className="check">
          <input type="checkbox" checked={acceptTerms} onChange={(e) => setAcceptTerms(e.target.checked)} />
          <span>
            I accept the{" "}
            <Link to="/legal/terms" target="_blank">
              Terms of Service
            </Link>{" "}
            and have read the{" "}
            <Link to="/legal/privacy" target="_blank">
              Privacy Policy
            </Link>
            .
          </span>
        </label>
        {fieldError("accept_terms")}

        {error && !error.field && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <button className="btn slime big" disabled={busy || !country || !ageConfirmed || !acceptTerms}>
          {busy ? "Creating…" : "Create account"}
        </button>
      </form>
    </div>
  );
}

/** Account → Sign-ins: the accounts you can sign in with, and linking more. */
export function SignIns() {
  const identities = useLoad(() => api.identities(), []);
  const providers = useLoad(() => api.providers(), []);
  const [params] = useSearchParams();
  const [error, setError] = useState<string>();
  const linked = params.get("linked");
  const failed = params.get("error");
  const unlink = async (provider: string, subject: string) => {
    setError(undefined);
    try {
      await api.unlink(provider, subject);
      identities.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const available = (Object.keys(PROVIDER_LABEL) as SignInProvider[]).filter((p) => providers.data?.[p]);
  return (
    <section id="sign-ins" className="account-panel" aria-labelledby="sign-ins-title">
      <h3 className="account-panel-title" id="sign-ins-title">
        Sign-ins
      </h3>
      <div className="panel panel-pad">
        {linked && linked !== "already" && (
          <p className="notice" role="status">
            {PROVIDER_LABEL[linked as SignInProvider] ?? linked} is linked: you can sign in with it, and SLAPorSLOP knows the
            repositories it owns are yours.
          </p>
        )}
        {linked === "already" && params.get("picture") !== "refresh" && (
          <p className="notice">That account was already linked.</p>
        )}
        {failed === "provider" && <p className="form-error">That sign-in didn't work. Please try again.</p>}
        {(identities.data?.length ?? 0) > 0 && (
          <ul className="report-list">
            {identities.data?.map((i) => (
              <li key={`${i.provider}:${i.provider_subject}`}>
                <span>
                  <b>{PROVIDER_LABEL[i.provider] ?? i.provider}</b> {i.login && <span className="mono">{i.login}</span>}{" "}
                  <span className="muted small">linked {formatDate(i.created_at)}</span>
                </span>
                {(identities.data?.length ?? 0) > 1 && (
                  <button className="btn small ghost" onClick={() => void unlink(i.provider, i.provider_subject)}>
                    Unlink
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
        <p className="muted small">
          Link the accounts you use: to sign in with any of them, and to publish repositories they own (GitHub, GitLab and
          Bitbucket check who owns a repository by the linked account). Linking asks you to sign in to that account, so
          nobody can link someone else's. If it already has its own SLAPorSLOP account, you can combine the two.
        </p>
        <div className="row-actions">
          {available.map((p) => (
            <LinkSignIn key={p} provider={p} onReturn={identities.reload}>
              Link {PROVIDER_LABEL[p]}
            </LinkSignIn>
          ))}
        </div>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}

/** After linking a sign-in that has its own SLAPorSLOP account: combine the two? */
export function CombineAccounts() {
  useDocumentTitle("Combine accounts");
  const navigate = useNavigate();
  const { refresh } = useSession();
  const preview = useLoad(() => api.combine.preview(), []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  if (preview.error) {
    return (
      <div className="page narrow">
        <div className="empty-state">
          Nothing to combine (it may have expired). <Link to="/account/security#sign-ins">Back to your account</Link>
        </div>
      </div>
    );
  }
  if (!preview.data) return <Loading />;
  const p = preview.data;
  const go = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await api.combine.confirm();
      await refresh();
      navigate("/account/security?linked=" + p.provider + "#sign-ins");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const keep = async () => {
    await api.combine.cancel().catch(() => undefined);
    navigate("/account/security#sign-ins");
  };
  const things = [
    [p.apps, "app"],
    [p.library, "game in the library", "games in the library"],
    [p.votes, "vote"],
    [p.friends, "friend"],
    [p.teams, "team"],
    [p.jobs, "People job"],
    [p.chats, "chat"],
  ] as Array<[number, string, string?]>;
  return (
    <div className="page narrow">
      <h1 className="page-title">Combine accounts?</h1>
      <div className="panel panel-pad">
        <p>
          That {PROVIDER_LABEL[p.provider]} sign-in already has its own SLAPorSLOP account, <b>{p.name}</b>. You've just proved it's
          yours by signing in to it.
        </p>
        <p>Combining moves everything from it into the account you're signed in to now:</p>
        <ul>
          {things
            .filter(([n]) => n > 0)
            .map(([n, one, many]) => (
              <li key={one}>
                {n} {n === 1 ? one : (many ?? `${one}s`)}
              </li>
            ))}
          <li>its sign-ins, reports, donations and everything else</li>
        </ul>
        <p className="muted small">
          Where both accounts have the same thing (both voted on an app, both in a team), this account's stays. {p.name} is
          then closed, and its sign-in opens this account. Chat history stays on that account's devices; its chats continue
          here. This can't be undone.
        </p>
        {p.blocked && (
          <p className="notice" role="alert">
            These accounts can't be combined: {p.blocked}.
          </p>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <div className="row-actions">
          <button className="btn slime" disabled={busy || Boolean(p.blocked)} onClick={() => void go()}>
            Combine them
          </button>
          <button className="btn ghost" onClick={() => void keep()}>
            Keep them separate
          </button>
        </div>
      </div>
    </div>
  );
}
