// SPDX-License-Identifier: AGPL-3.0-or-later
// Parts of the account page (pages/Account.tsx) for moderation, money, agreements and the user's
// own data (DESIGN §4, §16): their picture, decisions about them with the statement of reasons and
// an appeal, the reports they sent, donations and fee statements, the agreements waiting for them,
// notifications, downloading their data and closing the account.

import { useEffect, useState, type FormEvent } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";

import {
  api,
  ApiError,
  PROVIDER_LABEL,
  type CounterNotice,
  type DonationAccount,
  type FeeStatement,
  type LegalNotice,
  type ModerationDecision,
  type MyPicture,
  type MyReport,
  type SignInProvider,
  type VatTreatment,
} from "../api";
import { formatDate } from "../format";
import { useLoad, type Loaded } from "../hooks";
import { isDesktop, openExternal, saveFile } from "../platform";
import { disablePush, enablePush, pushState, type PushState } from "../push";
import { useSession } from "../session";
import { Avatar } from "./Avatar";
import { AcceptDocument, useTermsGate } from "./Terms";
import { ErrorState } from "./ui";

/** The Content Policy section a rule title points to (headings get ids from their text). */
function ruleLink(title: string): string {
  const slug = title
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");
  return `/legal/content#${slug}`;
}

/** What keeps a sign-in's picture current, or why there isn't one. */
function pictureHint(c: MyPicture["choices"][number]): string {
  const label = PROVIDER_LABEL[c.provider] ?? c.provider;
  if (c.updates === "never") return `${label} doesn't share a picture with Slop.`;
  if (!c.available) {
    return c.can_refresh
      ? `Slop gets this picture when you sign in with ${label}: press Refresh picture to fetch it now (a quick trip to ${label} and back).`
      : `Slop gets this picture when you sign in with ${label}.`;
  }
  return c.updates === "daily"
    ? "Kept up to date: Slop checks it every day."
    : `Updated each time you sign in with ${label}.`;
}

