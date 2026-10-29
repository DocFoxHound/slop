// SPDX-License-Identifier: AGPL-3.0-or-later
// The account page, laid out like an app's settings: a header with the person, a list of
// sections on the left (a row along the top on phones) and one section at a time, each at its own
// address (accountNav.ts). Old /account#<part> links go to the part in its section.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams } from "react-router";

import { api, ApiError } from "../api";
import { accountPath, accountSections, redirectForOldLink, sectionFromSlug, type AccountSection, type AccountSectionId } from "../accountNav";
import { tokenBadge } from "../agentTokens";
import { AiAgents } from "../components/AiAgents";
import { Avatar } from "../components/Avatar";
import { PasskeyNotice } from "../components/PasskeyNotice";
import { MyPeoplePageCard, useMyPeoplePage } from "../components/PeopleStatus";
import { SigningKeys } from "../components/SigningKeys";
import { ErrorState, Loading } from "../components/ui";
import {
  AccountAgreements,
  AccountData,
  AccountModeration,
  AccountMoney,
  AccountNotifications,
  AccountPicture,
} from "../components/AccountSections";
import { formatDate } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { desktop, isDesktop } from "../platform";
import { useSession } from "../session";
import { Passkeys, SignIns } from "./Auth";

/** "United States" for "US"; the code itself where the browser can't name it. */
function countryName(code: string): string {
  try {
    return new Intl.DisplayNames(["en"], { type: "region" }).of(code) ?? code;
  } catch {
    return code;
  }
}

export function Account() {
  const { section: slug } = useParams();
  const location = useLocation();
  const { me } = useSession();
  const section = sectionFromSlug(slug, isDesktop);
  useDocumentTitle(section && section.id !== "profile" ? `${section.label} · Your account` : "Your account");
  // Agreements waiting to be accepted, for the badge (the section shows them).
  const notices = useLoad(() => (me ? api.legal.notices() : Promise.resolve([])), [me?.id]);
  // AI agent tokens, for the badge when one expires within a week (the section lists them).
  const agentTokens = useLoad(() => (me ? api.agentTokens() : Promise.resolve([])), [me?.id]);

  if (me === undefined) return <Loading />;
  if (!me) {
    return (
      <div className="page narrow">
        <div className="panel panel-pad">
          <h1>Your account</h1>
          <p>
            <Link to={`/signin?return_to=${location.pathname}`}>Sign in</Link> to see your account.
          </p>
        </div>
      </div>
    );
  }
  const old = redirectForOldLink(location, isDesktop);
  if (old) return <Navigate to={old} replace />;
  if (!section) return <Navigate to="/account" replace />;

  const badges: Partial<Record<AccountSectionId, Badge>> = {};
  if (me.suspended) badges.moderation = { text: "!", label: "your account is suspended", tone: "bad" };
  else if (me.new_decisions > 0) badges.moderation = { text: String(me.new_decisions), label: `${me.new_decisions} new` };
  const toAccept = (notices.data ?? []).filter((n) => n.action === "accept").length;
  if (toAccept > 0) badges.agreements = { text: String(toAccept), label: `${toAccept} to accept` };
  const expiring = tokenBadge(agentTokens.data ?? []);
  if (expiring) badges.agents = expiring;

  return (
    <div className="page account-page">
      <AccountHeader />
      <div className="account-layout">
        <AccountNav current={section.id} badges={badges} />
        <SectionView section={section}>
          {section.id === "profile" && <ProfileSection />}
          {section.id === "security" && <SecuritySection />}
          {section.id === "device" && <ThisDevice />}
          {section.id === "agents" && (
            <>
              <AiAgents notices={notices} tokens={agentTokens} />
              <SigningKeys />
            </>
          )}
          {section.id === "notifications" && <AccountNotifications />}
          {section.id === "money" && <AccountMoney />}
          {section.id === "moderation" && <AccountModeration />}
          {section.id === "agreements" && <AccountAgreements notices={notices} />}
          {section.id === "data" && <AccountData />}
        </SectionView>
      </div>
    </div>
  );
}

/** The person: picture, name, country, their public profile, and signing out. */
function AccountHeader() {
  const { me, signOut } = useSession();
  const navigate = useNavigate();
  if (!me) return null;
  return (
    <header className="panel account-head">
      <Avatar name={me.display_name} url={me.avatar_url} />
      <div className="account-head-text">
        <h1 className="account-head-title">Your account</h1>
        <p className="account-head-name">{me.display_name}</p>
        <p className="account-head-meta muted small">
          <span>{countryName(me.country_code)}</span>
          {me.email && <span>{me.email}</span>}
        </p>
      </div>
      <div className="account-head-actions">
        <Link className="btn small" to={`/u/${me.id}`}>
          Your public profile
        </Link>
        <button type="button" className="btn small ghost danger" onClick={() => void signOut().then(() => navigate("/", { replace: true }))}>
          Sign out
        </button>
      </div>
    </header>
  );
}

