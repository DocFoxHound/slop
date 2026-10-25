// SPDX-License-Identifier: AGPL-3.0-or-later
// Where a creative's own People page is (people/mod.rs `PageStatus`), said the same way on their
// page, the editor, the People directory and their account: what's happening and what's next.
// Only portfolios are reviewed; the page's own text shows as saved once the page is live.

import type { ReactNode } from "react";
import { Link } from "react-router";

import { api, ApiError, type MyProfile, type PageState, type PageStatus } from "../api";
import { formatDate } from "../format";
import { useLoad } from "../hooks";
import { useSession } from "../session";

/** What the editor tells the page it opens after saving (router state), for its message. */
export type PeopleSaved = "created" | "saved";

/** Each state's badge: words, never colour alone (A11Y-1). */
export const PAGE_STATE: Record<PageState, { label: string; tone: string }> = {
  draft: { label: "DRAFT", tone: "neutral" },
  waiting: { label: "WAITING FOR REVIEW", tone: "soon" },
  turned_down: { label: "CHANGES NEEDED", tone: "bad" },
  live: { label: "LIVE", tone: "official" },
  hidden: { label: "HIDDEN", tone: "bad" },
};

export function PageStateBadge({ status }: { status: PageStatus }) {
  const s = PAGE_STATE[status.state];
  return <span className={`badge ${s.tone}`}>{s.label}</span>;
}

/** One line: where the page is, for cards and lists. */
export function pageStateSummary(status: PageStatus): string {
  switch (status.state) {
    case "draft":
      return "Only you can see it. Next: add your portfolio, which Slop reviews before your page goes public.";
    case "waiting":
      return "Your portfolio is waiting for Slop's review. Only you can see your page until it's approved.";
    case "turned_down":
      return "Your portfolio wasn't approved. Fix it and check it again; only you can see your page until one is approved.";
    case "live":
      return status.update_waiting
        ? "Everyone can see it. Your new portfolio is waiting for review."
        : status.reason
          ? "Everyone can see it. Your newest portfolio wasn't approved; the one before still shows."
          : "Everyone can see it on People.";
    case "hidden":
      return status.suspended ? "Hidden while your account is suspended." : "Hidden by a moderation decision. Only you can see it.";
  }
}

/**
 * The creative's own status, explained: what's happening, what happens next and what to do.
 * `inEditor`: shown above the editor itself, so it points at the panels below instead of linking.
 */
export function PageStatusDetails({
  status,
  inEditor = false,
  notHireable = false,
}: {
  status: PageStatus;
  inEditor?: boolean;
  /** Live and taking work, but Stripe isn't set up, so nobody can hire them yet. */
  notHireable?: boolean;
}) {
  const portfolio = inEditor ? null : (
    <Link className="btn small slime" to="/people/me#portfolio">
      {status.state === "turned_down" ? "Fix your portfolio" : "Add your portfolio"}
    </Link>
  );
  const lines: ReactNode[] = [];
  let title: string;
  let action: ReactNode = null;
  switch (status.state) {
    case "draft":
      title = "Draft: only you can see your page";
      lines.push(
        <>
          Next, add your portfolio: your shop window, showing makers who might hire you your style and your best work. It lives in a
          public GitHub repository of your own. {inEditor ? "The Portfolio section below" : "Your page's editor"} shows how to set one up, with
          ready-made templates and a line to hand an AI assistant. A person at Slop reviews every new portfolio, and your page goes public
          on People once it's approved. You'll get a notification either way.
        </>,
      );
      if (status.problem)
        lines.push(
          <>
            The last time Slop read your portfolio, something was wrong:{" "}
            <span className="pre-line">{status.problem}</span>
          </>,
        );
      action = portfolio;
      break;
    case "waiting":
      title = "Waiting for Slop's review";
      lines.push(
        <>
          A person at Slop is checking your portfolio{status.waiting_since ? ` (sent ${formatDate(status.waiting_since)})` : ""}. Only you
          can see your page until it's approved; then everyone can find it on People.
        </>,
        <>You'll get a notification when it's approved or turned down. You can keep editing your page meanwhile.</>,
      );
      break;
    case "turned_down":
      title = "Changes needed: your portfolio wasn't approved";
      lines.push(
        <>
          The reviewer said: <q>{status.reason}</q>
        </>,
        <>
          Fix it in your portfolio's repository, then choose <b>Check my portfolio now</b>
          {inEditor ? " (Portfolio, below)" : " in your page's editor"}: the new portfolio goes back for review. Only you can see your page
          until one is approved.
        </>,
      );
      action = portfolio;
      break;
    case "live":
      title = "Live: everyone can see your page";
      if (status.update_waiting)
        lines.push(
          <>Your new portfolio is waiting for Slop's review; your page shows the approved one until then. You'll get a notification.</>,
        );
      else if (status.reason)
        lines.push(
          <>
            Your newest portfolio wasn't approved: <q>{status.reason}</q> Your page keeps showing the one approved before.
          </>,
        );
      if (notHireable)
        lines.push(<>Makers can't hire you yet: set up payments under {inEditor ? "Get paid, below" : <Link to="/people/me">Get paid</Link>}.</>);
      break;
    case "hidden":
      title = "Hidden: only you can see your page";
      lines.push(
        status.suspended ? (
          <>Your account is suspended, so your page is hidden.</>
        ) : (
          <>Slop hid your page: {status.reason ?? "it broke Slop's rules"}.</>
        ),
        <>
          The decision says why and how to appeal: <Link to="/account#decisions">see it in your account</Link>.
        </>,
      );
      break;
  }
  return (
    <div className="people-status-details">
      <p className="people-status-title">
        <PageStateBadge status={status} /> <b>{title}</b>
      </p>
      {lines.map((l, i) => (
        <p key={i}>{l}</p>
      ))}
      {action && <div className="row-actions">{action}</div>}
    </div>
  );
}

/** The signed-in person's People page: null when they haven't made one; undefined while loading or signed out. */
export function useMyPeoplePage() {
  const { me } = useSession();
  return useLoad<MyProfile | null | undefined>(
    () =>
      me
        ? api.people.mine().catch((err: unknown) => {
            if (err instanceof ApiError && err.status === 404) return null;
            throw err;
          })
        : Promise.resolve(undefined),
    [me?.id],
  );
}

/** "Your People page": its address, status and the way to edit it (People directory, account). */
export function MyPeoplePageCard({ profile, inAccount = false }: { profile: MyProfile; inAccount?: boolean }) {
  const body = (
    <>
      <p className="people-mine-who small">
        <b>{profile.name}</b> <span className="muted">/people/{profile.handle}</span> <PageStateBadge status={profile.status} />
      </p>
      <p className="small">{pageStateSummary(profile.status)}</p>
      <div className="row-actions">
        <Link className="btn small slime" to={`/people/${profile.handle}`}>
          View your page
        </Link>
        <Link className="btn small" to="/people/me">
          Edit
        </Link>
      </div>
    </>
  );
  if (inAccount)
    return (
      <section id="people-page" className="account-panel" aria-labelledby="people-page-title">
        <h3 className="account-panel-title" id="people-page-title">
          Your People page
        </h3>
        <div className="panel panel-pad people-mine">{body}</div>
      </section>
    );
  return (
    <section className="panel panel-pad people-mine" aria-labelledby="people-mine-title">
      <h2 id="people-mine-title">Your People page</h2>
      {body}
    </section>
  );
}
