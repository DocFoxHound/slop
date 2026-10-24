// SPDX-License-Identifier: AGPL-3.0-or-later
// Fixing what someone wrote (crates/slop-api/src/admin_tools.rs): their display name and profile
// picture, their People page and the teams they run, and making them staff or taking them off staff (staff.rs). Every
// fix needs a reason the person is told, and is in the audit log with the old words.

import { useState, type FormEvent } from "react";
import { Link } from "react-router";

import { api, type AdminProfile, type StaffRole } from "../../api";
import { Avatar } from "../../components/Avatar";
import { ErrorState, Loading } from "../../components/ui";
import { useLoad } from "../../hooks";
import { ActionMessages, ReasonButton, useAction } from "./shared";

/** `onSaved` lets the account's record show a new display name. */
export function ProfileFix({ id, onSaved }: { id: string; onSaved?: () => void }) {
  const profile = useLoad(() => api.admin.profile(id), [id]);
  if (profile.error) return <ErrorState error={profile.error} onRetry={profile.reload} />;
  if (!profile.data) return <Loading />;
  return (
    <>
      <ProfileForm
        id={id}
        profile={profile.data}
        onSaved={() => {
          profile.reload();
          onSaved?.();
        }}
      />
      {profile.data.teams.map((t) => (
        <TeamForm key={t.slug} team={t} onSaved={profile.reload} />
      ))}
    </>
  );
}

function ProfileForm({ id, profile, onSaved }: { id: string; profile: AdminProfile; onSaved: () => void }) {
  const c = profile.creative;
  const [name, setName] = useState(profile.display_name);
  const [peopleName, setPeopleName] = useState(c?.name ?? "");
  const [headline, setHeadline] = useState(c?.headline ?? "");
  const [about, setAbout] = useState(c?.about_md ?? "");
  const [location, setLocation] = useState(c?.location ?? "");
  const [removeLinks, setRemoveLinks] = useState(false);
  const [removePicture, setRemovePicture] = useState(false);
  const [reason, setReason] = useState("");
  const [saved, setSaved] = useState<string>();
  const action = useAction(onSaved);
  const changed =
    name !== profile.display_name ||
    (c && (peopleName !== c.name || headline !== c.headline || about !== c.about_md || location !== c.location)) ||
    removeLinks ||
    removePicture;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    setSaved(undefined);
    void action.run("profile", async () => {
      const res = await api.admin.editProfile(id, {
        display_name: name !== profile.display_name ? name : undefined,
        creative: c
          ? {
              name: peopleName !== c.name ? peopleName : undefined,
              headline: headline !== c.headline ? headline : undefined,
              about_md: about !== c.about_md ? about : undefined,
              location: location !== c.location ? location : undefined,
              remove_links: removeLinks || undefined,
            }
          : undefined,
        remove_picture: removePicture || undefined,
        reason: reason.trim(),
      });
      setSaved(`Changed ${res.changed.join(", ")}. They've been told why.`);
      setReason("");
      setRemoveLinks(false);
      setRemovePicture(false);
    });
  };
  return (
    <details className="panel panel-pad">
      <summary>
        <b>Fix their profile</b>{" "}
        <span className="muted small">display name, picture{c ? " and People page" : ""}</span>
      </summary>
      <form className="form" onSubmit={submit}>
        <label>
          Display name
          <input value={name} maxLength={40} required onChange={(e) => setName(e.target.value)} />
        </label>
        {profile.avatar_url ? (
          <div className="picture-current">
            <Avatar name={profile.display_name} url={profile.avatar_url} />
            <label className="check small">
              <input type="checkbox" checked={removePicture} onChange={(e) => setRemovePicture(e.target.checked)} />
              <span>Remove their picture (they're back to the letter, and can choose a picture again)</span>
            </label>
          </div>
        ) : (
          <p className="muted small">No picture: they show the letter.</p>
        )}
        {c ? (
          <>
            <p className="muted small">
              People page: <Link to={`/people/${c.handle}`}>@{c.handle}</Link>
              {c.hidden_reason && <> · hidden ({c.hidden_reason})</>}
            </p>
            <label>
              Name on their People page
              <input value={peopleName} maxLength={60} required onChange={(e) => setPeopleName(e.target.value)} />
            </label>
            <label>
              Headline
              <input value={headline} maxLength={140} onChange={(e) => setHeadline(e.target.value)} />
            </label>
            <label>
              About (Markdown)
              <textarea value={about} rows={5} maxLength={6000} onChange={(e) => setAbout(e.target.value)} />
            </label>
            <label>
              Location
              <input value={location} maxLength={60} onChange={(e) => setLocation(e.target.value)} />
            </label>
            {c.links.length > 0 && (
              <fieldset>
                <legend>Links</legend>
                <ul className="plain small">
                  {c.links.map((l) => (
                    <li key={l.url}>
                      {l.label}: <code>{l.url}</code>
                    </li>
                  ))}
                </ul>
                <label className="check small">
                  <input type="checkbox" checked={removeLinks} onChange={(e) => setRemoveLinks(e.target.checked)} />
                  <span>Remove every link</span>
                </label>
              </fieldset>
            )}
          </>
        ) : (
          <p className="muted small">They have no People page.</p>
        )}
        <label>
          Why, as they'll read it
          <textarea value={reason} rows={2} maxLength={2000} required onChange={(e) => setReason(e.target.value)} />
        </label>
        <div className="row-actions">
          <button className="btn small" disabled={!changed || !reason.trim() || action.busy !== undefined}>
            Save the fix
          </button>
        </div>
        {saved && (
          <p className="notice" role="status">
            {saved}
          </p>
        )}
        <ActionMessages {...action} />
      </form>
    </details>
  );
}

