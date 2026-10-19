# Record of processing (draft)

Required by GDPR Art. 30 (`LEGAL-GUIDANCE.md` PRIV-1). Update it in the same change as any new
personal-data field. Lawful bases follow `LEGAL-GUIDANCE.md` §8.1.

| Data | Where | Purpose | Lawful basis | Kept for |
|---|---|---|---|---|
| Display name | `users.display_name` | Profile, store pages, reviews, votes | Contract | Life of account |
| Email (verified, from the sign-in provider) | `users.email` | Account and product emails | Contract | Life of account |
| Country (stated at sign-up) | `users.country_code` | Choosing age and legal rules (AGE-1) | Legal obligation / contract | Life of account |
| 16+ confirmation time | `users.age_16_confirmed_at` | Proof of the 16+ rule. No date of birth is stored | Legal obligation | Life of account |
| Provider account ID | `identities.provider_subject` | Sign-in | Contract | Life of account |
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

**Not collected:** passwords, provider access tokens (used once at sign-in, never stored), date
of birth, IP addresses in the database.
