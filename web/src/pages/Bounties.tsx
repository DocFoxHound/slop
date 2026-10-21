// SPDX-License-Identifier: AGPL-3.0-or-later
// Fix bounties (crates/slop-api/src/bounties.rs): players pledge toward a fix; whoever delivers
// it in an approved release is paid straight from each backer's saved card, less Slop's 5%.

import { useEffect, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { api, ApiError, type Bounty, type BountyDetail } from "../api";
import { ReportButton } from "../components/Report";
import { ErrorState, Loading } from "../components/ui";
import { formatDate, money, toMinor } from "../format";
import { useDocumentTitle, useLoad } from "../hooks";
import { useSession } from "../session";

const STATUS: Record<Bounty["status"], string> = {
  open: "OPEN",
  claimed: "DELIVERY WAITING",
  paying: "PAYING",
  paid: "PAID",
  expired: "EXPIRED",
  hidden: "HIDDEN",
};

function BountyRow({ b, showApp }: { b: Bounty; showApp?: boolean }) {
  return (
    <li>
      <span className="badge">{STATUS[b.status]}</span> <Link to={`/bounties/${b.id}`}>{b.title}</Link>
      {showApp && (
        <>
          {" "}
          for <Link to={`/app/${b.app_slug}`}>{b.app_title}</Link>
        </>
      )}
      <div className="muted small">
        {money(b.pledged_minor, b.currency)} from {b.backers} backer{b.backers === 1 ? "" : "s"} · open until{" "}
        {formatDate(b.expires_at)}
      </div>
    </li>
  );
}

/** Open bounties across the store: paid work for builders. */
export function BountiesPage() {
  useDocumentTitle("Fix bounties");
  const list = useLoad(() => api.bounties.all(), []);
  return (
    <div className="page narrow">
      <h1 className="page-title">Fix bounties</h1>
      <p className="muted">
        Players pledge toward fixes and features they want. Whoever delivers one in an approved release, the app's
        maker or anyone with a community build, is paid straight from the backers' cards, less Slop's 5%. Nothing is
        charged until a delivery is confirmed.
      </p>
      {list.error && <ErrorState error={list.error} onRetry={list.reload} />}
      {list.data?.length === 0 && <div className="empty-state">No open bounties yet. Open one on any app's page.</div>}
      <ul className="offer-list">{list.data?.map((b) => <BountyRow key={b.id} b={b} showApp />)}</ul>
    </div>
  );
}

/** On an app's page: its bounties, and opening one. */
export function AppBounties({ slug, title }: { slug: string; title: string }) {
  const { me } = useSession();
  const list = useLoad(() => api.bounties.forApp(slug), [slug]);
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState({ title: "", body: "", currency: "usd" });
  const [error, setError] = useState<string>();
  const submit = async () => {
    setError(undefined);
    try {
      const b = await api.bounties.open(slug, form);
      window.location.assign(`/bounties/${b.id}`);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't open the bounty.");
    }
  };
  const rows = list.data ?? [];
  return (
    <section className="panel panel-pad" id="bounties" aria-labelledby="bounties-title">
      <h2 id="bounties-title">Bounties</h2>
      <p className="muted small">Want something fixed or added to {title}? Open a bounty and pledge toward it.</p>
      {rows.length > 0 && (
        <ul className="offer-list">
          {rows.map((b) => (
            <BountyRow key={b.id} b={b} />
          ))}
        </ul>
      )}
      {me && !open && (
        <button type="button" className="btn small" onClick={() => setOpen(true)}>
          Open a bounty
        </button>
      )}
      {open && (
        <form
          className="form"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <label>
            What you want, in a line
            <input value={form.title} maxLength={100} required onChange={(e) => setForm({ ...form, title: e.target.value })} />
          </label>
          <label>
            Details: what's wrong or missing, and what "done" looks like
            <textarea value={form.body} maxLength={2000} rows={4} required onChange={(e) => setForm({ ...form, body: e.target.value })} />
          </label>
          <label>
            Currency
            <select value={form.currency} onChange={(e) => setForm({ ...form, currency: e.target.value })}>
              <option value="usd">US dollars</option>
              <option value="eur">Euros</option>
              <option value="gbp">Pounds</option>
            </select>
          </label>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <div className="row-actions">
            <button className="btn small slime" disabled={!form.title.trim() || !form.body.trim()}>
              Open it
            </button>
            <button type="button" className="btn small ghost" onClick={() => setOpen(false)}>
              Cancel
            </button>
          </div>
        </form>
      )}
    </section>
  );
}

/** One bounty: pledge, deliver, confirm or vote. */
export function BountyPage() {
  const { id = "" } = useParams();
  const [params] = useSearchParams();
  const { me } = useSession();
  const bounty = useLoad(() => api.bounties.get(id), [id, me?.id]);
  const mine = useLoad(() => (me ? api.myApps() : Promise.resolve([])), [me?.id]);
  const [error, setError] = useState<string>();
  const [amount, setAmount] = useState("10");
  const [agreed, setAgreed] = useState(false);
  const [claimApp, setClaimApp] = useState("");
  const [note, setNote] = useState("");
  useDocumentTitle(bounty.data ? bounty.data.title : "Bounty");
  // Back from Stripe Checkout: check the card was saved.
  useEffect(() => {
    if (params.get("pledged") && me) void api.bounties.checkPledge(id).then(() => bounty.reload());
  }, [id, me?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  if (bounty.error) return <div className="page"><ErrorState error={bounty.error} onRetry={bounty.reload} /></div>;
  if (!bounty.data) return <Loading />;
  const b: BountyDetail = bounty.data;
  const run = async (action: () => Promise<unknown>) => {
    setError(undefined);
    try {
      await action();
      bounty.reload();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Something went wrong. Try again.");
    }
  };
  const pledge = async () => {
    const minor = toMinor(amount);
    if (minor === null) return setError("Enter an amount, like 10 or 12.50.");
    await run(async () => {
      const started = await api.bounties.pledge(id, minor);
      if (started.url) window.location.assign(started.url);
    });
  };
  const pending = b.claims.find((c) => c.status === "pending");
  const deliverable = (mine.data ?? []).filter((a) => a.slug === b.app_slug || a.branch_of === b.app_slug);
  const takingPledges = b.status === "open" || b.status === "claimed";
  const pledged = b.my_pledge && ["saving", "active"].includes(b.my_pledge.status);

  return (
    <div className="page narrow">
      <div className="crumbs">
        <Link to="/bounties">Bounties</Link> › <Link to={`/app/${b.app_slug}`}>{b.app_title}</Link>
      </div>
      <h1 className="page-title">{b.title}</h1>
      <p>
        <span className="badge">{STATUS[b.status]}</span>{" "}
        <b>{money(b.pledged_minor, b.currency)}</b> from {b.backers} backer{b.backers === 1 ? "" : "s"} · open until{" "}
        {formatDate(b.expires_at)}
      </p>
      <p className="pre">{b.body}</p>
      <ReportButton kind="bounty" subject={b.id} label="this bounty" />
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}

      <section className="panel panel-pad" aria-labelledby="pledge-title">
        <h2 id="pledge-title">Your pledge</h2>
        {b.my_pledge && (
          <p>
            {money(b.my_pledge.amount_minor, b.currency)}:{" "}
            {{
              saving: "waiting for your card at Stripe",
              active: "your card is saved; charged only if a delivery is confirmed",
              due: "being charged",
              paid: "paid to the builder",
              failed: `your card was refused${b.my_pledge.failure ? ` (${b.my_pledge.failure})` : ""}`,
              withdrawn: "withdrawn",
              released: "released: nothing was charged",
            }[b.my_pledge.status] ?? b.my_pledge.status}
          </p>
        )}
        {pledged && takingPledges && (
          <button type="button" className="btn small ghost" onClick={() => void run(() => api.bounties.withdraw(id))}>
            Withdraw my pledge
          </button>
        )}
        {me && takingPledges && !pledged && b.payments !== "off" && (
          <form
            className="form"
            onSubmit={(e) => {
              e.preventDefault();
              void pledge();
            }}
          >
            <label>
              Amount ({b.currency.toUpperCase()}, 1 to 500)
              <input value={amount} inputMode="decimal" onChange={(e) => setAmount(e.target.value)} />
            </label>
            <label className="check">
              <input type="checkbox" checked={agreed} onChange={(e) => setAgreed(e.target.checked)} /> Save my card with
              Stripe and charge this amount only if a delivery is confirmed before {formatDate(b.expires_at)}. It's paid
              to whoever delivers it, less Slop's 5%. I can withdraw until then.
            </label>
            <button className="btn slime" disabled={!agreed}>
              Pledge
            </button>
          </form>
        )}
        {!me && <p className="muted">Sign in to pledge.</p>}
        {b.payments === "off" && <p className="muted">Payments aren't set up on this server yet.</p>}
      </section>

      <section className="panel panel-pad" aria-labelledby="claims-title">
        <h2 id="claims-title">Deliveries</h2>
        {b.claims.length === 0 && <p className="muted">Nobody has delivered it yet.</p>}
        <ul className="offer-list">
          {b.claims.map((c) => (
            <li key={c.id}>
              <span className="badge">{c.status.toUpperCase()}</span> {c.builder} in{" "}
              <Link to={`/app/${c.app_slug}`}>{c.app_title}</Link> {c.version}{" "}
              <span className="faint small">{formatDate(c.created_at)}</span>
              <p className="pre small">{c.note}</p>
              {c.status === "pending" && (c.accept_minor > 0 || c.reject_minor > 0) && (
                <p className="small">
                  Backers so far: {money(c.accept_minor, b.currency)} yes, {money(c.reject_minor, b.currency)} no
                </p>
              )}
            </li>
          ))}
        </ul>
        {pending && b.is_app_maker && (
          <div className="row-actions">
            <button type="button" className="btn small slime" onClick={() => void run(() => api.bounties.answer(id, pending.id, "accepted"))}>
              It's delivered: pay the builder
            </button>
            <button type="button" className="btn small ghost" onClick={() => void run(() => api.bounties.answer(id, pending.id, "declined"))}>
              Not delivered
            </button>
          </div>
        )}
        {pending && b.can_vote && (
          <div className="row-actions" role="group" aria-label="Was it delivered?">
            <button type="button" className={`btn small ${b.my_vote === true ? "slime" : "ghost"}`} aria-pressed={b.my_vote === true} onClick={() => void run(() => api.bounties.vote(id, pending.id, true))}>
              Yes, it's delivered
            </button>
            <button type="button" className={`btn small ${b.my_vote === false ? "slime" : "ghost"}`} aria-pressed={b.my_vote === false} onClick={() => void run(() => api.bounties.vote(id, pending.id, false))}>
              No
            </button>
          </div>
        )}
        {b.status === "open" && deliverable.length > 0 && (
          <form
            className="form"
            onSubmit={(e) => {
              e.preventDefault();
              void run(() => api.bounties.claim(id, claimApp || deliverable[0].slug, note));
            }}
          >
            <h3>Delivered it?</h3>
            <label>
              In which of your apps
              <select value={claimApp || deliverable[0].slug} onChange={(e) => setClaimApp(e.target.value)}>
                {deliverable.map((a) => (
                  <option key={a.slug} value={a.slug}>
                    {a.title}
                  </option>
                ))}
              </select>
            </label>
            <label>
              What you did, for the backers
              <textarea value={note} maxLength={2000} rows={3} required onChange={(e) => setNote(e.target.value)} />
            </label>
            <p className="faint small">
              Its newest approved release is the delivery. You're paid to the Stripe account you receive donations on.
            </p>
            <button className="btn small slime" disabled={!note.trim()}>
              Claim it
            </button>
          </form>
        )}
      </section>
    </div>
  );
}
