# Slop — Money: Donations & People Payments

**Status:** Draft · companion to [`DESIGN.md`](DESIGN.md) §12 and §15
**Last updated:** 2026-10-19
**Related:** `LEGAL.md` (private) (legal positions behind every rule here) · [`SECURITY.md`](SECURITY.md) §9 · [`METRICS.md`](METRICS.md)

> Wherever this document says "must" or "never", the reason is usually a legal position in `LEGAL.md`. Check there before changing it.

---

## 1. Principles

1. **Slop never holds money.** Every payment goes straight into the **maker's or creative's own Stripe account**. Slop receives only its own 5% platform fee (`LEGAL.md` §3.1).
2. **Donations aren't purchases.** Nothing is unlocked or given in return, and apps can't tell who donated (`LEGAL.md` §3.2).
3. **People is the only sale.** It's a creative selling services to a client, and Slop isn't a party to it (`LEGAL.md` §4).
4. **Donors pay the card fees.** The total is shown before paying (`LEGAL.md` §3.4).
5. **No card data at Slop.** Stripe Checkout/Elements only.

## 2. Money flows

| Flow | Paid by | Paid into | Slop's fee | Refunds |
|---|---|---|---|---|
| **Donation** (one-off) | Donor | Maker's Stripe account | 5% of the donation (**VAT included** where due, paid out of Slop's 5%, never added to the donation; `LEGAL.md` §3.5) | Final. The maker may refund mistakes. |
| **Server donation** (monthly) | Donor | Maker's Stripe account | 5% of each payment | Cancel any time. Past payments are final. |
| **People job** (one or two payments) | Client (the maker) | Creative's Stripe account | 5% of each payment, from what the creative receives | Issued by the creative. Otherwise the client goes to their card issuer. |

**Not offered:** wallets/top-ups, store credit, gift cards, escrow, payouts made by Slop, splitting a payment between accounts, or **in-app purchases** of any kind (decided 2026-10-17).

## 3. Payment provider: Stripe Connect

