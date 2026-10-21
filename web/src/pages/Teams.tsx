// SPDX-License-Identifier: AGPL-3.0-or-later
// Teams (DESIGN §14): player teams with picks, game nights and shared Private Slop; official
// studio teams verified through their GitHub organization (crates/slop-api/src/teams.rs).

import { useState, type FormEvent } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { api, ApiError, type TeamCard, type TeamPage } from "../api";
import { ReportButton } from "../components/Report";
import { ErrorState, Loading, Shelf } from "../components/ui";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { useSession } from "../session";

function Card({ t }: { t: TeamCard }) {
  return (
    <Link to={`/teams/${t.slug}`} className="panel panel-pad team-card">
      <b>{t.name}</b>{" "}
      {t.kind === "official" ? (
        t.verified ? (
          <span className="badge official">✓ OFFICIAL</span>
        ) : (
          <span className="badge neutral">NOT VERIFIED YET</span>
        )
      ) : (
        <span className="badge neutral">{t.join_policy === "open" ? "OPEN" : "INVITE ONLY"}</span>
      )}
      <div className="muted small">
        {t.members} member{t.members === 1 ? "" : "s"}
        {t.about && ` · ${t.about.slice(0, 120)}`}
      </div>
    </Link>
  );
}

export function Teams() {
  useDocumentTitle("Teams");
  const { me } = useSession();
  const all = useLoad(() => api.teams.list(), []);
  const mine = useLoad(() => (me ? api.teams.mine() : Promise.resolve({ teams: [], invites: [] })), [me?.id]);
  const [creating, setCreating] = useState(false);
  return (
    <div className="page narrow">
      <div className="title-row spread">
        <h1 className="page-title">Teams</h1>
        {me && !creating && (
          <button type="button" className="btn slime" onClick={() => setCreating(true)}>
            Start a team
          </button>
        )}
      </div>
      <p className="muted">
        <b>Player teams</b> share picks, plan game nights and can see Private Slop shared with them. <b>Official teams</b> are
        studios, verified through their GitHub organization.
      </p>
      {creating && <CreateTeam onCancel={() => setCreating(false)} />}
      {(mine.data?.invites.length ?? 0) > 0 && (
        <>
          <h2 className="section-title">Invitations</h2>
          <div className="team-grid">
            {mine.data!.invites.map((t) => (
              <Card key={t.slug} t={t} />
            ))}
          </div>
        </>
      )}
      {(mine.data?.teams.length ?? 0) > 0 && (
        <>
          <h2 className="section-title">Your teams</h2>
          <div className="team-grid">
            {mine.data!.teams.map((t) => (
              <Card key={t.slug} t={t} />
            ))}
          </div>
        </>
      )}
      <h2 className="section-title">Find a team</h2>
      {all.error && <ErrorState error={all.error} onRetry={all.reload} />}
      {all.data?.length === 0 && <div className="empty-state">No open teams yet. Start one!</div>}
      <div className="team-grid">
        {all.data?.map((t) => (
          <Card key={t.slug} t={t} />
        ))}
      </div>
    </div>
  );
}

