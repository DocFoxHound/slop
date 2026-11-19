// SPDX-License-Identifier: AGPL-3.0-or-later
// App of the Month for staff (crates/slop-api/src/awards.rs): this month's nominations with their
// votes (three each, never for an app you make), nominating an app, the owner's form to confirm
// up to three winners, and the winners with their prizes. The owner pays each prize by hand, as a
// donation to the app through its Donate button ("Pay prize" opens it with the amount filled in),
// then marks it paid. Everything is in the audit log.

import { useState } from "react";
import { Link } from "react-router";

import { api, type AdminAwards, type AwardCurrency } from "../../api";
import { awardAmount, monthLabel } from "../../components/Awards";
import { ErrorState, Loading } from "../../components/ui";
import { formatDate, toMinor } from "../../format";
import { useLoad } from "../../hooks";
import { ActionMessages, useAction } from "./shared";

/** "cozy-farm" from an address or a link to the app. */
const slugOf = (text: string) => text.trim().replace(/^.*\/app\//, "").replace(/[/?#].*$/, "");

const CURRENCIES: AwardCurrency[] = ["usd", "eur", "gbp"];

export function AwardsTab() {
  const [month, setMonth] = useState<string>();
  const data = useLoad(() => api.admin.awards(month), [month]);
  const action = useAction(data.reload);
  if (data.error) return <ErrorState error={data.error} onRetry={data.reload} />;
  if (!data.data) return <Loading />;
  const d = data.data;
  const votesLeft = d.max_votes - d.my_votes;
  return (
    <>
      <section>
        <h2 className="section-title">App of the Month</h2>
        <p className="muted small">
          Staff nominate public apps in the store (Beta or Released) and vote: {d.max_votes} votes each a month, never for
          an app you make. The owner confirms up to {d.max_winners} winners. Each gets a published award that the owner
          pays by hand as a donation to the app. Shown on the app's page, its maker's profile and{" "}
          <Link to="/awards">/awards</Link>.
        </p>
        <label className="small">
          Month{" "}
          <select value={d.month} onChange={(e) => setMonth(e.target.value)}>
            {d.months.map((m) => (
              <option key={m} value={m}>
                {monthLabel(m)}
              </option>
            ))}
          </select>
        </label>
        <ActionMessages error={action.error} notice={action.notice} />
      </section>

      <section>
        <h3>Nominations · {monthLabel(d.month)}</h3>
        {d.current ? (
          <p className="small" role="status">
            You have {votesLeft} of {d.max_votes} votes left this month.
          </p>
        ) : (
          <p className="small muted">Voting was open during {monthLabel(d.month)}.</p>
        )}
        {d.nominations.length === 0 ? (
          <div className="empty-state">No nominations for {monthLabel(d.month)}.</div>
        ) : (
          <ul className="award-nominations">
            {d.nominations.map((n) => {
              const canVote = d.current && !n.mine && n.eligible && (n.my_vote || votesLeft > 0);
              return (
                <li key={n.slug} className="panel panel-pad">
                  <div className="title-row">
                    <Link to={`/app/${n.slug}`}>
                      <b>{n.title}</b>
                    </Link>
                    {!n.eligible && <span className="badge warn">NOT IN THE STORE</span>}
                    {n.mine && <span className="badge neutral">YOU MAKE IT</span>}
                    <span className="small">
                      {n.votes} {n.votes === 1 ? "vote" : "votes"}
                      {n.voters.length > 0 && <span className="muted"> ({n.voters.join(", ")})</span>}
                    </span>
                    {d.current && (
                      <button
                        type="button"
                        className={`btn small ${n.my_vote ? "slime" : ""}`}
                        aria-pressed={n.my_vote}
                        disabled={!canVote || action.busy !== undefined}
                        onClick={() => void action.run(`vote-${n.slug}`, () => api.admin.voteAward(n.slug, !n.my_vote))}
                      >
                        {n.my_vote ? `Voted for ${n.title}` : `Vote for ${n.title}`}
                      </button>
                    )}
                  </div>
                  <p className="small muted">
                    Nominated by {n.nominated_by ?? "a former staff member"} on {formatDate(n.created_at)}
                    {n.note && <>: “{n.note}”</>}
                  </p>
                </li>
              );
            })}
          </ul>
        )}
        {d.current && <NominateForm busy={action.busy !== undefined} run={action.run} />}
      </section>

      {d.can_confirm && (
        <ConfirmForm
          key={`${d.month}:${d.winners.map((w) => `${w.slug}/${w.amount_minor}/${w.currency}`).join(",")}`}
          d={d}
          busy={action.busy !== undefined}
          run={action.run}
        />
      )}

      <section>
        <h3>Winners · {monthLabel(d.month)}</h3>
        {d.winners.length === 0 ? (
          <div className="empty-state">No winners confirmed for {monthLabel(d.month)} yet.</div>
        ) : (
          <ol className="award-winners-admin">
            {d.winners.map((w) => {
              const sameCurrency = w.donation_currency === w.currency;
              const payLink = `/app/${w.slug}?donate=${sameCurrency ? (w.amount_minor / 100).toFixed(2) : ""}`;
              return (
                <li key={w.slug} className="panel panel-pad">
                  <div className="title-row">
                    <Link to={`/app/${w.slug}`}>
                      <b>{w.title}</b>
                    </Link>
                    <span>{awardAmount(w.amount_minor, w.currency)} award</span>
                    {w.prize_paid_at ? (
                      <span className="badge official">PAID {formatDate(w.prize_paid_at)}</span>
                    ) : (
                      <span className="badge warn">NOT PAID YET</span>
                    )}
                  </div>
                  {w.blurb && <p className="small">{w.blurb}</p>}
                  <p className="small muted">
                    Confirmed by {w.confirmed_by ?? "a former staff member"} on {formatDate(w.confirmed_at)}.{" "}
                    {w.accepts_donations
                      ? "It takes donations right now."
                      : "It doesn't take donations right now: its maker was told to keep them on."}
                    {w.donation_currency && !sameCurrency && (
                      <> Its Donate button takes {w.donation_currency.toUpperCase()}: give the same value in that currency.</>
                    )}
                  </p>
                  {d.can_confirm && (
                    <div className="row-actions">
                      {!w.prize_paid_at && (
                        <Link
                          className={`btn small ${w.accepts_donations ? "slime" : ""}`}
                          to={payLink}
                          aria-label={`Pay prize: donate ${awardAmount(w.amount_minor, w.currency)} to ${w.title}`}
                        >
                          Pay prize
                        </Link>
                      )}
                      <button
                        type="button"
                        className="btn small"
                        disabled={action.busy !== undefined}
                        onClick={() =>
                          void action.run(`paid-${w.slug}`, () => api.admin.markPrizePaid(d.month, w.slug, !w.prize_paid_at))
                        }
                      >
                        {w.prize_paid_at ? `Mark ${w.title}'s prize not paid` : `Mark ${w.title}'s prize paid`}
                      </button>
                    </div>
                  )}
                </li>
              );
            })}
          </ol>
        )}
      </section>
    </>
  );
}

type Run = (key: string, action: () => Promise<unknown>) => Promise<void>;

function NominateForm({ busy, run }: { busy: boolean; run: Run }) {
  const [slug, setSlug] = useState("");
  const [note, setNote] = useState("");
  return (
    <form
      className="reason-form"
      onSubmit={(e) => {
        e.preventDefault();
        const s = slugOf(slug);
        if (!s) return;
        void run("nominate", async () => {
          await api.admin.nominateAward(s, note.trim());
          setSlug("");
          setNote("");
        });
      }}
    >
      <input
        aria-label="App address to nominate"
        value={slug}
        placeholder="App address or link"
        onChange={(e) => setSlug(e.target.value)}
      />
      <input
        aria-label="Why (for staff, optional)"
        value={note}
        maxLength={500}
        placeholder="Why, for the other staff (optional)"
        onChange={(e) => setNote(e.target.value)}
      />
      <button className="btn small" disabled={busy || !slugOf(slug)}>
        Nominate
      </button>
    </form>
  );
}

type WinnerPick = { slug: string; amount: string; currency: AwardCurrency; blurb: string };

/** The owner's form: up to three nominated apps, each with its award (default $25) and a line. */
function ConfirmForm({ d, busy, run }: { d: AdminAwards; busy: boolean; run: Run }) {
  const [picks, setPicks] = useState<WinnerPick[]>(() =>
    d.winners.map((w) => ({
      slug: w.slug,
      amount: (w.amount_minor / 100).toFixed(2),
      currency: w.currency,
      blurb: w.blurb,
    })),
  );
  const fixed = d.winners.some((w) => w.prize_paid_at);
  const set = (slug: string, change: Partial<WinnerPick>) =>
    setPicks(picks.map((p) => (p.slug === slug ? { ...p, ...change } : p)));
  const bad = picks.some((p) => toMinor(p.amount) === null);
  const title = (slug: string) => d.nominations.find((n) => n.slug === slug)?.title ?? slug;
  return (
    <section>
      <h3>Confirm winners · {monthLabel(d.month)}</h3>
      {fixed ? (
        <p className="small muted">A prize for this month is marked paid, so its winners are fixed.</p>
      ) : (
        <form
          className="form"
          aria-label={`Confirm winners for ${monthLabel(d.month)}`}
          onSubmit={(e) => {
            e.preventDefault();
            void run("confirm", () =>
              api.admin.confirmAwards(
                d.month,
                picks.map((p) => ({
                  slug: p.slug,
                  amount_minor: toMinor(p.amount) ?? undefined,
                  currency: p.currency,
                  blurb: p.blurb.trim(),
                })),
              ),
            );
          }}
        >
          <p className="small muted">
            Pick up to {d.max_winners}, in order. Saving replaces this month's winners (until a prize is marked paid),
            and each new winner's maker is told the award and that it comes as a donation from SLAPorSLOP.
          </p>
          <fieldset>
            <legend>Winners</legend>
            {d.nominations.map((n) => {
              const on = picks.some((p) => p.slug === n.slug);
              return (
                <label key={n.slug} className="check">
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={!on && (picks.length >= d.max_winners || !n.eligible || n.mine)}
                    onChange={(e) =>
                      setPicks(
                        e.target.checked
                          ? [...picks, { slug: n.slug, amount: "25.00", currency: "usd", blurb: "" }]
                          : picks.filter((p) => p.slug !== n.slug),
                      )
                    }
                  />
                  <span>
                    {n.title} ({n.votes} {n.votes === 1 ? "vote" : "votes"})
                  </span>
                </label>
              );
            })}
          </fieldset>
          {picks.map((p, i) => (
            <fieldset key={p.slug}>
              <legend>
                {i + 1}. {title(p.slug)}
              </legend>
              <label>
                Award
                <input inputMode="decimal" value={p.amount} onChange={(e) => set(p.slug, { amount: e.target.value })} />
              </label>
              <label>
                Currency
                <select value={p.currency} onChange={(e) => set(p.slug, { currency: e.target.value as AwardCurrency })}>
                  {CURRENCIES.map((c) => (
                    <option key={c} value={c}>
                      {c.toUpperCase()}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Why it won <span className="muted">(optional, shown publicly)</span>
                <input value={p.blurb} maxLength={160} onChange={(e) => set(p.slug, { blurb: e.target.value })} />
              </label>
            </fieldset>
          ))}
          {bad && <p className="form-error small">Give each award as an amount, e.g. 25.00.</p>}
          <button className="btn small slime" disabled={busy || bad || (picks.length === 0 && d.winners.length === 0)}>
            {picks.length === 0 ? "Clear this month's winners" : "Confirm winners"}
          </button>
        </form>
      )}
    </section>
  );
}