/** Account → Picture: a linked sign-in's picture, or the letter (crates/slop-api/src/avatars.rs). */
export function AccountPicture() {
  const { me, refresh } = useSession();
  const loaded = useLoad(() => api.picture(), [me?.id]);
  const [saved, setSaved] = useState<MyPicture>();
  // The choice being saved, shown at once; if it fails, the saved one shows again.
  const [pending, setPending] = useState<string>();
  const [error, setError] = useState<string>();
  const [params] = useSearchParams();
  if (!me) return null;
  const p = saved ?? loaded.data;
  const current = pending ?? (p?.provider ? `${p.provider}:${p.subject}` : "letter");
  const choose = async (provider: SignInProvider | null, subject?: string) => {
    setPending(provider ? `${provider}:${subject}` : "letter");
    setError(undefined);
    try {
      setSaved(await api.setPicture(provider, subject));
      // The menu's picture too.
      await refresh();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setPending(undefined);
    }
  };
  return (
    <section id="picture" className="account-panel" aria-labelledby="picture-title">
      <h3 className="account-panel-title" id="picture-title">
        Picture
      </h3>
      <div className="panel panel-pad">
        <div className="picture-current">
          <Avatar name={me.display_name} url={p ? p.avatar_url : me.avatar_url} />
          <p className="muted small">
            Shown next to your name wherever people see it on Slop: your profile, friends lists, chats, teams and
            People.
          </p>
        </div>
        {loaded.error && <ErrorState error={loaded.error} onRetry={loaded.reload} />}
        {params.get("picture") === "refresh" && !params.get("error") && (
          <p className="notice" role="status">
            Signed in again: Slop has your newest picture from there.
          </p>
        )}
        <fieldset className="picture-choices" disabled={pending !== undefined || !p} aria-busy={pending !== undefined}>
          <legend>Use</legend>
          <label className="check">
            <input
              type="radio"
              name="picture"
              checked={current === "letter"}
              onChange={() => void choose(null)}
            />{" "}
            <span>
              <b>The letter</b>
              <span className="muted small" style={{ display: "block" }}>
                The first letter of your name on a coloured square.
              </span>
            </span>
          </label>
          {p?.choices.map((c) => (
            <div className="picture-choice" key={`${c.provider}:${c.subject}`}>
              <label className="check">
                <input
                  type="radio"
                  name="picture"
                  checked={current === `${c.provider}:${c.subject}`}
                  disabled={!c.available}
                  onChange={() => void choose(c.provider, c.subject)}
                />{" "}
                <span>
                  <b>Your {PROVIDER_LABEL[c.provider] ?? c.provider} picture</b>{" "}
                  {c.login && <span className="mono">{c.login}</span>}
                  <span className="muted small" style={{ display: "block" }}>
                    {pictureHint(c)}
                  </span>
                </span>
              </label>
              {c.can_refresh && (
                <a className="btn small ghost" href={api.linkUrl(c.provider, "/account?picture=refresh#picture")}>
                  Refresh picture
                </a>
              )}
            </div>
          ))}
        </fieldset>
        {p && p.choices.length === 0 && (
          <p className="muted small">
            Link a sign-in under <Link to="/account/security#sign-ins">Sign-in &amp; security</Link> to use its picture.
          </p>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <p className="muted small">
          Slop keeps its own small copy and shows that, so people who see your picture never contact GitHub, Google or
          the others.
        </p>
      </div>
    </section>
  );
}

/** Account → Moderation: decisions about the user, and the reports they sent. */
export function AccountModeration() {
  const { refresh } = useSession();
  const decisions = useLoad(() => api.decisions.mine(), []);
  const reports = useLoad(() => api.reports.mine(), []);
  // Opening the list marks them seen; the notice on every page goes away.
  useEffect(() => {
    if (decisions.data?.some((d) => d.new)) void refresh();
  }, [decisions.data, refresh]);
  const none = decisions.data?.length === 0 && reports.data?.length === 0;
  return (
    <>
      {decisions.error && <ErrorState error={decisions.error} onRetry={decisions.reload} />}
      {decisions.data && decisions.data.length > 0 && (
        <section id="decisions" className="account-panel" aria-labelledby="decisions-title">
          <h3 className="account-panel-title" id="decisions-title">
            Decisions about your content or account
          </h3>
          <p className="muted small">
            Each one says what we did, why, and which rule it was under. You can appeal each one once, within six months; a
            person looks at it again.
          </p>
          {decisions.data.map((d) => (
            <Decision key={d.id} d={d} onChange={decisions.reload} />
          ))}
        </section>
      )}
      {reports.error && <ErrorState error={reports.error} onRetry={reports.reload} />}
      {reports.data && reports.data.length > 0 && <AccountReports reports={reports.data} />}
      {none && (
        <p className="empty-state account-empty">
          Nothing here: Slop hasn't made any decisions about your content or account, and you haven't reported anything.
          Use ⚑ Report on an app, review or profile that breaks the rules.
        </p>
      )}
    </>
  );
}

function Decision({ d, onChange }: { d: ModerationDecision; onChange: () => void }) {
  const [appealing, setAppealing] = useState(false);
  return (
    <article className={`panel panel-pad decision${d.new ? " is-new" : ""}`} aria-labelledby={`decision-${d.id}`}>
      <div className="decision-head">
        <h4 id={`decision-${d.id}`}>
          {d.action_label}: {d.subject_label}
        </h4>
        {d.new && <span className="badge warn">NEW</span>}
        {d.reversed_at && <span className="badge official">REVERSED</span>}
      </div>
      <p className="muted small">
        {d.reference} · {formatDate(d.created_at)} · {d.automated ? "made with automated tools" : "decided by a person, not automated tools"}
      </p>
      <dl className="kv">
        <dt>What happened</dt>
        <dd>{d.facts}</dd>
        <dt>The rule</dt>
        <dd>
          <Link to={ruleLink(d.rule_title)}>Content Policy: {d.rule_title}</Link>
        </dd>
        {d.law && (
          <>
            <dt>The law</dt>
            <dd>{d.law}</dd>
          </>
        )}
      </dl>
      {d.appeal_status === "pending" && (
        <p className="notice">
          You appealed on {formatDate(d.appealed_at!)}. A person will look at it again and tell you here.
        </p>
      )}
      {(d.appeal_status === "upheld" || d.appeal_status === "reversed") && (
        <p className="notice">
          <b>{d.appeal_status === "reversed" ? "Your appeal succeeded: the decision is reversed." : "Your appeal was looked at, and the decision stands."}</b>{" "}
          {d.appeal_reason}
        </p>
      )}
      {d.can_appeal &&
        (appealing ? (
          <AppealForm d={d} onDone={onChange} onCancel={() => setAppealing(false)} />
        ) : (
          <p>
            <button type="button" className="btn small" onClick={() => setAppealing(true)}>
              Appeal this decision
            </button>{" "}
            <span className="muted small">until {formatDate(d.appeal_until)}</span>
          </p>
        ))}
    </article>
  );
}

const EMPTY_COUNTER: CounterNotice = {
  name: "",
  address: "",
  phone: "",
  mistake_under_penalty: false,
  consent_to_jurisdiction: false,
  signature: "",
};

function AppealForm({ d, onDone, onCancel }: { d: ModerationDecision; onDone: () => void; onCancel: () => void }) {
  const copyright = d.rule === "copyright";
  const [text, setText] = useState("");
  const [counter, setCounter] = useState<CounterNotice>(EMPTY_COUNTER);
  const [error, setError] = useState<ApiError>();
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.decisions.appeal(d.id, text, copyright ? counter : undefined);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)));
      setBusy(false);
    }
  };
  const fieldError = (field: string) =>
    error?.field === field ? (
      <span className="field-error" role="alert">
        {error.message}
      </span>
    ) : null;
  // Errors about a field shown here go next to it; any other goes at the end.
  const inline = [
    "appeal",
    ...(copyright
      ? ["counter_notice.name", "counter_notice.address", "counter_notice.phone", "counter_notice.statements", "counter_notice.signature"]
      : []),
  ];
  return (
    <form className="form appeal-form" onSubmit={(e) => void submit(e)}>
      <label>
        Why should we look again?
        <textarea value={text} rows={4} maxLength={4000} required onChange={(e) => setText(e.target.value)} />
        <span className="hint">Explain the context, or what you've changed. You can appeal once.</span>
        {fieldError("appeal")}
      </label>
      {copyright && (
        <fieldset>
          <legend>Counter-notice</legend>
          <p className="muted small">
            An appeal against a copyright decision is a counter-notice. We pass these details to the person who reported it;
            if they don't start a court case within 10 to 14 working days, we restore it.
          </p>
          <label>
            Your full name
            <input value={counter.name} maxLength={100} required onChange={(e) => setCounter({ ...counter, name: e.target.value })} />
            {fieldError("counter_notice.name")}
          </label>
          <label>
            Your postal address
            <input value={counter.address} maxLength={300} required onChange={(e) => setCounter({ ...counter, address: e.target.value })} />
            {fieldError("counter_notice.address")}
          </label>
          <label>
            Your phone number
            <input value={counter.phone} maxLength={40} required onChange={(e) => setCounter({ ...counter, phone: e.target.value })} />
            {fieldError("counter_notice.phone")}
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={counter.mistake_under_penalty}
              onChange={(e) => setCounter({ ...counter, mistake_under_penalty: e.target.checked })}
            />
            <span>Under penalty of perjury, I believe in good faith it was removed by mistake or misidentification.</span>
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={counter.consent_to_jurisdiction}
              onChange={(e) => setCounter({ ...counter, consent_to_jurisdiction: e.target.checked })}
            />
            <span>
              I consent to the jurisdiction of the US federal district court for my address (or, outside the US, any district
              where Slop may be found), and will accept service from the person who reported it.
            </span>
          </label>
          {fieldError("counter_notice.statements")}
          <label>
            Your full name, as your signature
            <input value={counter.signature} maxLength={100} required onChange={(e) => setCounter({ ...counter, signature: e.target.value })} />
            {fieldError("counter_notice.signature")}
          </label>
        </fieldset>
      )}
      {error && !(error.field && inline.includes(error.field)) && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="row-actions">
        <button className="btn slime" disabled={busy || !text.trim()}>
          {busy ? "Sending…" : "Send appeal"}
        </button>
        <button type="button" className="btn ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

const REPORT_STATUS = { open: "Waiting for review", actioned: "Acted on", dismissed: "No action" } as const;

function AccountReports({ reports }: { reports: MyReport[] }) {
  return (
    <section id="reports" className="account-panel" aria-labelledby="reports-title">
      <h3 className="account-panel-title" id="reports-title">
        Your reports
      </h3>
      <ul className="panel report-list">
        {reports.map((r) => (
          <li key={r.reference}>
            <div>
              <b>{r.subject_label}</b> <span className="muted small">· {r.reason_label}</span>
              <div className="muted small">
                {r.reference} · sent {formatDate(r.created_at)}
                {r.decided_at && <> · answered {formatDate(r.decided_at)}</>}
              </div>
              {r.outcome && <p className="small">{r.outcome}</p>}
            </div>
            <span className={`badge ${r.status === "actioned" ? "official" : "neutral"}`}>{REPORT_STATUS[r.status].toUpperCase()}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Account → Agreements. `notices` is loaded by the page, which counts the ones to accept. */
export function AccountAgreements({ notices }: { notices: Loaded<LegalNotice[]> }) {
  const [accepting, setAccepting] = useState<LegalNotice>();
  return (
    <section id="agreements">
      {notices.error && <ErrorState error={notices.error} onRetry={notices.reload} />}
      <div className="panel panel-pad">
        {notices.data?.length ? (
          <ul className="legal-notices">
            {notices.data.map((n) => (
              <li key={`${n.kind}-${n.version}`}>
                <b>{n.title}</b>{" "}
                <span className="muted small">
                  {n.first
                    ? `published ${formatDate(n.effective_at)}`
                    : `${n.in_effect ? "changed" : "changes"} ${formatDate(n.effective_at)}: ${n.summary}`}
                </span>{" "}
                {n.action === "accept" ? (
                  <button type="button" className="btn small" onClick={() => setAccepting(n)}>
                    Review and accept
                  </button>
                ) : (
                  <Link to={`/legal/${n.kind}?version=${n.version}`}>Read it</Link>
                )}
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted small">You're up to date with everything you've agreed to.</p>
        )}
        <p className="small">
          <Link to="/legal">All terms and policies, with every earlier version</Link>
        </p>
      </div>
      {accepting && (
        <AcceptDocument
          kind={accepting.kind}
          version={accepting.version}
          onAccepted={() => {
            setAccepting(undefined);
            notices.reload();
          }}
          onCancel={() => setAccepting(undefined)}
        />
      )}
    </section>
  );
}

const CONFIRMATION = "close my account";

export function AccountData() {
  const navigate = useNavigate();
  const { refresh } = useSession();
  const [error, setError] = useState<string>();
  const [closing, setClosing] = useState(false);
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);

  const download = async () => {
    setError(undefined);
    try {
      saveFile("slop-data.json", await api.exportData(), "application/json");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const close = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.closeAccount(confirm);
      await refresh();
      navigate("/", { replace: true });
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <section id="data">
      <div className="panel panel-pad">
        <p>
          <button type="button" className="btn" onClick={() => void download()}>
            Download my data
          </button>{" "}
          <span className="muted small">Everything linked to your account, as one JSON file.</span>
        </p>
        {!closing ? (
          <p>
            <button type="button" className="btn ghost danger" onClick={() => setClosing(true)}>
              Close my account…
            </button>
          </p>
        ) : (
          <form className="form close-account" onSubmit={(e) => void close(e)}>
            <h3>Close your account</h3>
            <p>This can't be undone. When you close your account:</p>
            <ul>
              <li>You're signed out everywhere, and this sign-in can never be used on Slop again.</li>
              <li>Your email and settings are removed, and your library and playtime deleted.</li>
              <li>Your votes still count in ratings, but aren't linked to you any more.</li>
              <li>Your apps leave the store. Players who installed them keep them.</li>
              <li>Your bounty pledges are withdrawn, and the cards you saved for them are removed.</li>
              <li>
                A People page is frozen as it is and takes no new work; job records, agreements and reviews stay, because
                the other side relies on them.
              </li>
            </ul>
            <p className="muted small">
              Download your data first if you want a copy. Open People jobs, and pledges being charged for a delivery, must
              be finished first.
            </p>
            <label>
              Type <b>{CONFIRMATION}</b> to confirm
              <input value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="off" />
            </label>
            <div className="row-actions">
              <button className="btn danger" disabled={busy || confirm.trim().toLowerCase() !== CONFIRMATION}>
                {busy ? "Closing…" : "Close my account"}
              </button>
              <button type="button" className="btn ghost" onClick={() => setClosing(false)}>
                Keep my account
              </button>
            </div>
          </form>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}

export function AccountNotifications() {
  const prefs = useLoad(() => api.notifications.prefs(), []);
  const [error, setError] = useState<string>();
  if (prefs.error) return <ErrorState error={prefs.error} onRetry={prefs.reload} />;
  if (!prefs.data) return null;
  const p = prefs.data;
  const set = async (category: string, email: boolean) => {
    setError(undefined);
    try {
      await api.notifications.setPrefs({ [category]: email });
      prefs.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <section id="notifications">
      <div className="panel panel-pad switch-list">
        <p className="muted small">
          Everything shows under the 🔔 in Slop. Choose what's also emailed to you
          {p.has_email ? "" : " (you don't have an email on your account, so nothing is emailed)"}
          {p.email_on ? "" : ". This server doesn't send email yet"}. Slop never sends marketing.
        </p>
        {!isDesktop && <PushSwitch />}
        {p.categories.map((c) => (
          <label className="check" key={c.category}>
            <input type="checkbox" checked={c.email} onChange={(e) => void set(c.category, e.target.checked)} />
            <span>Email me about {c.label.charAt(0).toLowerCase() + c.label.slice(1)}</span>
          </label>
        ))}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}

/** Account → Money: donations given, where donations are received, and Slop's fee statements. */
export function AccountMoney() {
  const mine = useLoad(() => api.donations.mine(), []);
  const statements = useLoad(() => api.donations.feeStatements(), []);
  const account = useLoad(() => api.donations.account(), []);
  const [cancelling, setCancelling] = useState<string>();
  const [error, setError] = useState<string>();
  if (mine.error) return <ErrorState error={mine.error} onRetry={mine.reload} />;
  const d = mine.data;
  const receiving = Boolean(account.data && account.data.status !== "none" && account.data.payments !== "off");
  const loaded = d && (statements.data || statements.error) && (account.data || account.error);
  const none =
    loaded && d.donations.length === 0 && d.server_donations.length === 0 && !receiving && !statements.data?.statements.length;
  const cancel = async (id: string) => {
    setError(undefined);
    try {
      await api.donations.cancel(id);
      setCancelling(undefined);
      mine.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  const fmt = (minor: number, currency: string) => {
    const symbol = currency === "eur" ? "€" : currency === "gbp" ? "£" : "$";
    return `${symbol}${(minor / 100).toFixed(2)}`;
  };
  return (
    <>
      {d && (d.donations.length > 0 || d.server_donations.length > 0) && (
        <section id="donations" className="account-panel" aria-labelledby="donations-title">
          <h3 className="account-panel-title" id="donations-title">
            Your donations
          </h3>
          {d.server_donations.length > 0 && (
            <ul className="panel report-list">
              {d.server_donations.map((s) => (
                <li key={s.id}>
                  <div>
                    <b>Server donation to {s.app_title}</b>{" "}
                    <span className="muted small">· {fmt(s.total_minor, s.currency)} a month</span>
                    <div className="muted small">
                      {s.status === "active" && s.next_charge_at && `Next payment ${formatDate(s.next_charge_at)}`}
                      {s.status === "cancelled" &&
                        `Cancelled ${s.cancelled_at ? formatDate(s.cancelled_at) : ""}${s.cancelled_by === "servers_shut_down" ? " (the servers shut down)" : ""}`}
                      {s.simulated && " · simulated"}
                    </div>
                    <details className="small">
                      <summary>What you agreed to</summary>
                      <p>{s.consent_text}</p>
                    </details>
                  </div>
                  {s.status !== "cancelled" &&
                    (cancelling === s.id ? (
                      <span className="row-actions">
                        <button type="button" className="btn small danger" onClick={() => void cancel(s.id)}>
                          Yes, cancel it
                        </button>
                        <button type="button" className="btn small ghost" onClick={() => setCancelling(undefined)}>
                          Keep it
                        </button>
                      </span>
                    ) : (
                      <button type="button" className="btn small" onClick={() => setCancelling(s.id)}>
                        Cancel
                      </button>
                    ))}
                </li>
              ))}
            </ul>
          )}
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          {d.donations.length > 0 && (
            <table className="table small panel">
              <thead>
                <tr>
                  <th>Date</th>
                  <th>To</th>
                  <th>You paid</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {d.donations.map((x) => (
                  <tr key={x.id}>
                    <td>{formatDate(x.paid_at ?? x.created_at)}</td>
                    <td>
                      <Link to={`/app/${x.app_slug}`}>{x.app_title}</Link>
                      {x.kind === "monthly" && <span className="muted"> · monthly</span>}
                    </td>
                    <td>{fmt(x.total_minor, x.currency)}</td>
                    <td>
                      {x.status === "paid" ? "Paid" : x.status === "pending" ? "Waiting" : x.status.replace("_", " ")}
                      {x.simulated && <span className="muted"> · simulated</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <p className="faint small">Receipts for real payments come by email from the maker's Stripe account.</p>
        </section>
      )}
      {receiving && <ReceivingDonations account={account} />}
      {account.data?.status === "none" && account.data.payments !== "off" && <DonationsSetupHint />}
      {statements.data && statements.data.statements.length > 0 && (
        <FeeStatements statements={statements.data.statements} vat={statements.data.vat} onVat={statements.reload} />
      )}
      {none && (
        <p className="empty-state account-empty">
          No donations yet. Donations you give, and donations to your apps once you set them up, show here with Slop's fee
          statements.
        </p>
      )}
    </>
  );
}

/**
 * Makers with apps but no Stripe account yet: where donations are turned on (per app, on its
 * Manage page, which starts Stripe's own sign-up). Private apps take no donations, so they're left
 * out.
 */
function DonationsSetupHint() {
  const apps = useLoad(async () => {
    const mine = await api.myApps();
    const settings = await Promise.all(mine.map((a) => api.donations.settings(a.slug).catch(() => null)));
    return mine.filter((_, i) => settings[i] && !settings[i]!.private && !settings[i]!.donations_on);
  }, []);
  const list = apps.data ?? [];
  if (list.length === 0) return null;
  const shown = list.slice(0, 10);
  return (
    <section id="donations-setup" className="account-panel" aria-labelledby="donations-setup-title">
      <h3 className="account-panel-title" id="donations-setup-title">
        Want donations?
      </h3>
      <div className="panel panel-pad">
        <p>
          Turn them on per app: <b>Publish → your app → Manage → Donations</b>. That starts Stripe's own sign-up (you need to
          be 18 or older), and donations then go straight to your Stripe account. Slop's fee is 5%, and donations never unlock
          anything.
        </p>
        <ul className="plain donation-setup-list">
          {shown.map((a) => (
            <li key={a.slug}>
              <b>{a.title}</b>{" "}
              <Link to={`/publish/${a.slug}/manage#donations`} aria-label={`Set up donations for ${a.title}`}>
                Set up donations
              </Link>
            </li>
          ))}
        </ul>
        {list.length > shown.length && (
          <p className="small">
            <Link to="/publish">All your apps</Link>
          </p>
        )}
      </div>
    </section>
  );
}

/** Where donations are paid, and switching or disconnecting it: 72 hours' wait (SECURITY §9). */
function ReceivingDonations({ account }: { account: Loaded<DonationAccount> }) {
  const gate = useTermsGate();
  const [confirm, setConfirm] = useState<"switch" | "disconnect">();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const a = account.data;
  if (!a || a.status === "none" || a.payments === "off") return null;
  const run = async (action: () => Promise<{ url: string | null } | void>) => {
    setBusy(true);
    setError(undefined);
    try {
      const res = await gate.guard(action);
      setConfirm(undefined);
      if (res && res.url) await openExternal(res.url);
      account.reload();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const c = a.change;
  return (
    <section id="receiving" className="account-panel" aria-labelledby="receiving-title">
      {gate.prompt}
      <h3 className="account-panel-title" id="receiving-title">
        Receiving donations
      </h3>
      <div className="panel panel-pad">
        <p>
          Donations to your apps go to your own Stripe account
          {a.status === "active" ? "." : a.status === "pending" ? ", which Stripe is still checking." : ", which Stripe has restricted: open Stripe to fix it."}{" "}
          Bank details are changed in Stripe, not here.
        </p>
        {c ? (
          <div className="notice" role="status">
            <p>
              <b>{c.kind === "switch" ? "Switching to a new Stripe account" : "Disconnecting your Stripe account"}</b>, asked{" "}
              {formatDate(c.requested_at)}. It happens on {formatDate(c.effective_at)}
              {c.kind === "switch" && !c.new_account_ready ? ", once you've finished the new account's sign-up on Stripe" : ""}.
              {c.kind === "disconnect" && " Donations then turn off on all your apps, and monthly server donations end."}
            </p>
            <p className="small">If you didn't ask for this, cancel it now and sign out your other sessions below.</p>
            <div className="row-actions">
              <button type="button" className="btn small" disabled={busy} onClick={() => void run(() => api.donations.cancelAccountChange())}>
                Cancel the change
              </button>
              {c.kind === "switch" && !c.new_account_ready && (
                <button type="button" className="btn small ghost" disabled={busy} onClick={() => void run(() => api.donations.refreshAccount())}>
                  I've finished on Stripe
                </button>
              )}
            </div>
          </div>
        ) : confirm ? (
          <div className="notice">
            <p>
              {confirm === "switch"
                ? "You'll set up a new Stripe account now. For your safety, new donations start going to it 72 hours from now, and we email you straight away."
                : "In 72 hours donations turn off on all your apps and monthly server donations end. We email you straight away, and you can cancel until then."}
            </p>
            <div className="row-actions">
              <button
                type="button"
                className={confirm === "disconnect" ? "btn small danger" : "btn small slime"}
                disabled={busy}
                onClick={() => void run(() => api.donations.changeAccount(confirm, "/account/money#receiving"))}
              >
                {confirm === "switch" ? "Set up the new account" : "Disconnect in 72 hours"}
              </button>
              <button type="button" className="btn small ghost" onClick={() => setConfirm(undefined)}>
                Never mind
              </button>
            </div>
          </div>
        ) : (
          <div className="row-actions">
            <button type="button" className="btn small" onClick={() => setConfirm("switch")}>
              Use a different Stripe account
            </button>
            <button type="button" className="btn small ghost" onClick={() => setConfirm("disconnect")}>
              Disconnect Stripe
            </button>
          </div>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}

const money = (minor: number, currency: string) => {
  const symbol = currency === "eur" ? "€" : currency === "gbp" ? "£" : "$";
  return `${symbol}${(minor / 100).toFixed(2)}`;
};

/** Slop's fee by month, with the VAT inside it (LEGAL §3.5). */
function FeeStatements({ statements, vat, onVat }: { statements: FeeStatement[]; vat: VatTreatment; onVat: () => void }) {
  const showVat = vat.kind !== "none";
  return (
    <section id="fee-statements" className="account-panel" aria-labelledby="fee-statements-title">
      <h3 className="account-panel-title" id="fee-statements-title">
        Fee statements
      </h3>
      <p className="muted small">
        Slop's 5% fee on the donations and People payments you received, by month.{" "}
        {vat.kind === "none" && "There's no VAT on Slop's fee where you are."}
        {vat.kind === "included" &&
          `The fee includes ${(vat.rate_bp! / 100).toFixed(vat.rate_bp! % 100 ? 1 : 0)}% VAT, which Slop pays out of its 5%; it's never added on top.`}
        {vat.kind === "reverse_charge" &&
          `Reverse charge: you account for the VAT on Slop's fee yourself (VAT number ${vat.vat_number}).`}{" "}
        {showVat && (vat.slop_vat_id ? `Slop's VAT number: ${vat.slop_vat_id}.` : "Slop's VAT registration is still being set up.")}
      </p>
      <table className="table small panel">
        <thead>
          <tr>
            <th>Month</th>
            <th>For</th>
            <th>Payments</th>
            <th>Received</th>
            <th>Slop fee</th>
            {showVat && <th>VAT in the fee</th>}
          </tr>
        </thead>
        <tbody>
          {statements.map((s) => (
            <tr key={s.month + s.kind + s.currency}>
              <td>{s.month}</td>
              <td>{s.kind === "donation" ? "Donations" : "People jobs"}</td>
              <td>{s.payments}</td>
              <td>{money(s.received_minor, s.currency)}</td>
              <td>
                {money(s.slop_fee_minor, s.currency)}
                {s.simulated && <span className="muted"> · simulated</span>}
              </td>
              {showVat && <td>{vat.kind === "reverse_charge" ? "Reverse charge" : money(s.vat_minor, s.currency)}</td>}
            </tr>
          ))}
        </tbody>
      </table>
      {showVat && <VatNumber vat={vat} onSaved={onVat} />}
    </section>
  );
}

function VatNumber({ vat, onSaved }: { vat: VatTreatment; onSaved: () => void }) {
  const [value, setValue] = useState(vat.vat_number ?? "");
  const [note, setNote] = useState<string>();
  const [error, setError] = useState<string>();
  const save = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    setNote(undefined);
    try {
      const res = await api.donations.setVat(value.trim() || null);
      setNote(res.note ?? (res.checked ? "Confirmed. Slop's fee is reverse-charged from now on." : "Removed."));
      onSaved();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    }
  };
  return (
    <form className="form inline-form" onSubmit={(e) => void save(e)}>
      <label>
        VAT number, if you're a VAT-registered business{" "}
        <input value={value} onChange={(e) => setValue(e.target.value)} placeholder={`${vat.country === "GR" ? "EL" : vat.country}123456789`} />
      </label>
      <button className="btn small">Save</button>
      {vat.vat_number && !vat.vat_number_checked && !note && (
        <p className="muted small">Not confirmed yet, so VAT is shown as included in Slop's fee.</p>
      )}
      {note && <p className="muted small">{note}</p>}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

/** Notifications on this phone or browser (web push). */
function PushSwitch() {
  const [state, setState] = useState<PushState>();
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    pushState().then(setState, () => setState("unsupported"));
  }, []);
  if (!state || state === "unsupported") return null;
  const flip = async (on: boolean) => {
    setBusy(true);
    try {
      setState(await (on ? enablePush() : disablePush()));
    } catch {
      setState("off");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="push-switch">
      {state === "needs-home-screen" ? (
        <p className="muted small">
          📱 To get notifications on this iPhone or iPad, add Slop to your Home Screen (Share → Add to Home Screen), open it
          from there, and come back here.
        </p>
      ) : state === "blocked" ? (
        <p className="muted small">📱 Notifications are blocked for Slop in this browser's settings.</p>
      ) : state === "unavailable" ? (
        <p className="muted small">📱 Notifications on phones aren't switched on on this server yet.</p>
      ) : (
        <label className="check">
          <input type="checkbox" checked={state === "on"} disabled={busy} onChange={(e) => void flip(e.target.checked)} />
          <span>
            <b>Notifications on this phone or browser</b>
            <span className="muted small" style={{ display: "block" }}>
              Everything that shows under the 🔔. Slop's push says nothing until your device fetches it from Slop.
            </span>
          </span>
        </label>
      )}
    </div>
  );
}
