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
| Session (hashed token, user agent, times) | `sessions` | Keeping the user signed in; devices page | Contract | Until expiry or revocation (30 days max) ⚖️ purge job still to build |

**Not collected:** passwords, provider access tokens (used once at sign-in, never stored), date
of birth, IP addresses in the database.
