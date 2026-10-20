// A read-only token for a private repository outside GitHub (crates/slop-api/src/repo_tokens.rs).
// Slop keeps it encrypted, only sends it to the repository's own host, and never shows it again.

import { useState } from "react";

import { api, ApiError, type RepoTokenInput, type RepoTokenKind } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { ErrorState } from "./ui";

const HOSTS: Record<RepoTokenKind, { label: string; how: string }> = {
  gitlab: {
    label: "GitLab",
    how: "In the project: Settings → Access tokens. Role Reporter, scopes read_api and read_repository.",
  },
  bitbucket: {
    label: "Bitbucket",
    how: "In the repository: Repository settings → Security → Access tokens. Scope Repositories: Read.",
  },
  gitea: {
    label: "Gitea, Forgejo or Codeberg",
    how: "Your settings → Applications → Generate new token, with repository access Read only.",
  },
};

export function guessKind(repoUrl: string): RepoTokenKind {
  if (/^https?:\/\/(www\.)?bitbucket\.org\//i.test(repoUrl)) return "bitbucket";
  if (/gitlab/i.test(repoUrl)) return "gitlab";
  return "gitea";
}

export function isGithub(repoUrl: string): boolean {
  return /^https:\/\/github\.com\//i.test(repoUrl.trim());
}

/** The host and token fields, for the publish form and the Manage page. */
export function TokenFields({ value, onChange }: { value: RepoTokenInput; onChange: (v: RepoTokenInput) => void }) {
  return (
    <div className="token-fields">
      <label>
        Host{" "}
        <select value={value.kind} onChange={(e) => onChange({ ...value, kind: e.target.value as RepoTokenKind })}>
          {(Object.keys(HOSTS) as RepoTokenKind[]).map((k) => (
            <option key={k} value={k}>
              {HOSTS[k].label}
            </option>
          ))}
        </select>
      </label>
      <label>
        Read-only token{" "}
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={value.token}
          onChange={(e) => onChange({ ...value, token: e.target.value })}
        />
      </label>
      <p className="muted small">
        {HOSTS[value.kind].how} Slop keeps it encrypted, only uses it with this repository's host, and never shows it again.
        People you share the app with download straight from your host through short-lived links; hosts that can't give those
        can't be shared through Slop.
      </p>
    </div>
  );
}

/** The Manage page's panel: whether a token is saved, and replacing or removing it. */
export function RepoAccessPanel({ slug, repoUrl }: { slug: string; repoUrl: string }) {
  const status = useLoad(() => api.repoToken.status(slug), [slug]);
  const [value, setValue] = useState<RepoTokenInput>({ kind: guessKind(repoUrl), token: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  if (status.error) return <ErrorState error={status.error} onRetry={status.reload} />;
  const s = status.data;
  if (!s || s.github) return null;
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      setValue((v) => ({ ...v, token: "" }));
      status.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="panel panel-pad">
      <h2>Private repository access</h2>
      {!s.available ? (
        <p className="muted">This server can't keep repository tokens yet.</p>
      ) : (
        <>
          <p className="muted small">
            {s.saved
              ? `A ${HOSTS[s.kind ?? "gitea"].label} token is saved (since ${formatDate(s.saved_at!)}${
                  s.last_used_at ? `, last used ${formatDate(s.last_used_at)}` : ""
                }). Replace it before it expires.`
              : "Slop needs a read-only token to fetch releases from this private repository and share them."}
          </p>
          <TokenFields value={value} onChange={setValue} />
          <div className="row-actions">
            <button className="btn small slime" disabled={busy || !value.token.trim()} onClick={() => void run(() => api.repoToken.save(slug, value))}>
              {busy ? "Checking…" : s.saved ? "Replace token" : "Save token"}
            </button>
            {s.saved && (
              <button className="btn small ghost danger" disabled={busy} onClick={() => void run(() => api.repoToken.forget(slug))}>
                Remove token
              </button>
            )}
          </div>
        </>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