interface Badge {
  /** Shown: a count, or "!". */
  text: string;
  /** Read out after the section's name. */
  label: string;
  tone?: "warn" | "bad";
}

/** The sections, grouped: a sticky list beside the content, a scrolling row on phones. */
function AccountNav({ current, badges }: { current: AccountSectionId; badges: Partial<Record<AccountSectionId, Badge>> }) {
  const list = useRef<HTMLDivElement>(null);
  const sections = accountSections(isDesktop);
  const groups = [...new Set(sections.map((s) => s.group))];
  // On phones the row scrolls sideways: keep the current section in view.
  useEffect(() => {
    const row = list.current;
    const active = row?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!row || !active || row.scrollWidth <= row.clientWidth) return;
    row.scrollLeft = active.offsetLeft - (row.clientWidth - active.offsetWidth) / 2;
  }, [current]);
  return (
    <nav className="account-nav" aria-label="Account settings">
      <div className="account-nav-scroll" ref={list}>
        {groups.map((group) => (
          <div className="account-nav-group" key={group}>
            <p className="account-nav-heading" id={`account-nav-${group.replace(/\W+/g, "-").toLowerCase()}`}>
              {group}
            </p>
            <ul aria-labelledby={`account-nav-${group.replace(/\W+/g, "-").toLowerCase()}`}>
              {sections
                .filter((s) => s.group === group)
                .map((s) => {
                  const badge = badges[s.id];
                  return (
                    <li key={s.id}>
                      <Link
                        to={accountPath(s.id)}
                        className={`account-nav-link${s.id === current ? " on" : ""}`}
                        aria-current={s.id === current ? "page" : undefined}
                      >
                        <span>{s.label}</span>
                        {badge && (
                          <span className={`account-nav-badge ${badge.tone ?? ""}`}>
                            <span aria-hidden="true">{badge.text}</span>
                            <span className="sr-only">({badge.label})</span>
                          </span>
                        )}
                      </Link>
                    </li>
                  );
                })}
            </ul>
          </div>
        ))}
      </div>
    </nav>
  );
}

/**
 * The section shown, under its heading. Moving to another section (the list, Back) puts focus on
 * its heading, so keyboard and screen reader users start there; the first page load doesn't move
 * focus, and a link to a part of the section goes to that part instead.
 */
function SectionView({ section, children }: { section: AccountSection; children: ReactNode }) {
  const heading = useRef<HTMLHeadingElement>(null);
  const location = useLocation();
  const shown = useRef(section.id);
  useEffect(() => {
    if (shown.current === section.id) return;
    shown.current = section.id;
    if (!location.hash) heading.current?.focus({ preventScroll: true });
  }, [section.id, location.hash]);
  return (
    <section className="account-content" aria-labelledby="account-section-title">
      <header className="account-section-head">
        <h2 id="account-section-title" tabIndex={-1} ref={heading}>
          {section.title ?? section.label}
        </h2>
        <p className="muted">{section.intro}</p>
      </header>
      {children}
    </section>
  );
}

