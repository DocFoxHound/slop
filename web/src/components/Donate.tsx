// SPDX-License-Identifier: AGPL-3.0-or-later
// The Donate box on an app's page (DESIGN §12, PAYMENTS §5–6). The donor sees what they pay
// first and largest (PAY-10), what the maker receives, Slop's fee and the processing addition.
// Donations give nothing back, and the app is the same for everyone.
//
// It shows on every public Beta or released app's page, never hidden: when donations are off it
// says why, and the app's maker gets the fix right there (DonationsFix).
//
// On a community build or mod, a one-off donation is shared: the original's maker gets 30% as its
// own donation, paid first, then the branch's author gets the rest. Two payments, because Slop
// never splits one; after Stripe the donor comes back here (?shared=…&rest=…) for the second.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";

import { api, ApiError, type DonationQuote } from "../api";
import { toMinor } from "../format";
import { useLoad } from "../hooks";
import { openExternal } from "../platform";
import { solve } from "../pow";
import { useSession } from "../session";
import { DonationsFix } from "./DonationsManage";
import { ErrorState } from "./ui";

/** Always with cents: "$3.40". */
export function amount(minor: number, currency: string): string {
  const symbol = currency === "eur" ? "€" : currency === "gbp" ? "£" : "$";
  return `${symbol}${(minor / 100).toFixed(2)}`;
}

/** The original's share for a donation of `branch` to a branch: 30% of the two together. */
export function shareFor(branch: number, percent: number): number {
  return Math.ceil((branch * percent) / (100 - percent));
}

type StartBody = Parameters<typeof api.donations.start>[1];

