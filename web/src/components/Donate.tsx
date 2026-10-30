// SPDX-License-Identifier: AGPL-3.0-or-later
// The Donate button, next to Play, Install or Download on an app's store page and in the library
// (DESIGN §12, PAYMENTS §5–6). It opens the donate form right there: the donor sees what they pay
// first and largest (PAY-10), what the maker receives, SLAPorSLOP's fee and the processing addition.
// Donations give nothing back, and the app is the same for everyone.
//
// Green like Play while the viewer can donate; grey once they have ("Donated"), or with their
// monthly server donation and Cancel beside it (PAY-15: two clicks, at once). It shows on every
// public Beta or released app, never hidden: when donations are off it's quiet and says why, and
// the app's maker gets the fix in its panel (DonationsFix).
//
// On a community build or mod, a one-off donation is shared: the original's maker gets 30% as its
// own donation, paid first, then the branch's author gets the rest. Two payments, because SLAPorSLOP
// never splits one; after Stripe the donor comes back here (?shared=…&rest=…) for the second.

import { useEffect, useId, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { Link, useSearchParams } from "react-router";

import { api, ApiError, type DonateInfo, type DonationQuote } from "../api";
import { formatDate, toMinor } from "../format";
import { useLoad } from "../hooks";
import { openExternal } from "../platform";
import { solve } from "../pow";
import { useSession } from "../session";
import { DonationsFix } from "./DonationsManage";
import { ErrorState } from "./ui";
import { InlineIcon } from "./StoreIcons";

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

export function DonateButton({ slug, title, isMine = false }: { slug: string; title: string; isMine?: boolean }) {
  const { me } = useSession();
  const [params] = useSearchParams();
  const info = useLoad(() => api.donations.info(slug), [slug, me?.id]);
  // Back from Stripe (?donated=… or ?shared=…): the form opens to say thanks, or for step 2.
  const back = Boolean(params.get("donated") || params.get("shared"));
  const [open, setOpen] = useState(back);
  // The form, rather than what the viewer already gave.
  const [form, setForm] = useState(back);
  const [confirming, setConfirming] = useState(false);
  const [cancelled, setCancelled] = useState(false);
  const [cancelError, setCancelError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const panelId = useId();
  const tipId = useId();
  const button = useRef<HTMLButtonElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const confirmBox = useRef<HTMLDivElement>(null);
  const done = useRef<HTMLParagraphElement>(null);
  const focusNext = useRef<"panel" | "confirm" | "done" | null>(null);
  useEffect(() => {
    const target = { panel, confirm: confirmBox, done }[focusNext.current ?? "panel"];
    if (focusNext.current) target.current?.focus();
    focusNext.current = null;
  }, [open, form, confirming, cancelled]);

  const d = info.data;
  // Not a public app (a local build, say): nothing to donate to.
  if (info.error?.status === 404) return null;
  if (!d && !info.error) return null;
  const mine = d?.mine;
  const monthly = mine?.monthly ?? null;
  const last = mine?.last ?? null;
  const quiet = Boolean(info.error) || !d?.on || isMine;
  const note = info.error ? null : !d?.on ? "Donations off" : isMine ? "Donations on" : null;
  const tip = info.error
    ? null
    : !d?.on
      ? `Donations are off. ${d?.off_reason ?? ""}`.trim()
      : isMine
        ? "It's your own app: players donate here."
        : null;

  const close = () => {
    setOpen(false);
    setForm(false);
    button.current?.focus();
  };
  const toggle = () => {
    if (open) return close();
    setConfirming(false);
    setCancelled(false);
    // Someone who has given sees what first, with Donate again; everyone else gets the form.
    setForm(!monthly && !last);
    focusNext.current = "panel";
    setOpen(true);
  };
  const onPanelKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };
  const keep = () => {
    setConfirming(false);
    setTimeout(() => cancelButton.current?.focus());
  };
  const cancelMonthly = async () => {
    if (!monthly) return;
    setBusy(true);
    setCancelError(undefined);
    try {
      await api.donations.cancel(monthly.id);
      setConfirming(false);
      setCancelled(true);
      focusNext.current = "done";
      info.reload();
    } catch (err) {
      setCancelError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const label = monthly ? (
    <>
      <InlineIcon name="heart" /> {amount(monthly.total_minor, monthly.currency)}/month
    </>
  ) : last && !quiet ? (
    <>
      <InlineIcon name="heart" /> Donated
    </>
  ) : (
    <>
      <InlineIcon name="heart" /> Donate
    </>
  );
  const tone = monthly ? "donate-btn given" : quiet ? "donate-btn quiet" : last ? "donate-btn given" : "donate-btn slime";

  return (
    <>
      <span className="donate-wrap">
        <button
          type="button"
          ref={button}
          className={`btn ${tone}`}
          aria-expanded={open}
          aria-controls={open ? panelId : undefined}
          aria-describedby={tip ? tipId : undefined}
          aria-label={monthly ? `${amount(monthly.total_minor, monthly.currency)}/month server donation` : undefined}
          onClick={toggle}
        >
          {label}
        </button>
        {tip && (
          <span className="donate-tip" role="tooltip" id={tipId}>
            {tip}
          </span>
        )}
        {note && <span className="small muted donate-note">{note}</span>}
        {monthly && !confirming && (
          <button
            type="button"
            ref={cancelButton}
            className="btn donate-cancel"
            aria-label="Cancel server donation"
            onClick={() => {
              setOpen(false);
              setCancelError(undefined);
              focusNext.current = "confirm";
              setConfirming(true);
            }}
          >
            Cancel
          </button>
        )}
      </span>
      {confirming && monthly && d && (
        <div
          className="notice donate-confirm"
          role="group"
          aria-label="Cancel your server donation"
          ref={confirmBox}
          tabIndex={-1}
          onKeyDown={(e) => {
            if (e.key === "Escape" && !busy) {
              e.preventDefault();
              keep();
            }
          }}
        >
          <p>
            Cancel your server donation of <b>{amount(monthly.total_minor, monthly.currency)} a month</b> to {title}? It
            ends now: no more payments are taken
            {monthly.next_charge_at ? `, including the one due ${formatDate(monthly.next_charge_at)}` : ""}. What you've
            already given stays with {d.maker}.
          </p>
          {cancelError && (
            <p className="form-error" role="alert">
              {cancelError}
            </p>
          )}
          <div className="actions">
            <button type="button" className="btn small danger" disabled={busy} onClick={() => void cancelMonthly()}>
              {busy ? "Cancelling…" : "Yes, cancel it"}
            </button>
            <button type="button" className="btn small ghost" disabled={busy} onClick={keep}>
              Keep it
            </button>
          </div>
        </div>
      )}
      {cancelled && !confirming && (
        <p className="small donate-done" role="status" ref={done} tabIndex={-1}>
          Your server donation to {title} is cancelled. No more payments will be taken.
        </p>
      )}
      {open && (
        <div
          className="panel panel-pad donate-panel"
          id={panelId}
          role="dialog"
          aria-label={`Donate to ${title}`}
          ref={panel}
          tabIndex={-1}
          onKeyDown={onPanelKey}
        >
          <button type="button" className="btn small ghost donate-close" aria-label="Close" onClick={close}>
            <span aria-hidden="true">×</span>
          </button>
          {info.error ? (
            <>
              <h3>
                <InlineIcon name="heart" /> Donate
              </h3>
              <ErrorState error={info.error} onRetry={info.reload} />
            </>
          ) : !d ? null : !d.on ? (
            <section className="donate-box donate-off" aria-label={`Donations to ${title} are off`}>
              <h3>
                <InlineIcon name="heart" /> Donate
              </h3>
              <p className="small">
                <b>Donations are off.</b> {d.off_reason}
              </p>
              {isMine ? (
                <DonationsFix slug={slug} onChanged={info.reload} />
              ) : (
                <p className="muted small">Donations go straight to {d.maker} and never unlock anything.</p>
              )}
            </section>
          ) : form ? (
            <DonateForm slug={slug} title={title} d={d} monthlyAllowed={!monthly} onDone={info.reload} />
          ) : (
            <div className="donate-box donate-given">
              <h3>
                <InlineIcon name="heart" /> {monthly ? "Your server donation" : "You donated"}
              </h3>
              {monthly && (
                <p>
                  You give <b>{amount(monthly.total_minor, monthly.currency)} a month</b> for {title}'s servers
                  {monthly.started_at ? `, since ${formatDate(monthly.started_at)}` : ""}.
                  {monthly.next_charge_at && ` The next payment is on ${formatDate(monthly.next_charge_at)}.`} Cancel it any
                  time with Cancel next to this button.
                </p>
              )}
              {last && (
                <p>
                  You donated <b>{amount(last.total_minor, last.currency)}</b> on {formatDate(last.paid_at)}. It went
                  straight to {d.maker}.
                </p>
              )}
              <p className="muted small">
                Donating again is up to you: {title} is the same for everyone. Receipts are on your{" "}
                <Link to="/account/money#donations">account page</Link>.
              </p>
              <button
                type="button"
                className="btn small"
                onClick={() => {
                  focusNext.current = "panel";
                  setForm(true);
                }}
              >
                Donate again
              </button>
            </div>
          )}
        </div>
      )}
    </>
  );
}

/** The donate form itself: amounts, monthly for server donations, consent, then Stripe. */
function DonateForm({
  slug,
  title,
  d,
  monthlyAllowed,
  onDone,
}: {
  slug: string;
  title: string;
  d: DonateInfo;
  /** False while the viewer already gives monthly (one each). */
  monthlyAllowed: boolean;
  /** A donation went through: the button reloads what the viewer gave. */
  onDone: () => void;
}) {
  const { me, refresh } = useSession();
  const [params, setParams] = useSearchParams();
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
        onDone();
        setParams({}, { replace: true });
      });
  }, [donated, setParams, onDone]);

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

  const shared = d.shared && !monthly ? d.shared : null;
  const share = shared && selected !== null ? shareFor(selected, shared.percent) : null;
  useEffect(() => {
    setShareQuote(undefined);
    if (!shared || share === null) return;
    const timer = window.setTimeout(() => {
      api.donations.quote(shared.slug, share).then(setShareQuote, () => setShareQuote(undefined));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [shared?.slug, share]); // eslint-disable-line react-hooks/exhaustive-deps
  if (thanks) {
    return (
      <div className="donate-box">
        <h3>
          <InlineIcon name="heart" /> Thank you
        </h3>
        <p>Your donation goes straight to {d.maker}. Your receipt is on your <Link to="/account/money#donations">account page</Link>.</p>
      </div>
    );
  }

  const q = customQuote;
  const consentText = monthly ? (q?.consent ?? null) : null;

  const startWithCheck = async (to: string, body: StartBody) => {
    try {
      return await api.donations.start(to, body);
    } catch (err) {
      // After a few unfinished checkouts SLAPorSLOP asks the browser for a quick proof of work
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
      onDone();
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
        onDone();
        void refresh();
      }
    });
  };

  if (sharePaid) {
    return (
      <div className="donate-box">
        <h3>
          <InlineIcon name="heart" /> Step 2 of 2
        </h3>
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
    <form className="donate-box form" onSubmit={(e) => void submit(e)} aria-label={`Donate to ${title}`}>
      <h3>
        <InlineIcon name="heart" /> Donate
      </h3>
      <p className="muted small">
        Goes straight to {d.maker} through Stripe. Donations give nothing back: {title} is the same for everyone.
      </p>
      {d.servers && monthlyAllowed && (
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
          {amount(q.slop_fee, d.currency)} SLAPorSLOP fee ·{" "}
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
        {monthly && " Cancel any time, right next to the Donate button or on your account page."}
      </p>
    </form>
  );
}
