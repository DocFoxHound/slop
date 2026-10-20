# Record of processing (draft)

Required by GDPR Art. 30 (`LEGAL-GUIDANCE.md` PRIV-1). Update it in the same change as any new
personal-data field. Lawful bases follow `LEGAL-GUIDANCE.md` §8.1.

| Data | Where | Purpose | Lawful basis | Kept for |
|---|---|---|---|---|
| Display name | `users.display_name` | Profile, store pages, reviews, votes | Contract | Life of account |
| Email (verified, from the sign-in provider) | `users.email` | Account and product emails | Contract | Life of account |
| Country (stated at sign-up) | `users.country_code` | Choosing age and legal rules (AGE-1) | Legal obligation / contract | Life of account |
| 16+ confirmation time | `users.age_16_confirmed_at` | Proof of the 16+ rule. No date of birth is stored | Legal obligation | Life of account |
| Provider account ID | `identities.provider_subject` | Sign-in; after an account is closed, refusing that sign-in | Contract; legitimate interest (closed and banned accounts don't return) | Life of account; kept after closing, only to refuse the sign-in |
| Last sign-in time | `identities.last_sign_in_at` | Security, devices page | Legitimate interest | Life of account |
| Pending sign-up (name, email) | `pending_signups` | Finishing sign-up | Contract | 30 minutes, then deleted |
| Session (hashed token, user agent, times) | `sessions` | Keeping the user signed in; devices page | Contract | Until it ends (expiry, idle timeout or sign-out), then deleted after 7 days by the hourly cleanup |
| Desktop device name | `desktop_auth_requests.device_name`, `sessions.user_agent` | Approval page and devices page | Contract | 10 minutes for requests; sessions as above |
| Desktop access and refresh tokens (hashed) | `access_tokens`, `refresh_tokens` | Keeping the desktop app signed in | Contract | Deleted by the hourly cleanup once expired (15 minutes / 30 days), or with their session |
| Client IP address | Memory only (rate limiter) | Rate limiting | Legitimate interest | Never stored; forgotten once the client is idle |
| "Share my playtime" setting | `users.share_playtime` | PRIV-9: whether launches get Pulse tokens | Contract | Life of account |
| Pulse sessions (hashed token, app, user, times) | `pulse_sessions` | Live player counts | Legitimate interest (PRIV-9: EU consent question open) | Deleted within 24 hours |
| Playtime and launch days per app | `user_app_usage` | Shown to the user; weights their votes (METRICS §2) | Contract / legitimate interest | Life of account |
| Player counts per app per 5 minutes | `pulse_counts` | "Playing now" history and monthly peaks | Not personal data (counts only) | Kept |
| Library | `library_entries` | The user's library; later, recommendations | Contract | Until removed or account deleted |
| Release submitter | `releases.submitted_by` | Who submitted a build (accountability) | Legitimate interest | Life of app + 6 years (scan records, LEGAL-GUIDANCE §15) |
| Votes and vote history (account, app, vote, playtime and launch days at the time, weight, time) | `vote_events`, `votes` | Ratings weighted by playtime, ratings over time, abuse checks, "For you" | Contract (ratings); legitimate interest (recommendations, abuse checks) | Life of the account; anonymised on deletion (METRICS §6) |
| "Recommendations based on my activity" setting | `users.personalize` | Switching "For you" profiling off (LEGAL §7); off by default in the UK under-18 view (PRIV-6) | Legal obligation | Life of account |
| Review decisions | `audit_log` | Record of review and moderation decisions | Legal obligation | 6 years (LEGAL-GUIDANCE §15) |
| Review notes (author's name) | `review_notes` | Reviewers' findings on items waiting for a decision | Legitimate interest (moderation) | With the item they're about |
| Admin API tokens (hashed token, name, last use) | `admin_tokens` | AI reviewers acting for an admin; showing which tokens are in use | Legitimate interest (moderation) | Until revoked or expired, then removable |
| 18+ confirmation for People | `users.adult_confirmed_at` | Selling or hiring needs adults (LEGAL §6); no date of birth | Legal obligation / contract | Life of account |
| Creative profile (name, headline, about, optional location, links, trader declaration, AI-use disclosure) | `creatives` | The public People page; trader status shown to clients (CRD Art. 6a, HUM-1) | Contract; legal obligation (trader declaration) | Life of the profile |
| Stripe account ID and status | `creatives.stripe_account_id`, `stripe_status` | Paying the creative directly (PAYMENTS §3). Identity, tax and bank details stay at Stripe | Contract; legal obligation (DAC7 through Stripe) | Life of the profile + 6 years |
| Portfolio links (files stay in the creative's repository) | `portfolio_sets` | Showing reviewed work on the People page | Contract | Life of the profile |
| Jobs: request, offer, agreed Service Agreement, steps and their notes, delivery links | `jobs`, `job_events` | The contract record between creative and client (HUM-16, HUM-17) | Contract; legal obligation (records) | Life of the account + 6 years |
| People payments and refunds (payer, amounts, fee, Stripe IDs) | `job_payments`, `payments`, `stripe_events` | Mirror of Stripe charges, reconciliation, fraud checks (PAYMENTS §8–9) | Contract; legal obligation (accounting) | 6 years (tax and accounting) |
| Job conversations (text) | `job_messages.body` | Relaying the recorded conversation and letting both sides download it | Contract | While open + 30 days after closing, then deleted |
| Job conversation fingerprints | `job_messages.sha256` | Proving a side's copy is unchanged (not the content) | Legitimate interest (evidence for both sides) | With the job record |
| A side's own conversation copy, submitted for a problem or dispute | `job_evidence` | Mediation of a problem or review dispute | Legitimate interest; provided by that side | With the job record |
| Changes to Agreement, problems, proposals, mediator suggestions | `job_changes`, `job_problems`, `job_problem_proposals` | The contract record and mediation (HUM-16, HUM-19) | Contract | Life of the account + 6 years |
| Review disputes (statements, decision, reason) | `review_disputes`, `creative_reviews.hidden_reason`, `mediator_note` | Deciding disputes on the rules, with a statement of reasons (DSA Art. 17) | Legal obligation / legitimate interest | Life of the review + 6 years |
| Reviews of creatives and replies | `creative_reviews` | Verified-client ratings on People pages (LEGAL §4.5) | Legitimate interest (right to object, GDPR Art. 21) | Life of the profile; kept, frozen, when either account closes (it's part of the job record) |
| Account closed / suspended | `users.closed_at`, `users.suspended_at` | Refusing a closed account's sign-in; limiting a suspended account (DESIGN §4, CON-9) | Contract; legitimate interest | Life of the account record |
| Maker's Stripe account for donations, and the tax-notice confirmation | `donation_accounts` | Paying donations straight to the maker (PAYMENTS §3); PAY-9 | Contract; legal obligation | Life of the account + 6 years |
| Donations (donor, app, amounts, fee, status, Stripe charge or invoice; a message and the choice to share the donor's name) | `donations` | Receipts and history for the donor, totals for the maker, Slop's fee records | Contract; legal obligation (accounting, 10 years) | 10 years (LEGAL-GUIDANCE §15) |
| Server donations (donor, app, amounts, the terms they confirmed, status, reminder times) | `server_donations` | Monthly donations, proof of consent (PAY-14), reminders (PAY-16) | Contract; legal obligation | 10 years |
| Interested list | `interests` | The user's Interested apps: library, stage notifications, "For you"; only counts are public | Contract; legitimate interest (recommendations) | Until removed or the account closes |
| Notifications (title, link, read time) | `notifications` | Telling the user about their jobs, Interested apps and moderation decisions | Contract | 180 days after being read, a year at most |
| Email choices | `users.email_prefs` | Which notification emails the user wants | Contract | Life of account |
| Outgoing emails (address, subject, text) | `email_outbox` | Delivering product emails, including report outcomes to signed-out reporters | Contract; legal obligation (DSA Art. 16(5)) | 30 days after sending |
| Accepted versions of the Terms, Publisher Agreement and People Terms | `legal_acceptances` | Proof of which words the user agreed to, and when | Legal obligation / legitimate interest (contract claims) | Life of the account + 6 years (LEGAL-GUIDANCE §15) |
| Reports (subject, reason, details, reporter's account, or a signed-out reporter's name and email; a quoted job message; copyright-notice details) | `reports` | Notice-and-action (DSA Art. 16, CON-8), DMCA notices (CON-3), records of decisions and time taken | Legal obligation | 6 years; child-abuse reports preserved at least 1 year (18 U.S.C. §2258A(h)) |
| Moderation decisions, statements of reasons, appeals and counter-notices (name, address, phone) | `moderation_decisions` | Statements of reasons (DSA Art. 17, CON-9), appeals (CON-16), the repeat-infringer count (CON-4) | Legal obligation | 6 years (LEGAL-GUIDANCE §15) |

**Not collected:** passwords, provider access tokens (used once at sign-in, never stored), date
of birth, IP addresses in the database.