export function DonateBox({ slug, title, isMine = false }: { slug: string; title: string; isMine?: boolean }) {
  const { me, refresh } = useSession();
  const [params, setParams] = useSearchParams();
  const info = useLoad(() => api.donations.info(slug), [slug, me?.id]);
  const [monthly, setMonthly] = useState(false);
  const [chosen, setChosen] = useState<number>();
  const [custom, setCustom] = useState("");
  const [customQuote, setCustomQuote] = useState<DonationQuote & { consent: string | null }>();
  const [shareName, setShareName] = useState(false);
  const [message, setMessage] = useState("");
  const [consent, setConsent] = useState(false);
  const [adult, setAdult] = useState(false);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(false);
  const [thanks, setThanks] = useState(false);
  // The original's share is paid: its donation ID and what the branch gets next.
  const [sharePaid, setSharePaid] = useState<{ id: string; rest: number }>();
  const [shareQuote, setShareQuote] = useState<DonationQuote>();

  // Back from Stripe after paying the original's share: on to the second payment.
  const sharedId = params.get("shared");
  const rest = Number(params.get("rest"));
  useEffect(() => {
    if (!sharedId || !rest) return;
    void api.donations
      .check(sharedId)
      .catch(() => undefined)
      .then(() => {
        setSharePaid({ id: sharedId, rest });
        setParams({}, { replace: true });
      });
  }, [sharedId, rest, setParams]);

  // Back from Stripe: read the payment now rather than wait for the webhook.
  const donated = params.get("donated");
  useEffect(() => {
    if (!donated) return;
    void api.donations
      .check(donated)
      .catch(() => undefined)
      .then(() => {
        setThanks(true);
        setParams({}, { replace: true });
      });
  }, [donated, setParams]);

  // The quote (and, for server donations, the exact terms) for whatever amount is chosen.
  const selected = custom ? toMinor(custom) : (chosen ?? null);
  useEffect(() => {
    setCustomQuote(undefined);
    if (selected === null) return;
    const timer = window.setTimeout(() => {
      api.donations.quote(slug, selected).then(setCustomQuote, () => setCustomQuote(undefined));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [slug, selected]);
  // New terms need a new tick.
  useEffect(() => setConsent(false), [selected, monthly]);

  const d = info.data;
  const shared = d?.shared && !monthly ? d.shared : null;
  const share = shared && selected !== null ? shareFor(selected, shared.percent) : null;
  useEffect(() => {
    setShareQuote(undefined);
    if (!shared || share === null) return;
    const timer = window.setTimeout(() => {
      api.donations.quote(shared.slug, share).then(setShareQuote, () => setShareQuote(undefined));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [shared?.slug, share]); // eslint-disable-line react-hooks/exhaustive-deps
  if (info.error) {
    return (
      <div className="panel panel-pad donate-box">
        <h3>♥ Donate</h3>
        <ErrorState error={info.error} onRetry={info.reload} />
      </div>
    );
  }
  if (!d) return null;
  if (thanks) {
    return (
      <div className="panel panel-pad donate-box">
        <h3>♥ Thank you</h3>
        <p>Your donation goes straight to {d.maker}. Your receipt is on your <Link to="/account/money#donations">account page</Link>.</p>
      </div>
    );
  }
  if (!d.on) {
    return (
      <section className="panel panel-pad donate-box donate-off" aria-label={`Donate to ${title}`}>
        <h3>♥ Donate</h3>
        <p className="small">
          <b>Donations are off.</b> {d.off_reason}
        </p>
        {isMine ? (
          <DonationsFix slug={slug} onChanged={info.reload} />
        ) : (
          <p className="muted small">Donations go straight to {d.maker} and never unlock anything.</p>
        )}
      </section>
    );
  }

  const q = customQuote;
  const consentText = monthly ? (q?.consent ?? null) : null;

  const startWithCheck = async (to: string, body: StartBody) => {
    try {
      return await api.donations.start(to, body);
    } catch (err) {
      // After a few unfinished checkouts Slop asks the browser for a quick proof of work
      // (card testing, PAYMENTS §9). It takes a second or two.
      if (!(err instanceof ApiError) || err.code !== "challenge_required") throw err;
      setChecking(true);
      const c = await api.donations.challenge();
      const nonce = await solve(c.prefix, c.difficulty);
      setChecking(false);
      return api.donations.start(to, { ...body, challenge: { id: c.id, nonce } });
    }
  };

  /** The branch's own payment, after the original's share. */
  const payBranch = async (paid: { id: string; rest: number }) => {
    const res = await startWithCheck(slug, {
      amount_minor: paid.rest,
      monthly: false,
      share_name: shareName,
      message: shareName && message.trim() ? message.trim() : undefined,
      consent: false,
      confirm_adult: adult,
      share_donation: paid.id,
    });
    if (res.url) {
      await openExternal(res.url);
    } else {
      setThanks(true);
      void refresh();
    }
  };

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(undefined);
    try {
      await work();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
      setChecking(false);
    }
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!q) return;
    const body: StartBody = {
      amount_minor: q.donation,
      monthly,
      share_name: shareName,
      message: shareName && message.trim() ? message.trim() : undefined,
      consent,
      confirm_adult: adult,
    };
    void run(async () => {
      if (shared && share !== null) {
        // First the original's share, as its own donation to its maker.
        const res = await startWithCheck(shared.slug, {
          ...body,
          amount_minor: share,
          share_for: slug,
          rest_minor: q.donation,
        });
        if (res.url) {
          await openExternal(res.url);
          return;
        }
        const paid = { id: res.id, rest: q.donation };
        setSharePaid(paid);
        await payBranch(paid);
        return;
      }
      const res = await startWithCheck(slug, body);
      if (res.url) {
        await openExternal(res.url);
      } else {
        setThanks(true);
        void refresh();
      }
    });
  };

  if (sharePaid) {
    return (
      <div className="panel panel-pad donate-box">
        <h3>♥ Step 2 of 2</h3>
        <p>
          Thank you: your share for {d.shared?.title ?? "the original"} is paid. Now your donation of{" "}
          <b>{amount(sharePaid.rest, d.currency)}</b> to {d.maker}.
        </p>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <button className="btn slime" disabled={busy} onClick={() => void run(() => payBranch(sharePaid))}>
          {checking ? "Checking your browser…" : busy ? "Opening…" : `Donate ${amount(sharePaid.rest, d.currency)} to ${d.maker}`}
        </button>
      </div>
    );
  }

  return (
    <form className="panel panel-pad donate-box form" onSubmit={(e) => void submit(e)} aria-label={`Donate to ${title}`}>
      <h3>♥ Donate</h3>
      <p className="muted small">
        Goes straight to {d.maker} through Stripe. Donations give nothing back: {title} is the same for everyone.
      </p>
      {d.servers && (
        <div className="seg small-seg" role="tablist" aria-label="How often">
          <button type="button" role="tab" aria-selected={!monthly} className={!monthly ? "on" : ""} onClick={() => setMonthly(false)}>
            Once
          </button>
          <button type="button" role="tab" aria-selected={monthly} className={monthly ? "on" : ""} onClick={() => setMonthly(true)}>
            Server donation · monthly
          </button>
        </div>
      )}
      {monthly && d.servers && <p className="small">“{d.servers.statement}”</p>}
      <div className="donate-presets" role="radiogroup" aria-label="Amount">
        {d.presets.map((p) => (
          <button
            type="button"
            key={p.donation}
            role="radio"
            aria-checked={!custom && chosen === p.donation}
            className={`donate-preset ${!custom && chosen === p.donation ? "on" : ""}`}
            onClick={() => {
              setChosen(p.donation);
              setCustom("");
            }}
          >
            <b>{amount(p.total, d.currency)}</b>
            <span className="small muted">{d.maker} gets {amount(p.maker_receives, d.currency)}</span>
          </button>
        ))}
      </div>
      <label>
        Or your own amount for {d.maker} ({amount(d.min_minor, d.currency)} to {amount(d.max_minor, d.currency)})
        <input inputMode="decimal" value={custom} onChange={(e) => setCustom(e.target.value)} placeholder="e.g. 7.50" />
      </label>
      {shared && share !== null && share < d.min_minor && (
        <p className="form-error small">
          Shared donations start at {amount(Math.ceil((d.min_minor * (100 - shared.percent)) / shared.percent), d.currency)}, so{" "}
          {shared.maker}'s share is at least {amount(d.min_minor, d.currency)}.
        </p>
      )}
      {shared && q && share !== null && share >= d.min_minor && (
        <p className="notice small" aria-live="polite">
          Shared with the original: <b>{amount(shareQuote?.total ?? share, d.currency)}</b> goes first to {shared.maker}{" "}
          for <Link to={`/app/${shared.slug}`}>{shared.title}</Link> ({shared.percent}% of your donations), then the amount
          below to {d.maker}. That's two payments, each straight to its maker.
        </p>
      )}
      {q && (
        <p className="donate-total" aria-live="polite">
          You pay <b>{amount(q.total, d.currency)}</b>
          {monthly && " every month"} · {d.maker} receives {amount(q.maker_receives, d.currency)} ·{" "}
          {amount(q.slop_fee, d.currency)} Slop fee ·{" "}
          {q.donor_covers_fees
            ? `${amount(q.processing, d.currency)} card processing`
            : `about ${amount(q.processing, d.currency)} card processing comes out of what they receive`}
        </p>
      )}
      {!monthly && (
        <>
          <label className="check">
            <input type="checkbox" checked={shareName} onChange={(e) => setShareName(e.target.checked)} />
            <span>Tell {d.maker} my name (only them; donors are never shown publicly)</span>
          </label>
          {shareName && (
            <label>
              A message for them <span className="muted">(optional)</span>
              <textarea value={message} maxLength={500} rows={2} onChange={(e) => setMessage(e.target.value)} />
            </label>
          )}
        </>
      )}
      {monthly && consentText && (
        <label className="check consent">
          <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} />
          <span>{consentText}</span>
        </label>
      )}
      {me && !me.adult_confirmed && (
        <label className="check">
          <input type="checkbox" checked={adult} onChange={(e) => setAdult(e.target.checked)} />
          <span>I'm 18 or older.</span>
        </label>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {d.cant === "Sign in to donate." ? (
        <Link className="btn slime" to={`/signin?return_to=/app/${slug}`}>
          Sign in to donate
        </Link>
      ) : d.cant ? (
        <p className="small muted">{d.cant}</p>
      ) : (
        <button
          className="btn slime"
          disabled={
            busy ||
            !q ||
            (monthly && !consent) ||
            (Boolean(me) && !me!.adult_confirmed && !adult) ||
            (shared !== null && (share === null || share < d.min_minor || !shareQuote))
          }
        >
          {checking
            ? "Checking your browser…"
            : busy
              ? "Opening…"
              : !q
                ? "Choose an amount"
                : shared
                  ? `Donate ${amount(q.total + (shareQuote?.total ?? 0), d.currency)} in two payments`
                  : `Donate ${amount(q.total, d.currency)}${monthly ? " a month" : ""}`}
        </button>
      )}
      <p className="faint small">
        {d.simulated
          ? "This server simulates payments: no money moves."
          : `Payment runs on ${d.maker}'s own Stripe account, so they see what Stripe shows them for your receipt. Donations are final; the maker can refund mistakes.`}
        {monthly && " Cancel any time from your account page."}
      </p>
    </form>
  );
}