- **Account type:** the maker or creative has their **own Stripe account and dashboard**, and Stripe carries losses (Stripe's "Standard" setup). This is the setup that keeps Slop out of the money (`LEGAL.md` §3.1).
- **Charge type:** **direct charges** on the connected account, with `application_fee_amount` set to Slop's 5%.
- **What Stripe handles** (`LEGAL.md` §3.7):
  - Identity checks (KYC), sanctions screening, payouts to the maker's bank.
  - Chargebacks, which come out of the maker's account.
  - US 1099-K forms.
- **Countries:** makers can receive donations only if they're in a country Stripe supports for accounts (around 45+ countries; check Stripe's live list, don't copy it into code). Makers elsewhere can still list apps. Their store page says "This maker can't receive donations in their country yet".
  - Slop keeps its own **blocked-countries list** for sign-up (sanctions, `LEGAL.md` §9). That's separate from Stripe's list.
- **No reserve needed:** a chargeback reserve is money a platform sets aside in case a donor disputes a charge after the maker has been paid and the maker can't pay it back. With makers on their own Stripe accounts, Stripe recovers it from the maker. Slop doesn't need one.

## 4. Onboarding

1. The maker clicks **Enable donations** and is sent to Stripe's onboarding. They create or connect a Stripe account, Stripe verifies their identity and bank account, and they return to Slop.
2. Slop stores only the Stripe account ID and status (`pending`, `active`, `restricted`). Donate buttons appear only for `active` accounts.
3. Shown once before enabling: "Donations go straight to you. Slop takes a 5% platform fee. Donations may be taxable income for you."
4. **18+ only** for receiving and making payments (`LEGAL.md` §6).
5. **Official teams** connect the studio's Stripe account (Stripe verifies the business).

## 5. Donations

### 5.1 Flow
1. The player taps **♥ Donate**. Presets are shown, including the maker's suggested amount, and any amount from $1 to $500 is allowed. Presets are amounts **the maker receives before Slop's fee**.
2. The total is calculated so that the card fee is covered:
   ```
   donation D (chosen) · Slop fee F = 5% of D · maker receives D − F
   total charged T is chosen so that T − card_fee(T) = D
   ```
   It's shown as: "You pay **$3.40** · @maker receives **$2.85** · $0.15 Slop fee · $0.40 card processing."
   - The addition is the **same for every payment method** (`LEGAL.md` §3.4).
   - Card fees vary by card and country. Slop calculates the addition using a fixed published rate per currency, not the exact fee for each card. Whatever is left over or short after Stripe's actual fee is the maker's.
3. Payment goes through Stripe Checkout/Elements on the maker's account. 3-D Secure runs where required.
4. A **verified Stripe webhook** confirms the payment. Slop records it in donation history (`METRICS.md`).
5. The donor gets a receipt from the maker's Stripe account plus a thank-you from Slop. **Donors are never shown publicly.** A donor may choose to share their name and a message privately with the maker.

### 5.2 Rules (from `LEGAL.md` §3.2)
- Nothing is unlocked, ever. No supporter lists, badges, credits or roles (decided 2026-10-17). Donors get a thank-you and a receipt.
- **No "has this player donated?" API** of any kind. Apps can't change their behaviour for donors.
- Donations are **final**. The maker can refund a donation from their Slop dashboard (e.g. a mistyped amount).
- Wording follows the word list in `LEGAL.md` §12.

### 5.3 Community builds and mods
A community build or mod has its own Donate button for the author's own Stripe account, and **one-off donations are shared with the original's maker** (decided 2026-10-21), so improving someone's project pays better than copying it:
- The original's maker gets **30% of the two payments together**, as **its own direct donation** to their Stripe account, paid **first**; then the rest goes to the branch's author as a second donation. Two payments, never one split payment: Slop still never holds or divides money.
- The server won't start the branch's payment until a matching share has been paid by the same donor in the last two hours, and each share is used once (`donations.share_for`, `share_donation_id`, migration 0047). Shared donations start at $2.34 to the branch, so the share meets the $1 minimum.
- If the original doesn't take donations, or the donor is the original's maker, it all goes to the branch's author.
- Monthly server donations aren't shared: they pay for the branch's own servers.
- Each payment has its own fees (Slop's 5% and processing) and its own receipt.

### 5.4 Splits between collaborators
**One Stripe account per app** (decided 2026-10-17). If several people made the app, the publisher splits the money with them outside Slop (`LEGAL.md` §3.6).

### 5.4 Fix bounties (built 2026-10-21, `bounties.rs`)

Players pledge toward a fix or feature on an app; whoever delivers it in an approved release (the app's maker, or a community build's maker) is paid.

- **A pledge saves a card, nothing more.** Stripe Checkout in **setup mode on Slop's platform account** saves the backer's card to their Stripe customer. Slop stores Stripe's IDs only. 1.00 to 500.00 per backer per bounty, one currency per bounty; withdrawable until a delivery is confirmed.
- **Confirmation:** the app's maker accepts or declines a delivery. When the maker delivered it themselves, or doesn't answer in 14 days, the backers vote; more than half of the pledged amount decides. If every pledge that counts for the vote is withdrawn, nobody is left to decide or pay: the delivery is declined, the builder is told, and the bounty reopens until its deadline (or ends, if that has passed).
- **Payment is a direct charge on the builder's own Stripe account** (their donation account): the saved card is cloned to that account and charged off-session, with Slop's 5% as `application_fee_amount`, recorded in `payments` (kind `bounty`). **Slop never holds the money.** A refused card marks that pledge failed and tells the backer; the others still pay.
- **Expiry:** 180 days without a confirmed delivery ends the bounty and releases the saved cards (detached at Stripe).
- **Staff hiding a bounty** (`hide_bounty`) releases every pledge still open or due: nothing more is charged, even for a delivery already accepted, and the cards are detached at Stripe once the decision is saved. If an appeal reverses it, the bounty comes back as it was, for the time it had left but at least 7 days (one hidden while it was being paid counts as paid). The released pledges stay released, so their backers are told it's back and can pledge again.
- **Closing an account** waits while one of its pledges is due (confirmed, not yet charged), as it does for an owed job payment. Otherwise its pledges are withdrawn and its Stripe customer, with every card saved to it, is deleted.
- **Refunds and disputes** the builder's Stripe account reports on a pledge's charge are recorded in `payments` against it (the charge is matched to the payment intent the pledge's payment holds), as for donations and People jobs. They change no pledge or bounty status.
- **Needs before going live:** Stripe's rules for saving a card for later merchant-initiated charges (the pledge text is the customer's agreement, and it's shown before Checkout), the lawyer's review of that text and of refunds and disputes (the builder handles them, as with donations), and a webhook for **platform** events (`checkout.session.completed`) besides the connected-account one. Pledges also sync when the backer returns from Checkout, so the webhook is a backup.

### 5.5 Donation holds (built 2026-10-22, `donation_holds.rs`)

When a complaint says an app shouldn't be making money (someone else's work or trademark, a law), staff can stop its donations without taking the app out of the store:

- **Paused for review:** quick and reversible while Slop looks into it. A moderator or finance staff member places it alone.
- **Turned off until fixed:** Slop found a problem the maker has to fix. A sensitive action, so it goes through two-person approval (`staff.rs`) once Slop has a second person.

While held, new donations stop, including the app's share of donations to its community builds. Monthly server donations are paused at Stripe (`pause_collection` with `void`: no charge, not cancelled) and their donors are told. Checkout pages opened before the hold are closed, and bounty payouts to a delivery from the app wait until it's lifted (since 2026-10-22). Donors only ever see "Donations to this app are paused". The maker gets the reason (and the ground, if given) as a notification and on the app's Donations page, where they can ask for a review once they've fixed it: one request at a time, and again a day after an answer. Staff lift the hold or keep it with an answer. Lifting restarts the monthly donations from their next date, and every step is in the audit log. Admin panel: the app's page under Apps, and Money.

## 6. Server donations (recurring)

- **Available only for apps that declare servers.** The publisher ticks "This app runs servers" and writes a short statement ("Pays for 3 game servers and the leaderboard database").
- The button is labelled **"♥ Server donation · monthly"**.
- Set up with **Stripe Billing on the maker's account**: a monthly charge with `application_fee_percent = 5`. The donor still pays the card fees (same calculation as §5.1).
- Before confirming, the donor sees: "**$5.40 every month** until you cancel. Cancel any time from your library." They must actively confirm.
- **Cancelling:** two clicks from the library or account page. It takes effect immediately and no further payments are made.
- **Servers are never tied to donating.** If the maker marks the servers as shut down, all server donations to that app are cancelled automatically and donors are emailed.
- Receipts for every payment. An email reminder before the first renewal.

## 7. People payments

**Decided 2026-10-19.** Simple for both sides: the maker sees one price and pays it; the creative sees what they'll receive.

- **One price.** The creative's quoted price is what the maker pays, in full, with nothing added at checkout (PAY-10). Prices are in USD, EUR or GBP, from **$10 to $10,000** per job (fraud limits, §9).
- **Who pays what:**
  ```
  price P (what the maker pays) · Slop fee = 5% of P (VAT included, LEGAL.md §3.5)
  creative receives P − 5% − Stripe's card fee (Stripe takes it from the creative's account)
  ```
  The offer form shows the creative "You'll receive about $X" before they send it. The maker never sees a fee added.
- **When it's paid** (the creative picks; the default is the middle one):
  - **All up front** — one payment when the maker accepts.
  - **Half now, half on delivery** (default) — the first half (rounded up to the cent) on accepting, the rest when the maker says the work looks good.
  - **All on delivery** — one payment at the end.
- **Each payment is a direct charge** on the creative's Stripe account through Stripe Checkout, with `application_fee_amount` = 5% of that payment. **No escrow:** Slop can't hold the money until delivery (`LEGAL-GUIDANCE.md` §2), so the split is how the risk is shared. The payment page never promises protection Slop can't give.
- **The payment page says:** "Your contract is with @creative, not Slop. Slop doesn't settle disputes. Your rights are against @creative." (HUM-5)
- **Work starts** when the first payment succeeds (straight away for "all on delivery").
- **Delivery before the last payment is a preview.** Paying what's due on delivery accepts it; the creative then sends the final files, which completes the job. Rights pass only on full payment.
- **Changes to Agreement** carry their own price (zero or $10+), paid when the client accepts the change, as a separate direct charge with the same 5% fee.
- **Owed money:** a client with a payment still due on finished work can't start new jobs until it's paid (HUM-21). Slop never collects it for the creative.
- **Completion:** after delivery the maker has **7 days** to press "Looks good" or ask for a revision. Otherwise the job completes automatically, unless a problem about it is open (it waits until the problem closes). If a final payment is still unpaid then, it stays **owed** on the job page; the credit and badge (`DESIGN.md` §15.4) wait until it's paid.
- **Refunds:** the creative issues them from their Stripe dashboard (full or partial). Slop records them from Stripe's webhooks. **Slop doesn't decide disputes.** A refund both sides agree to in a problem (`DESIGN.md` §15.6) is shown as agreed until the creative issues it. If the creative refuses, the maker's option is a chargeback through their card issuer, which goes against the creative's Stripe account; the downloadable job record is both sides' evidence.
- **Cancelling:** before acceptance either side can decline for free. After acceptance the creative can cancel (refunding what's fair from Stripe). EU/UK consumers hiring a trader can **withdraw** within 14 days (HUM-2, HUM-3), even after the job completes (HUM-4); if they asked for work to start straight away, the right ends once the work is delivered. Withdrawing from a completed job takes its credit away.
- **Slop acts only on rule-breaking** (scams, fraud, repeated non-delivery). Possible actions: warning, suspension, removal.
- **Tax reporting:** creatives with EU links give DAC7 details during onboarding (`LEGAL.md` §4.3). Stripe's onboarding collects identity and bank details; Slop stores only the account ID and status.
- **Without Stripe configured** (development, the simulation sandbox) the job flow works and payments are marked **simulated**. Production never offers simulated payments.

## 8. Records (no ledger of balances)

Slop holds no balances, so it doesn't need a double-entry ledger. It keeps a **record of payments** that mirrors Stripe:

```
payments(id, kind,                  -- donation | server_donation | people_job | refund
         stripe_account_id,         -- recipient (maker/creative)
         stripe_charge_id UNIQUE, payer_account_id, app_id NULL, job_id NULL,
         amount_total_minor, donation_minor, slop_fee_minor, card_fee_minor, currency,
         status,                    -- succeeded | refunded | partially_refunded | disputed
         anonymous bool, message_id NULL, created_at)
```

- **Integer amounts in minor units** (cents), with the currency on every row.
- Records are only ever **added**. Corrections (refunds, disputes) are new rows linked to the original. Built for People jobs, donations and bounty pledges: a `charge.refunded` or `charge.dispute.created` event is matched to the original by the charge, its payment intent (bounty pledges) or its invoice (monthly donations, where Stripe's charge names it).
- **Idempotent webhook processing**, keyed on the Stripe event ID. The current state is always fetched from Stripe's API, never trusted from the webhook body (`SECURITY.md` §8.2).
- **Daily reconciliation** of the record against Stripe's application fees and charges. Any mismatch alerts an admin.
- Slop's own fee income is reconciled for accounting, with a monthly fee statement to each maker (`LEGAL.md` §3.5).

## 9. Fraud

| Threat | Mitigation |
|---|---|
| **Card testing** (stolen cards tried with small donations) | Stripe Radar, sign-in required to donate, a $1 minimum, a rate limit per account (per IP for every write), a proof-of-work check after unfinished checkouts, 18+ (built). Bounty pledges, which save a card on Slop's own account, have the same checks since 2026-10-22. Not built: limits per card. |
| **Cashing out stolen cards through your own app** | Stripe's KYC and risk checks on the maker's account, and payout timing set by Stripe. Slop flags donation spikes from new accounts or many countries and can disable donations on an app. |
| **Hijacked maker account** | Bank details are managed in the maker's **Stripe** dashboard, not in Slop, so taking over a Slop account doesn't redirect money. Disconnecting or switching the Stripe account requires a 72-hour delay, and an email is sent (`SECURITY.md` §9). No step-up since 2026-10-25 (`SECURITY.md` §4.3). |
| **Self-dealing People jobs** | Built: client and creative can't be the same account, and a job payment is only recorded for the Checkout session Slop made for it, on the creative's account, for the amount and currency due (a creative can't mark a job paid with a payment to themselves). Not built yet: spotting the same card, device or Stripe account on both sides, or money going round in loops (security fix plan 4.5 and 6.7). |
| **Buying a bounty's vote** | The deliverer and the app's maker can't vote, and their pledges don't count; only pledges made before the delivery count; no pledges while a delivery waits; backers get three days unless every one of them has voted (built 2026-10-22). |

## 10. The mockup and what's built

The original mockup's (`mockup/`) Donate button (with the donor-pays-fees breakdown) is pure UI.

**Built (2026-10-19):** People payments (`crates/slop-api/src/people/`): Stripe Connect onboarding, Checkout for each job payment as a direct charge with the 5% application fee, signed webhooks, and the `payments` record (§8).

**Built (2026-10-20):** donations (`crates/slop-api/src/donations.rs`):
- **Enable donations** on the app's Manage page: the tax notice (PAY-9, recorded), then Stripe's sign-up for the maker's own account (the People account is reused if they have one).
- **The Donate box:** presets around the maker's suggestion, each showing the donor's total first (PAY-10), plus any amount from $1 to $500. Processing comes from `processing_rates` (one published rate per currency: 2.9% + 30¢ / 25 c / 20 p), or out of the maker's share where `country_policies.donor_covers_fees` is off (PAY-12). An unticked box shares the donor's name and a message with the maker only. 18+, signed in, 10 starts an hour at most.
- **Server donations:** Stripe Billing on the maker's account with `application_fee_percent` set so the fee is 5% of the donation; the exact terms are saved (`server_donations.consent_text`); cancel in two clicks from the account page with a confirmation email; reminders 7 days before the first renewal and yearly; "The servers are shut down" cancels them all and emails the donors. These emails go out whatever the donor's email choices.
- **Records:** `donations` (one row per payment, monthly ones too) and `payments` rows (kind `donation`) mirroring Stripe; refunds from the maker's button call Stripe on the maker's account (PAY-3).
- **The maker's view:** totals, months with at least 10 donations, server-donor counts from 10 up, names and messages only from donors who shared them, and a refund button for mistakes. **Fee statements** by month on the account page (donations and People jobs).
- **Daily reconciliation** (PAY-2): yesterday's Stripe application fees per account against Slop's records; mismatches are logged and written to the audit log.
- **PAY-5 test:** the app-facing code (Pulse, updates, release manifests, the desktop update feed) never mentions donations, and an app's page is identical before and after someone donates.
- Webhooks also handle `invoice.paid` and `customer.subscription.deleted`.

- **Fee statements show the VAT inside Slop's fee** (LEGAL §3.5, built 2026-10-20): at the maker's country rate (`country_policies.vat_rate_bp`, EU and UK standard rates) for individuals, or "reverse charge" for VAT-registered businesses whose number the EU's VIES service confirms (UK numbers can't be checked automatically yet, so VAT stays shown as included). Slop's own VAT number appears once `SLOP_VAT_ID` is set.
- **Card testing** (§9, built 2026-10-20): after 3 unfinished checkouts in a day, starting another donation needs a proof-of-work check the browser solves in a second or two (`challenge.rs`). It's not a third-party CAPTCHA, so nothing goes to another company; Stripe Radar and the rate limits do the rest.
- **Switching or disconnecting the Stripe account** (§9, built 2026-10-20): works from any signed-in session (the step-up it needed went on 2026-10-25, `SECURITY.md` §4.3), then waits 72 hours, announced at once in Slop and by email (whatever the maker's email choices), and can be cancelled meanwhile. A switch also waits for the new account to finish Stripe's sign-up; existing server donations stay on the old account. Disconnecting turns donations off and ends server donations, telling the donors. Connecting a first account works from any signed-in session too.

Not yet: Slop's VAT registrations and returns (§11).

## 11. Open questions
1. **iOS:** whether, and how, donations work in an iOS app (`DESIGN.md` §17).
2. **VAT set-up:** Stripe Tax, or a VAT service, for the EU non-Union OSS and UK returns (`LEGAL.md` §3.5).