function CreateTeam({ onCancel }: { onCancel: () => void }) {
  const navigate = useNavigate();
  const [kind, setKind] = useState<"player" | "official">("player");
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [about, setAbout] = useState("");
  const [open, setOpen] = useState(false);
  const [org, setOrg] = useState("");
  const [error, setError] = useState<ApiError>();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    try {
      const res = await api.teams.create({
        slug,
        name,
        kind,
        about,
        join_policy: kind === "player" ? (open ? "open" : "invite") : undefined,
        github_org: kind === "official" ? org : undefined,
      });
      navigate(`/teams/${res.slug}`);
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
    }
  };
  const fieldError = (f: string) =>
    error?.field === f ? (
      <span className="field-error" role="alert">
        {error.message}
      </span>
    ) : null;
  return (
    <form className="panel panel-pad form" onSubmit={(e) => void submit(e)}>
      <fieldset>
        <legend>What kind?</legend>
        <label className="check">
          <input type="radio" name="kind" checked={kind === "player"} onChange={() => setKind("player")} />
          <span>
            <b>Player team</b> <span className="muted small">— friends, a club, a guild</span>
          </span>
        </label>
        <label className="check">
          <input type="radio" name="kind" checked={kind === "official"} onChange={() => setKind("official")} />
          <span>
            <b>Official studio team</b> <span className="muted small">— verified through your GitHub organization</span>
          </span>
        </label>
      </fieldset>
      <label>
        Name
        <input value={name} maxLength={60} required onChange={(e) => setName(e.target.value)} />
        {fieldError("name")}
      </label>
      <label>
        Address
        <input value={slug} maxLength={40} required onChange={(e) => setSlug(e.target.value.toLowerCase())} placeholder="night-owls" />
        <span className="hint">slop/teams/{slug || "…"}</span>
        {fieldError("slug")}
      </label>
      <label>
        About <span className="muted">(optional)</span>
        <textarea value={about} maxLength={2000} rows={2} onChange={(e) => setAbout(e.target.value)} />
      </label>
      {kind === "player" ? (
        <label className="check">
          <input type="checkbox" checked={open} onChange={(e) => setOpen(e.target.checked)} />
          <span>Anyone can find and join it (otherwise by invitation)</span>
        </label>
      ) : (
        <label>
          GitHub organization
          <input value={org} required onChange={(e) => setOrg(e.target.value)} placeholder="your-studio" />
          <span className="hint">You must be an owner of it. Next, you'll install Slop's GitHub App there so Slop can check it.</span>
          {fieldError("github_org")}
        </label>
      )}
      {error && !error.field && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="row-actions">
        <button className="btn slime">Create</button>
        <button type="button" className="btn ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

const CHECK_NAME: Record<string, string> = {
  app_installed: "Slop's GitHub App is installed",
  owner: "You own the organization",
  domain: "A verified domain",
  two_factor: "Two-factor authentication required",
  repositories: "Apps from the organization's repositories",
  business: "Donations go to a business account",
};

export function Team() {
  const { slug = "" } = useParams();
  const { me } = useSession();
  const navigate = useNavigate();
  const team = useLoad(() => api.teams.page(slug), [slug, me?.id]);
  useDocumentTitle(team.data?.name);
  const [error, setError] = useState<string>();
  if (team.error) return <ErrorState error={team.error} onRetry={team.reload} />;
  if (!team.data) return <Loading />;
  const t = team.data;
  const admin = t.role === "owner" || t.role === "admin";
  const run = async (action: () => Promise<unknown>, after?: () => void) => {
    setError(undefined);
    try {
      await action();
      if (after) after();
      else team.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <div className="page team-page">
      <div className="crumbs">
        <Link to="/teams">Teams</Link> › {t.name}
      </div>
      <div className="title-row spread">
        <div>
          <h1 className="page-title">
            {t.name}{" "}
            {t.kind === "official" &&
              (t.verified ? (
                <span className="badge official" title="Verified through its GitHub organization">
                  ✓ OFFICIAL
                </span>
              ) : (
                <span className="badge neutral">NOT VERIFIED YET</span>
              ))}
          </h1>
          <p className="muted small">
            {t.kind === "official" ? `Studio · github.com/${t.github_org}` : t.join_policy === "open" ? "Open player team" : "Invite-only player team"} ·{" "}
            {t.member_count} member{t.member_count === 1 ? "" : "s"}
          </p>
        </div>
        <div className="row-actions">
          {me && !t.role && t.invited && (
            <>
              <button type="button" className="btn slime" onClick={() => void run(() => api.teams.accept(slug))}>
                Accept the invitation
              </button>
              <button type="button" className="btn ghost" onClick={() => void run(() => api.teams.decline(slug), () => navigate("/teams"))}>
                Decline
              </button>
            </>
          )}
          {me && !t.role && !t.invited && t.kind === "player" && t.join_policy === "open" && (
            <button type="button" className="btn slime" onClick={() => void run(() => api.teams.join(slug))}>
              Join
            </button>
          )}
          {t.role && t.kind === "player" && !t.hidden && (
            // Team chat: everyone in the team, end-to-end encrypted (pages/Chat.tsx).
            isDesktop ? (
              <button
                type="button"
                className="btn"
                onClick={() => void run(async () => navigate(`/chat/${await desktop.chat<string>({ action: "start_team", team: slug })}`), () => undefined)}
              >
                💬 Team chat
              </button>
            ) : (
              <Link className="btn" to="/chat">
                💬 Team chat in the app
              </Link>
            )
          )}
          {t.role && (
            <button type="button" className="btn ghost" onClick={() => void run(() => api.teams.leave(slug), () => navigate("/teams"))}>
              Leave
            </button>
          )}
          {!t.role && <ReportButton kind="team" subject={slug} label="this team" />}
        </div>
      </div>
      {t.hidden && <p className="notice">This team is hidden by a moderation decision. See your account page.</p>}
      {t.about && <p className="pre">{t.about}</p>}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      {t.kind === "official" && t.role && <Verification t={t} onChange={team.reload} />}
      {t.kind === "official" && t.apps.length > 0 && (
        <>
          <h2 className="section-title">Their apps</h2>
          <Shelf apps={t.apps} empty="" />
        </>
      )}

      {t.kind === "player" && t.role && (
        <div className="two-col">
          <div>
            <h2 className="section-title">Game nights</h2>
            <GameNights t={t} onChange={team.reload} />
            <h2 className="section-title">Picks</h2>
            <Picks t={t} onChange={team.reload} isAdmin={admin} />
            {t.shared.length > 0 && (
              <>
                <h2 className="section-title">Private Slop shared with the team</h2>
                <Shelf apps={t.shared} empty="" />
              </>
            )}
          </div>
          <aside className="side-col">
            <Members t={t} admin={admin} onChange={team.reload} run={run} />
          </aside>
        </div>
      )}
      {(t.kind === "official" || !t.role) && t.members.length > 0 && (
        <Members t={t} admin={admin} onChange={team.reload} run={run} />
      )}
    </div>
  );
}

function Members({
  t,
  admin,
  onChange,
  run,
}: {
  t: TeamPage;
  admin: boolean;
  onChange: () => void;
  run: (a: () => Promise<unknown>) => Promise<void>;
}) {
  const { me } = useSession();
  const [who, setWho] = useState("");
  return (
    <div className="panel panel-pad">
      <h3>Members</h3>
      <ul className="plain team-members">
        {t.members.map((m) => (
          <li key={m.user_id}>
            <Link to={`/u/${m.user_id}`}>{m.name}</Link> {m.role !== "member" && <span className="muted small">· {m.role}</span>}
            {t.role === "owner" && m.user_id !== me?.id && (
              <select
                aria-label={`${m.name}'s role`}
                value={m.role}
                onChange={(e) => void run(() => api.teams.setRole(t.slug, m.user_id, e.target.value))}
              >
                <option value="member">member</option>
                <option value="admin">admin</option>
                <option value="owner">owner</option>
              </select>
            )}
            {admin && m.role !== "owner" && m.user_id !== me?.id && (
              <button type="button" className="link-btn small muted" onClick={() => void run(() => api.teams.removeMember(t.slug, m.user_id))}>
                Remove
              </button>
            )}
          </li>
        ))}
      </ul>
      {admin && (
        <form
          className="row-actions"
          onSubmit={(e) => {
            e.preventDefault();
            void run(() => api.teams.invite(t.slug, who)).then(() => {
              setWho("");
              onChange();
            });
          }}
        >
          <input aria-label="Their profile link" placeholder="Invite: paste their profile link" value={who} onChange={(e) => setWho(e.target.value)} />
          <button className="btn small" disabled={!who.trim()}>
            Invite
          </button>
        </form>
      )}
    </div>
  );
}

function Picks({ t, onChange, isAdmin }: { t: TeamPage; onChange: () => void; isAdmin: boolean }) {
  const [app, setApp] = useState("");
  const [note, setNote] = useState("");
  const [error, setError] = useState<string>();
  const add = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    try {
      await api.teams.pick(t.slug, app.trim().replace(/^.*\/app\//, "").replace(/\/$/, ""), note);
      setApp("");
      setNote("");
      onChange();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <>
      {t.picks.length === 0 ? (
        <p className="muted small">No picks yet. Share an app the team should try.</p>
      ) : (
        <ul className="plain team-picks">
          {t.picks.map((p) => (
            <li key={p.id} className="panel panel-pad">
              <Link to={`/app/${p.slug}`}>
                <b>{p.title}</b>
              </Link>{" "}
              <span className="muted small">· picked by {p.by}</span>
              {p.note && <p className="small">“{p.note}”</p>}
              {isAdmin && (
                <button type="button" className="link-btn small muted" onClick={() => void api.teams.unpick(t.slug, p.id).then(onChange)}>
                  Remove
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      <form className="form compact-form" onSubmit={(e) => void add(e)}>
        <div className="row-actions">
          <input aria-label="App link or address" placeholder="App link or address" value={app} onChange={(e) => setApp(e.target.value)} />
          <input aria-label="Why (optional)" placeholder="Why? (optional)" value={note} maxLength={280} onChange={(e) => setNote(e.target.value)} />
          <button className="btn small" disabled={!app.trim()}>
            Pick it
          </button>
        </div>
        {error && <p className="form-error">{error}</p>}
      </form>
    </>
  );
}

function GameNights({ t, onChange }: { t: TeamPage; onChange: () => void }) {
  const [title, setTitle] = useState("");
  const [when, setWhen] = useState("");
  const [app, setApp] = useState("");
  const [error, setError] = useState<string>();
  const add = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    try {
      await api.teams.addEvent(t.slug, {
        title,
        starts_at: new Date(when).toISOString(),
        app_slug: app.trim() ? app.trim().replace(/^.*\/app\//, "").replace(/\/$/, "") : undefined,
      });
      setTitle("");
      setWhen("");
      setApp("");
      onChange();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <>
      {t.events.length === 0 ? (
        <p className="muted small">Nothing planned.</p>
      ) : (
        <ul className="plain team-events">
          {t.events.map((e) => (
            <li key={e.id} className="panel panel-pad">
              <b>{e.title}</b>{" "}
              <span className="small">
                {new Date(e.starts_at).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })}
              </span>
              <div className="muted small">
                {e.app_slug && (
                  <>
                    <Link to={`/app/${e.app_slug}`}>{e.app_title}</Link> ·{" "}
                  </>
                )}
                set by {e.by}
                {e.mine && (
                  <>
                    {" "}
                    ·{" "}
                    <button type="button" className="link-btn small muted" onClick={() => void api.teams.removeEvent(t.slug, e.id).then(onChange)}>
                      Cancel it
                    </button>
                  </>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
      <form className="form compact-form" onSubmit={(e) => void add(e)}>
        <div className="row-actions">
          <input aria-label="Game night title" placeholder="Game night" value={title} maxLength={100} onChange={(e) => setTitle(e.target.value)} />
          <input aria-label="When" type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} />
          <input aria-label="App (optional)" placeholder="App (optional)" value={app} onChange={(e) => setApp(e.target.value)} />
          <button className="btn small" disabled={!title.trim() || !when}>
            Plan it
          </button>
        </div>
        {error && <p className="form-error">{error}</p>}
      </form>
    </>
  );
}

function Verification({ t, onChange }: { t: TeamPage; onChange: () => void }) {
  const [domain, setDomain] = useState(t.dns?.domain ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const verify = async () => {
    setBusy(true);
    setError(undefined);
    try {
      if ((t.dns?.domain ?? "") !== domain) await api.teams.update(t.slug, { dns_domain: domain });
      await api.teams.verify(t.slug);
      onChange();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="panel panel-pad verification">
      <h2>Verification</h2>
      <p className="muted small">
        Slop checks these through its GitHub App on github.com/{t.github_org}, and again every day. While any fails, the
        badge is off.
        {t.checked_at && ` Last checked ${formatDate(t.checked_at)}.`}
      </p>
      {t.checks.length > 0 && (
        <ul className="plain checks">
          {t.checks.map((c) => (
            <li key={c.check}>
              <span aria-hidden="true">{c.ok ? "✓" : "✗"}</span> <b>{CHECK_NAME[c.check] ?? c.check}</b>{" "}
              <span className={`small ${c.ok ? "muted" : "warn"}`}>
                {c.ok ? "" : "Not yet: "}
                {c.detail}
              </span>
            </li>
          ))}
        </ul>
      )}
      <details>
        <summary className="small">No verified domain on GitHub? Prove one with DNS</summary>
        <label className="small">
          Your studio's domain
          <input value={domain} onChange={(e) => setDomain(e.target.value)} placeholder="studio.example" />
        </label>
        {t.dns && (
          <p className="small">
            Add a TXT record named <code>_slop-team.{domain || "<your domain>"}</code> with the value <code>{t.dns.value}</code>.
          </p>
        )}
      </details>
      <div className="row-actions">
        <button type="button" className="btn slime" disabled={busy} onClick={() => void verify()}>
          {busy ? "Checking…" : "Check now"}
        </button>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