function TeamForm({ team, onSaved }: { team: AdminProfile["teams"][number]; onSaved: () => void }) {
  const [name, setName] = useState(team.name);
  const [about, setAbout] = useState(team.about);
  const [reason, setReason] = useState("");
  const action = useAction(onSaved);
  const changed = name !== team.name || about !== team.about;
  return (
    <details className="panel panel-pad">
      <summary>
        <b>Fix their team: {team.name}</b>{" "}
        <span className="muted small">
          {team.kind} team · they're its {team.role}
          {team.hidden_reason && " · hidden"}
        </span>
      </summary>
      <form
        className="form"
        onSubmit={(e) => {
          e.preventDefault();
          void action.run("team", async () => {
            await api.admin.editTeam(team.slug, {
              name: name !== team.name ? name : undefined,
              about: about !== team.about ? about : undefined,
              reason: reason.trim(),
            });
            setReason("");
          });
        }}
      >
        <p className="muted small">
          <Link to={`/teams/${team.slug}`}>/teams/{team.slug}</Link>
        </p>
        <label>
          Team name
          <input value={name} maxLength={60} required onChange={(e) => setName(e.target.value)} />
        </label>
        <label>
          About
          <textarea value={about} rows={3} maxLength={2000} onChange={(e) => setAbout(e.target.value)} />
        </label>
        <label>
          Why, as the team's owners will read it
          <textarea value={reason} rows={2} maxLength={2000} required onChange={(e) => setReason(e.target.value)} />
        </label>
        <div className="row-actions">
          <button className="btn small" disabled={!changed || !reason.trim() || action.busy !== undefined}>
            Save the fix
          </button>
        </div>
        <ActionMessages {...action} />
      </form>
    </details>
  );
}

const ROLES: StaffRole[] = ["reviewer", "support", "moderator", "finance", "security", "owner"];

/** Making someone staff, or taking them off staff, from their account page. */
export function StaffControls({ id, name, isStaff, onDone }: { id: string; name: string; isStaff: boolean; onDone: () => void }) {
  const staff = useLoad(() => api.admin.staff(), []);
  const [role, setRole] = useState<StaffRole>("reviewer");
  const action = useAction(() => {
    staff.reload();
    onDone();
  });
  const mine = staff.data?.my_roles ?? [];
  if (!mine.includes("owner") && !mine.includes("security")) return null;
  const member = staff.data?.staff.find((m) => m.id === id);
  return (
    <section className="panel panel-pad">
      <h3>Staff</h3>
      {isStaff ? (
        <p>
          {name} is staff{member && member.roles.length > 0 ? <>: {member.roles.join(", ")}</> : " with no role (can only look)"}.
          Change single roles under <Link to="/admin?tab=staff">Staff</Link>.
        </p>
      ) : (
        <p className="muted">Not staff.</p>
      )}
      <div className="row-actions">
        <form
          className="reason-form"
          onSubmit={(e) => {
            e.preventDefault();
            void action.run("grant", () => api.admin.changeRole(id, role, true));
          }}
        >
          <select aria-label="Role to give" value={role} onChange={(e) => setRole(e.target.value as StaffRole)}>
            {ROLES.filter((r) => r !== "owner" || mine.includes("owner")).map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <button className="btn small" disabled={action.busy !== undefined || member?.roles.includes(role)}>
            {isStaff ? "Give this role" : "Make staff"}
          </button>
        </form>
        {isStaff && (
          <ReasonButton
            label="Remove from staff"
            placeholder="Why, for the record (10+ characters): every role and API token ends"
            danger
            disabled={action.busy !== undefined}
            onSubmit={(reason) => void action.run("remove", () => api.admin.removeStaff(id, reason))}
          />
        )}
      </div>
      <p className="muted small">
        Staff changes are sensitive: you may be asked to confirm with your passkey, and they wait for a second person
        when Slop requires two. When an AI agent asks for one, it always waits for a person to approve it here.
      </p>
      <ActionMessages {...action} />
    </section>
  );
}