/** Profile: name, email and country, the picture, and what others see. */
function ProfileSection() {
  const { me, refresh } = useSession();
  const [privacyError, setPrivacyError] = useState<string>();
  if (!me) return null;
  const savePrivacy = async (changes: Parameters<typeof api.updateMe>[0]) => {
    setPrivacyError(undefined);
    try {
      await api.updateMe(changes);
    } catch (err) {
      setPrivacyError(err instanceof ApiError ? err.message : String(err));
    }
    // Either way, the switches show what's saved.
    await refresh();
  };
  return (
    <>
      <section id="details" className="account-panel" aria-labelledby="details-title">
        <h3 className="account-panel-title" id="details-title">
          Details
        </h3>
        <div className="panel panel-pad">
          <dl className="kv">
            <dt>Name</dt>
            <dd>{me.display_name}</dd>
            <dt>Email</dt>
            <dd>{me.email ?? <span className="muted">None shared by GitHub</span>}</dd>
            <dt>Country</dt>
            <dd>
              {countryName(me.country_code)} <span className="muted">({me.country_code})</span>
            </dd>
          </dl>
        </div>
      </section>

      <AccountPicture />

      <PeoplePage />

      <section id="privacy" className="account-panel" aria-labelledby="privacy-title">
        <h3 className="account-panel-title" id="privacy-title">
          Privacy
        </h3>
        <div className="panel panel-pad switch-list">
          <label className="check">
            <input type="checkbox" checked={me.profile_public} onChange={(e) => void savePrivacy({ profile_public: e.target.checked })} />{" "}
            <span>
              <b>Show my votes and reviews on my profile</b>
              <span className="muted small" style={{ display: "block" }}>
                Off: your profile shows only your name and apps, and your reviews show as "A player".{" "}
                <Link to={`/u/${me.id}`}>See your profile</Link>
              </span>
            </span>
          </label>
          <label className="check">
            <input type="checkbox" checked={me.share_playtime} onChange={(e) => void savePrivacy({ share_playtime: e.target.checked })} />{" "}
            <span>
              <b>Share my playtime</b>
              <span className="muted small" style={{ display: "block" }}>
                Games you launch from SLAPorSLOP can count you in their "playing now" numbers, and your playtime makes your votes
                count more. SLAPorSLOP gets nothing but a launch token and a heartbeat; never what you do in the game.
              </span>
            </span>
          </label>
          <label className="check">
            <input type="checkbox" checked={me.personalize} onChange={(e) => void savePrivacy({ personalize: e.target.checked })} />{" "}
            <span>
              <b>Recommendations based on my activity</b>
              <span className="muted small" style={{ display: "block" }}>
                "For you" and Discover use your library, your Interested list, what you've played, how you voted and the apps
                you donated to. Off: everyone sees the same popular, well-rated and new apps.
              </span>
            </span>
          </label>
          {privacyError && (
            <p className="form-error" role="alert">
              {privacyError}
            </p>
          )}
          {me.gpc && (
            <p className="muted small">
              Your browser sends Global Privacy Control, asking sites not to sell or share your data. SLAPorSLOP never does, and
              because of it, recommendations based on your activity stay off unless you turn them on.
            </p>
          )}
        </div>
      </section>
    </>
  );
}

/** A creative's People page and where it stands; nothing for everyone else. */
function PeoplePage() {
  const mine = useMyPeoplePage();
  return mine.data ? <MyPeoplePageCard profile={mine.data} /> : null;
}

/** Sign-in & security: linked sign-ins, passkeys, and the sessions signed in now. */
function SecuritySection() {
  const { me } = useSession();
  const sessions = useLoad(() => (me ? api.sessions() : Promise.resolve([])), [me?.id]);
  const [sessionError, setSessionError] = useState<string>();
  const revoke = async (id: string) => {
    setSessionError(undefined);
    try {
      await api.revokeSession(id);
      sessions.reload();
    } catch (err) {
      setSessionError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <>
      <PasskeyNotice />
      <SignIns />
      <Passkeys />
      <section id="sessions" className="account-panel" aria-labelledby="sessions-title">
        <h3 className="account-panel-title" id="sessions-title">
          Where you're signed in
        </h3>
        <p className="muted small">
          Browsers, SLAPorSLOP apps and AI agents signed in to your account. Sign out any you don't recognise.
        </p>
        {sessions.error && <ErrorState error={sessions.error} onRetry={sessions.reload} />}
        {sessionError && (
          <p className="form-error" role="alert">
            {sessionError}
          </p>
        )}
        <ul className="sessions panel">
          {sessions.data?.map((s) => (
            <li key={s.id}>
              <div>
                <b>
                  {s.kind === "desktop"
                    ? (s.user_agent ?? "SLAPorSLOP app")
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
      </section>
    </>
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
      <div className="panel panel-pad switch-list">
        {notify && (
          <>
            <label className="check">
              <input type="checkbox" checked={notify.on} onChange={(e) => saveNotify({ ...notify, on: e.target.checked })} />
              <span>
                <b>Notifications for new chat messages</b>
                <span className="muted small" style={{ display: "block" }}>
                  Shown when SLAPorSLOP isn't in front. Muted chats never notify.
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
              checked={Boolean(overlay)}
              onChange={(e) => void desktop.overlaySetting(e.target.checked).then(setOverlay, () => undefined)}
            />
            <span>
              <b>In-game overlay (Shift+Tab)</b>
              <span className="muted small" style={{ display: "block" }}>
                Chats and friends over a game you started from SLAPorSLOP, when it's fullscreen or maximized. Shift+Tab works as
                usual everywhere else.
              </span>
            </span>
          </label>
        )}
      </div>
    </section>
  );
}
