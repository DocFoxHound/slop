// SPDX-License-Identifier: AGPL-3.0-or-later
// Sign-in, sign-up and the account page (with the devices list, docs/SECURITY.md §4.2).

import { useEffect, useState, type FormEvent } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams, useSearchParams } from "react-router";

import { api, ApiError, PROVIDER_LABEL, type SignInProvider } from "../api";
import { StepUpPrompt } from "../components/PasskeyStepUp";
import { AiAgents } from "../components/AiAgents";
import { SigningKeys } from "../components/SigningKeys";
import { createPasskey, passkeysSupported, signWithPasskey } from "../passkeys";
import { ErrorState, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { useSession } from "../session";
import {
  AccountAgreements,
  AccountData,
  AccountDecisions,
  AccountDonations,
  AccountNotifications,
  AccountReports,
} from "../components/AccountSections";

const SIGNIN_ERRORS: Record<string, string> = {
  cancelled: "Sign-in was cancelled.",
  provider: "That sign-in didn't work. Please try again.",
  closed: "That account was closed, so its sign-in can't be used on Slop again.",
  link_expired: "Linking was started in another session. Start it again from your account page.",
};

/** Only same-site paths, matching the API's own check. */
function safeReturnTo(value: string | null): string {
  return value && value.startsWith("/") && !value.startsWith("//") && !value.includes("\\") ? value : "/";
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
  const linking = params.get("reason") === "link";
  const available = (Object.keys(PROVIDER_LABEL) as SignInProvider[]).filter((p) => providers.data?.[p]);
  return (
    <div className="page narrow">
      <div className="panel panel-pad auth-card">
        <h1>Sign in to Slop</h1>
        <p className="muted">Slop never sees or stores a password. You sign in with an account you already have.</p>
        {error && (
          <p className="form-error" role="alert">
            {SIGNIN_ERRORS[error] ?? "Sign-in failed. Please try again."}
          </p>
        )}
        {linking && (
          <p className="notice">
            To link another sign-in, first sign in again with one you already use. Then you'll go straight on to linking.
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
          Slop never joins accounts just because their emails match.
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
      navigate(returnTo);
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

/** Account → Sign-ins: passkeys on this account. Adding or removing one needs a recent sign-in. */
function Passkeys() {
  const keys = useLoad(() => api.passkeys.list(), []);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError | string>();
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      keys.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err : err instanceof Error ? err.message : String(err));
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
    <div className="passkeys">
      <h3>Passkeys</h3>
      <p className="muted small">
        Sign in with Face ID, Touch ID, Windows Hello or a security key instead of a provider, and confirm sensitive changes
        without signing in again. Slop keeps only the public half.
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
          {typeof error === "string" ? (
            error
          ) : error.code === "step_up_required" ? (
            <StepUpPrompt signInAgain={`/signin?return_to=${encodeURIComponent("/account#sign-ins")}`} onDone={() => setError(undefined)} />
          ) : (
            error.message
          )}
        </p>
      )}
    </div>
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
        <h1>Sign in to Slop</h1>
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
  useDocumentTitle("Approve the Slop app");
  const request = useLoad(() => (me ? api.desktopRequest(id) : Promise.resolve(undefined)), [id, me?.id]);
  const [state, setState] = useState<"ask" | "sending" | "done" | "cancelled">("ask");
  const [error, setError] = useState<string>();
  const [stepUp, setStepUp] = useState(false);

  if (me === undefined) return <Loading />;
  if (!me) return <Navigate to={`/signin?return_to=${encodeURIComponent(location.pathname)}`} replace />;
  if (request.error) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad">
          <h1>This sign-in link has expired</h1>
          <p>Go back to the Slop app and choose Sign in again.</p>
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
      // An old sign-in can't approve a new device: confirm it's you first.
      setStepUp(err instanceof ApiError && err.code === "step_up_required");
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
            <p>Go back to the Slop app. You can close this tab.</p>
          </>
        ) : (
          <>
            <h1>Sign in to the Slop app?</h1>
            <p>
              <b>{request.data.device_name}</b> wants to sign in as <b>{me.display_name}</b>.
            </p>
            <p className="notice">
              Only continue if you just chose <b>Sign in</b> in the Slop app on this computer. If you didn't, someone may be
              trying to get into your account: choose <b>Don't allow</b>.
            </p>
            {error && (
              <p className="form-error" role="alert">
                {stepUp ? (
                  <StepUpPrompt
                    signInAgain={`/signin?return_to=${encodeURIComponent(location.pathname)}`}
                    onDone={() => {
                      setStepUp(false);
                      setError(undefined);
                    }}
                  />
                ) : (
                  error
                )}
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
      navigate(safeReturnTo(res.return_to), { replace: true });
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
        <h1>Create your Slop account</h1>
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
          <span className="hint">Slop is available in the US, the EU and the UK. Some rules depend on where you live.</span>
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

export function Account() {
  useDocumentTitle("Your account");
  const navigate = useNavigate();
  const { me, signOut, refresh } = useSession();
  const sessions = useLoad(() => (me ? api.sessions() : Promise.resolve([])), [me?.id]);

  if (me === undefined) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad">
          <h1>Your account</h1>
          <p>
            <Link to="/signin?return_to=/account">Sign in</Link> to see your account.
          </p>
        </div>
      </div>
    );
  }

  const revoke = async (id: string) => {
    await api.revokeSession(id);
    sessions.reload();
  };

  return (
    <div className="page narrow">
      <h1 className="page-title">Your account</h1>
      <div className="panel panel-pad">
        <dl className="kv">
          <dt>Name</dt>
          <dd>{me.display_name}</dd>
          <dt>Email</dt>
          <dd>{me.email ?? <span className="muted">None shared by GitHub</span>}</dd>
          <dt>Country</dt>
          <dd>{me.country_code}</dd>
        </dl>
      </div>

      <h2 className="section-title" id="privacy">Privacy</h2>
      <div className="panel panel-pad">
        <label className="check">
          <input
            type="checkbox"
            checked={me.profile_public}
            onChange={(e) => void api.updateMe({ profile_public: e.target.checked }).then(refresh)}
          />{" "}
          <span>
            <b>Show my votes and reviews on my profile</b>
            <span className="muted small" style={{ display: "block" }}>
              Off: your profile shows only your name and apps, and your reviews show as "A player".{" "}
              <Link to={`/u/${me.id}`}>See your profile</Link>
            </span>
          </span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={me.share_playtime}
            onChange={(e) => void api.updateMe({ share_playtime: e.target.checked }).then(refresh)}
          />{" "}
          <span>
            <b>Share my playtime</b>
            <span className="muted small" style={{ display: "block" }}>
              Games you launch from Slop can count you in their "playing now" numbers, and your playtime makes your
              votes count more. Slop gets nothing but a launch token and a heartbeat; never what you do in the game.
            </span>
          </span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={me.personalize}
            onChange={(e) => void api.updateMe({ personalize: e.target.checked }).then(refresh)}
          />{" "}
          <span>
            <b>Recommendations based on my activity</b>
            <span className="muted small" style={{ display: "block" }}>
              "For you" uses your library, what you've played and how you voted. Off: everyone sees the same popular,
              well-rated and new apps.
            </span>
          </span>
        </label>
        {me.gpc && (
          <p className="muted small">
            Your browser sends Global Privacy Control, asking sites not to sell or share your data. Slop never does, and
            because of it, recommendations based on your activity stay off unless you turn them on.
          </p>
        )}
      </div>

      <SignIns />
      <SigningKeys />
      <AiAgents />
      <AccountNotifications />
      {isDesktop && <ThisDevice />}
      <AccountDecisions />
      <AccountDonations />
      <AccountReports />
      <AccountAgreements />

      <h2 className="section-title">Where you're signed in</h2>
      {sessions.error && <ErrorState error={sessions.error} onRetry={sessions.reload} />}
      <ul className="sessions panel">
        {sessions.data?.map((s) => (
          <li key={s.id}>
            <div>
              <b>
                {s.kind === "desktop"
                  ? (s.user_agent ?? "Slop app")
                  : s.kind === "agent"
                    ? (s.user_agent ?? "AI agent")
                    : describeAgent(s.user_agent)}
              </b>{" "}
              {s.current && <span className="badge official">THIS DEVICE</span>}
              <div className="muted small">
                Signed in {formatDate(s.created_at)} · last used {formatDate(s.last_seen_at)}
              </div>
            </div>
            {!s.current && (
              <button className="btn small" onClick={() => void revoke(s.id)}>
                Sign out
              </button>
            )}
          </li>
        ))}
      </ul>

      <button
        className="btn ghost danger"
        onClick={() =>
          void signOut().then(() => navigate("/", { replace: true }))
        }
      >
        Sign out
      </button>

      <AccountData />
    </div>
  );
}

function describeAgent(ua: string | null): string {
  if (!ua) return "Unknown device";
  const browser = /Firefox\//.test(ua) ? "Firefox" : /Edg\//.test(ua) ? "Edge" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "Browser";
  const os = /Windows/.test(ua) ? "Windows" : /Mac OS/.test(ua) ? "macOS" : /Android/.test(ua) ? "Android" : /iPhone|iPad/.test(ua) ? "iOS" : /Linux/.test(ua) ? "Linux" : "";
  return os ? `${browser} on ${os}` : browser;
}

/** The desktop app's own settings: chat notifications and the in-game overlay. */
function ThisDevice() {
  const [notify, setNotify] = useState<{ on: boolean; show_text: boolean }>();
  const [overlay, setOverlay] = useState<boolean>();
  useEffect(() => {
    desktop.chat<{ on: boolean; show_text: boolean }>({ action: "notification_settings" }).then(setNotify, () => undefined);
    desktop.overlaySetting().then(setOverlay, () => undefined);
  }, []);
  const saveNotify = (next: { on: boolean; show_text: boolean }) => {
    setNotify(next);
    void desktop.chat({ action: "notifications", ...next }).catch(() => undefined);
  };
  return (
    <section id="this-device">
      <h2 className="section-title">This computer</h2>
      <div className="panel panel-pad">
        {notify && (
          <>
            <label className="check">
              <input type="checkbox" checked={notify.on} onChange={(e) => saveNotify({ ...notify, on: e.target.checked })} />
              <span>
                <b>Notifications for new chat messages</b>
                <span className="muted small" style={{ display: "block" }}>
                  Shown when Slop isn't in front. Muted chats never notify.
                </span>
              </span>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={notify.show_text}
                disabled={!notify.on}
                onChange={(e) => saveNotify({ ...notify, show_text: e.target.checked })}
              />
              <span>Show the message in the notification (otherwise just who it's from)</span>
            </label>
          </>
        )}
        {overlay !== undefined && (
          <label className="check">
            <input
              type="checkbox"
              checked={overlay}
              onChange={(e) => void desktop.overlaySetting(e.target.checked).then(setOverlay, () => undefined)}
            />
            <span>
              <b>In-game overlay (Shift+Tab)</b>
              <span className="muted small" style={{ display: "block" }}>
                Chats and friends over a game you started from Slop, when it's fullscreen or maximized. Shift+Tab works as
                usual everywhere else.
              </span>
            </span>
          </label>
        )}
      </div>
    </section>
  );
}

/** Account → Sign-ins: the accounts you can sign in with, and linking more. */
export function SignIns() {
  const identities = useLoad(() => api.identities(), []);
  const providers = useLoad(() => api.providers(), []);
  const [params] = useSearchParams();
  const [error, setError] = useState<string>();
  const [stepUp, setStepUp] = useState(false);
  const linked = params.get("linked");
  const failed = params.get("error");
  // Linking another sign-in to an account with passkeys: one of them confirms it first.
  const confirmLink = params.get("confirm_link") as SignInProvider | null;
  const unlink = async (provider: string, subject: string) => {
    setError(undefined);
    setStepUp(false);
    try {
      await api.unlink(provider, subject);
      identities.reload();
    } catch (err) {
      setStepUp(err instanceof ApiError && err.code === "step_up_required");
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const available = (Object.keys(PROVIDER_LABEL) as SignInProvider[]).filter((p) => providers.data?.[p]);
  return (
    <section id="sign-ins">
      <h2 className="section-title">Sign-ins</h2>
      <div className="panel panel-pad">
        {linked && linked !== "already" && (
          <p className="notice" role="status">
            {PROVIDER_LABEL[linked as SignInProvider] ?? linked} is linked: you can sign in with it, and Slop knows the
            repositories it owns are yours.
          </p>
        )}
        {linked === "already" && <p className="notice">That account was already linked.</p>}
        {failed === "provider" && <p className="form-error">That sign-in didn't work. Please try again.</p>}
        {confirmLink && PROVIDER_LABEL[confirmLink] && (
          <div className="notice" role="status">
            <p>
              To link {PROVIDER_LABEL[confirmLink]}, confirm it's you with one of your passkeys.
            </p>
            <StepUpPrompt
              signInAgain={`/signin?return_to=${encodeURIComponent("/account#sign-ins")}`}
              onDone={() => window.location.assign(api.linkUrl(confirmLink))}
            />
          </div>
        )}
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
        <p className="muted small">
          Link the accounts you use: to sign in with any of them, and to publish repositories they own (GitHub, GitLab and
          Bitbucket check who owns a repository by the linked account). Linking asks you to sign in to that account, so
          nobody can link someone else's. If it already has its own Slop account, you can combine the two.
        </p>
        <div className="row-actions">
          {available.map((p) => (
            <a key={p} className="btn small" href={api.linkUrl(p)}>
              Link {PROVIDER_LABEL[p]}
            </a>
          ))}
        </div>
        {error && (
          <p className="form-error" role="alert">
            {stepUp ? (
              <StepUpPrompt
                signInAgain={`/signin?return_to=${encodeURIComponent("/account#sign-ins")}`}
                onDone={() => {
                  setStepUp(false);
                  setError(undefined);
                }}
              />
            ) : (
              error
            )}
          </p>
        )}
        <Passkeys />
      </div>
    </section>
  );
}

/** After linking a sign-in that has its own Slop account: combine the two? */
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
          Nothing to combine (it may have expired). <Link to="/account#sign-ins">Back to your account</Link>
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
      navigate("/account?linked=" + p.provider + "#sign-ins");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const keep = async () => {
    await api.combine.cancel().catch(() => undefined);
    navigate("/account#sign-ins");
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
          That {PROVIDER_LABEL[p.provider]} sign-in already has its own Slop account, <b>{p.name}</b>. You've just proved it's
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
