// Sign-in, sign-up and the account page (with the devices list, docs/SECURITY.md §4.2).

import { useState, type FormEvent } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";

import { api, ApiError } from "../api";
import { ErrorState, Loading } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { isDesktop } from "../platform";
import { useSession } from "../session";

const SIGNIN_ERRORS: Record<string, string> = {
  cancelled: "Sign-in was cancelled.",
  provider: "GitHub didn't let us sign you in. Please try again.",
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
  const providers = useLoad(() => api.providers(), []);

  if (isDesktop) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad">
          <h1>Sign in</h1>
          <p>Signing in from the desktop app is coming soon. You can browse the whole store without an account.</p>
        </div>
      </div>
    );
  }

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
        {providers.data?.github === false && (
          <p className="notice">Sign-in isn't set up on this server yet.</p>
        )}
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

export function SignUp() {
  useDocumentTitle("Create your account");
  const navigate = useNavigate();
  const { refresh } = useSession();
  const info = useLoad(() => api.signupInfo(), []);
  const [name, setName] = useState<string>();
  const [country, setCountry] = useState("");
  const [ageConfirmed, setAgeConfirmed] = useState(false);
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
      const res = await api.signup({ display_name: displayName, country_code: country, age_confirmed: ageConfirmed });
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

        {error && !error.field && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <button className="btn slime big" disabled={busy || !country || !ageConfirmed}>
          {busy ? "Creating…" : "Create account"}
        </button>
      </form>
    </div>
  );
}

export function Account() {
  useDocumentTitle("Your account");
  const navigate = useNavigate();
  const { me, signOut, canSignIn } = useSession();
  const sessions = useLoad(() => (me ? api.sessions() : Promise.resolve([])), [me?.id]);

  if (me === undefined) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad">
          <h1>Your account</h1>
          {canSignIn ? (
            <p>
              <Link to="/signin?return_to=/account">Sign in</Link> to see your account.
            </p>
          ) : (
            <p>Signing in from the desktop app is coming soon.</p>
          )}
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

      <h2 className="section-title">Where you're signed in</h2>
      {sessions.error && <ErrorState error={sessions.error} onRetry={sessions.reload} />}
      <ul className="sessions panel">
        {sessions.data?.map((s) => (
          <li key={s.id}>
            <div>
              <b>{describeAgent(s.user_agent)}</b> {s.current && <span className="badge official">THIS DEVICE</span>}
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
    </div>
  );
}

function describeAgent(ua: string | null): string {
  if (!ua) return "Unknown device";
  const browser = /Firefox\//.test(ua) ? "Firefox" : /Edg\//.test(ua) ? "Edge" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "Browser";
  const os = /Windows/.test(ua) ? "Windows" : /Mac OS/.test(ua) ? "macOS" : /Android/.test(ua) ? "Android" : /iPhone|iPad/.test(ua) ? "iOS" : /Linux/.test(ua) ? "Linux" : "";
  return os ? `${browser} on ${os}` : browser;
}
