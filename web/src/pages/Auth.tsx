// Sign-in, sign-up and the account page (with the devices list, docs/SECURITY.md §4.2).

import { useEffect, useState, type FormEvent } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams, useSearchParams } from "react-router";

import { api, ApiError } from "../api";
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
  provider: "GitHub didn't let us sign you in. Please try again.",
  closed: "That account was closed, so its sign-in can't be used on Slop again.",
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
        {providers.data?.github === false && <p className="notice">Sign-in isn't set up on this server yet.</p>}
        <a
          className={`btn big provider ${providers.data?.github ? "" : "disabled"}`}
          href={providers.data?.github ? api.githubSignInUrl(returnTo) : undefined}
          aria-disabled={!providers.data?.github}
        >
          Continue with GitHub
        </a>
        <p className="faint small">More sign-in options (Google, Apple, Microsoft, Discord) are on the way.</p>
      </div>
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
      </div>

      <AccountNotifications />
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
              <b>{s.kind === "desktop" ? (s.user_agent ?? "Slop app") : describeAgent(s.user_agent)}</b>{" "}
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
