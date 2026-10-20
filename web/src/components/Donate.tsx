// The Donate box on an app's page (DESIGN §12, PAYMENTS §5–6). The donor sees what they pay
// first and largest (PAY-10), what the maker receives, Slop's fee and the processing addition.
// Donations give nothing back, and the app is the same for everyone.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";

import { api, ApiError, type DonationQuote } from "../api";
import { toMinor } from "../format";
import { useLoad } from "../hooks";
import { openExternal } from "../platform";
import { useSession } from "../session";

/** Always with cents: "$3.40". */
export function amount(minor: number, currency: string): string {
  const symbol = currency === "eur" ? "€" : currency === "gbp" ? "£" : "$";
  return `${symbol}${(minor / 100).toFixed(2)}`;
}

export function DonateBox({ slug, title }: { slug: string; title: string }) {
  const { me, refresh } = useSession();
  const [params, setParams] = useSearchParams();
  const info = useLoad(() => api.donations.info(slug), [slug, me?.id]);
  const [monthly, setMonthly] = useState(false);
  const [chosen, setChosen] = useState<number>();
  const [custom, setCustom] = useState("");
  const [customQuote, setCustomQuote] = useState<DonationQuote & { consent: string | null }>();
  const [share, setShare] = useState(false);
  const [message, setMessage] = useState("");
  const [consent, setConsent] = useState(false);
  const [adult, setAdult] = useState(false);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [thanks, setThanks] = useState(false);

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
  if (!d) return null;
  if (thanks) {
    return (
      <div className="panel panel-pad donate-box">
        <h3>♥ Thank you</h3>
        <p>Your donation goes straight to {d.maker}. Your receipt is on your <Link to="/account#donations">account page</Link>.</p>
      </div>
    );
  }
  if (!d.on) return null;

  const q = customQuote;
  const consentText = monthly ? (q?.consent ?? null) : null;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!q) return;
    setBusy(true);
    setError(undefined);
    try {
      const res = await api.donations.start(slug, {
        amount_minor: q.donation,
        monthly,
        share_name: share,
        message: share && message.trim() ? message.trim() : undefined,
        consent,
        confirm_adult: adult,
      });
      if (res.url) {
        await openExternal(res.url);
      } else {
        setThanks(true);
        void refresh();
      }
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

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
            <span className="small muted">{d.maker.split(" ")[0]} gets {amount(p.maker_receives, d.currency)}</span>
          </button>
        ))}
      </div>
      <label>
        Or your own amount for {d.maker.split(" ")[0]} ({amount(d.min_minor, d.currency)} to {amount(d.max_minor, d.currency)})
        <input inputMode="decimal" value={custom} onChange={(e) => setCustom(e.target.value)} placeholder="e.g. 7.50" />
      </label>
      {q && (
        <p className="donate-total" aria-live="polite">
          You pay <b>{amount(q.total, d.currency)}</b>
          {monthly && " every month"} · {d.maker.split(" ")[0]} receives {amount(q.maker_receives, d.currency)} ·{" "}
          {amount(q.slop_fee, d.currency)} Slop fee ·{" "}
          {q.donor_covers_fees
            ? `${amount(q.processing, d.currency)} card processing`
            : `about ${amount(q.processing, d.currency)} card processing comes out of what they receive`}
        </p>
      )}
      {!monthly && (
        <>
          <label className="check">
            <input type="checkbox" checked={share} onChange={(e) => setShare(e.target.checked)} />
            <span>Tell {d.maker.split(" ")[0]} my name (only them; donors are never shown publicly)</span>
          </label>
          {share && (
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
      {d.cant ? (
        <p className="small muted">
          {d.cant === "Sign in to donate." ? <Link to={`/signin?return_to=/app/${slug}`}>Sign in to donate.</Link> : d.cant}
        </p>
      ) : (
        <button
          className="btn slime"
          disabled={busy || !q || (monthly && !consent) || (Boolean(me) && !me!.adult_confirmed && !adult)}
        >
          {busy ? "Opening…" : q ? `Donate ${amount(q.total, d.currency)}${monthly ? " a month" : ""}` : "Choose an amount"}
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
