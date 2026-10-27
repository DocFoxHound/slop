# SLAPorSLOP — Donation, Payment & Vote History (Metrics)

**Status:** Draft · companion to [`DESIGN.md`](DESIGN.md) §7 (ratings) and §12 (money)
**Last updated:** 2026-10-17
**Related:** [`PAYMENTS.md`](PAYMENTS.md) · `LEGAL.md` (private) §7 · [`SECURITY.md`](SECURITY.md) §10 · [`ARCHITECTURE.md`](ARCHITECTURE.md)

---

SLAPorSLOP keeps a detailed history of:
- **Money:** donations, server donations and People payments, plus their refunds. (Donations aren't purchases; see `LEGAL.md` §3.2.)
- **Likes/dislikes:** 👍 Slaps / 👎 Slop votes, including every time someone changes their vote.

**Database: PostgreSQL only** (decided 2026-10-17). Gameplay data isn't collected. Slop Pulse stays as described in DESIGN §11.

## 1. What gets recorded

### 1.1 Payment history
These records mirror Stripe (`PAYMENTS.md` §8). SLAPorSLOP holds no balances, so this is a history, not a bank ledger.

| Record | Fields |
|---|---|
| Donation | donor, app (or branch), amount the maker receives, total charged, SLAPorSLOP fee, card fee, currency, whether the donor shared their name with the maker (private), message, time |
| Server donation | the same, plus the recurring setup it belongs to, which charge in the series it is, and the cancellation time |
| People payment | client, creative, job, milestone, amount, fee, status |
| Refund / dispute | the original payment, amount, who started it (maker/creative, or a card dispute), time |

### 1.2 Vote history
Every vote action is stored as an **event**. A vote is never just overwritten.

| Field | Why |
|---|---|
| account, app, vote (👍 / 👎 / withdrawn), time | The basic record |
| app version and branch at the time of the vote | "Did version 1.4 fix it?"; ratings per branch |
| **playtime at the time of the vote** (from Pulse) | **Weighted ratings** (§2) |
| previous vote, if changed | Shows people changing their mind after updates |
| review ID (if any) | Links to the written review |
| platform | "Mostly Slop on Linux" |

The current vote per account and app is kept as well. It's updated in the same database transaction.

### 1.3 Other activity
Library adds, installs and Interested. These feed "Popular this month", "For you" and maker dashboards.

## 2. Weighted and unweighted ratings (decided)

- **Weighted** (the default shown): each vote counts according to the voter's playtime at the time they voted. Someone who played 20 hours counts for more than someone who tried it for 5 minutes.
  - Proposed formula: weight = `min(1, log10(1 + minutes) / log10(1 + 600))`. That means 10 hours or more counts fully, and one minute counts ≈ 0.11.
  - Votes with **under 2 minutes** of playtime count as 0.1.
  - The formula is published on the "How ratings work" page.
- **Unweighted:** one account = one vote.
- Store pages show **both**: "**84% Slaps** (weighted by playtime) · 79% unweighted", with a toggle on the rating-over-time chart.
- Apps that aren't games (tools with no Pulse playtime) use launches instead: the number of separate days the user launched the app, with 10 days or more counting fully.

**Built 2026-10-20:** the "How ratings work" page (`/ratings`) publishes the formula and the tables of weights.

## 3. Who sees what

| Viewer | Payments | Votes |
|---|---|---|
| **The user** | Their full history and receipts, exportable | Their full vote history |
| **Other users** | Nothing. Donors are never shown publicly. | **Current votes and reviews are public on profiles by default** (decided). The user can make them private. The history of changes is never shown. |
| **The maker** | Totals per day, version and country. Names and messages only from donors who chose to share them. Minimum group of 10 people before a breakdown is shown. | Weighted and unweighted totals over time, per version/branch/platform. Public reviews. |
| **SLAPorSLOP staff** | Full detail by role, with access logged | Full detail for abuse checks, with access logged |
| **Public** | Nothing | Rating-over-time charts |

**Built 2026-10-20 (votes):** the maker's Manage page shows each player's current vote grouped by the version and platform it was cast on (the desktop app reports the platform at launch), and "update impact" per version (👎 → 👍 and 👍 → 👎 changes, new votes). Branches are separate apps with their own ratings.

## 4. Architecture

```
  SLAPorSLOP client / phone app / website                  Stripe (webhooks)
               │                                            │
               └──────────── HTTPS ─────────────┬───────────┘
                                                ▼
                   ┌──────────────────────────────────────────────┐
                   │  SLAPorSLOP API (stateless, several copies behind  │
                   │  the cloud load balancer, autoscaled)        │
                   │  auth · validation · rate limits ·           │
                   │  idempotency keys                            │
                   └──────────┬──────────────────────────┬────────┘
                       writes │                          │ reads (history, dashboards, charts)
                              ▼                          ▼
                 ┌───────────────────────┐    ┌───────────────────────┐
                 │ PostgreSQL primary    │──► │ PostgreSQL read       │
                 │ payments · vote_events│    │ replica(s)            │
                 │ votes · rollup tables │    └───────────────────────┘
                 └───────────────────────┘
                   + background jobs (rollups every few minutes, nightly re-checks)
```

- **Apps never talk to the database directly.** Everything goes through the API, which checks:
  - Is the user signed in?
  - Have they launched the app (needed to vote)?
  - Is the webhook genuine (for payments)?
  - Rate limits.
- Each vote is written in **one transaction**: a new vote event, the current vote updated, and a counter queued. A retried request with the same idempotency key does nothing.
- **Reads** for charts and dashboards come from **rollup tables** on a read replica, never calculated live for each page view.

### 4.1 Tables (sketch)
```sql
CREATE TABLE vote_events (
  id          uuid PRIMARY KEY,          -- UUIDv7 (time-ordered)
  account_id  uuid NOT NULL,
  app_id      uuid NOT NULL,
  vote        smallint NOT NULL,         -- 1 Slaps, -1 Slop, 0 withdrawn
  prev_vote   smallint,
  release     text NOT NULL,
  branch_id   uuid,
  platform    text NOT NULL,
  playtime_s  integer NOT NULL,
  weight      real NOT NULL,             -- calculated from playtime at the time (§2)
  review_id   uuid,
  idem_key    text NOT NULL,
  created_at  timestamptz NOT NULL DEFAULT now()
) PARTITION BY RANGE (created_at);      -- monthly partitions

CREATE TABLE votes (                    -- current vote
  account_id uuid, app_id uuid,
  vote smallint NOT NULL, weight real NOT NULL, updated_at timestamptz NOT NULL,
  PRIMARY KEY (account_id, app_id)
);

CREATE TABLE app_rating_daily (         -- rollup used by charts and store pages
  app_id uuid, day date, release text, platform text,
  up_count int, down_count int, up_weight real, down_weight real,
  PRIMARY KEY (app_id, day, release, platform)
);
```
The `payments` table is in `PAYMENTS.md` §8. A matching `app_donations_daily` rollup feeds maker dashboards.

Database permissions: the application's database role can only **insert** into `vote_events` and `payments`, not update or delete. Corrections are new rows.

### 4.2 Scaling on a small budget
- **Load:** 1 million monthly users who each vote twice and donate once a month is about 3 million writes a month. That's **1–2 per second** on average and maybe 50/second at peaks. One managed Postgres instance handles this easily.
- **The real load is reads** (ratings on every store page). This is handled by:
  - rollup tables;
  - caching for a few minutes (in the API and at the CDN for public pages);
  - one read replica when needed.
- **Grow in this order, only when needed:** a bigger instance → a read replica → monthly partitions with old ones archived to cheap storage → more replicas. Postgres doesn't need replacing at any of these steps.
- **Hosting:** a managed Postgres from the cloud provider, with automatic backups and point-in-time recovery. The API runs on the provider's container service with autoscaling, so no servers are run by hand. **Ansible isn't needed for this.** Infrastructure is defined in Terraform/OpenTofu (`ARCHITECTURE.md` §7).

## 5. Uses
- The rating-over-time chart (weighted and unweighted), per version, branch and platform.
- **Update impact:** "after 1.4, 212 players changed 👎 → 👍".
- **Abuse checks:**
  - Bursts of votes from new accounts.
  - Coordinated changes of vote.
  - Card-testing patterns in donations (`PAYMENTS.md` §9).
- "For you" recommendations, which can be switched off (`LEGAL.md` §7).
- Maker dashboards and user history pages.

## 6. Privacy and retention
- **Payment records:** kept as long as tax and accounting law requires (typically 7–10 years), encrypted at rest, with access by role.
- **Votes:** kept for the life of the account. When an account is deleted, its votes are **anonymized** (linked to a random ID) so ratings stay honest.
- **Export:** a user's full donation, payment and vote history can be downloaded.
- Public votes by default are explained at sign-up (`LEGAL.md` §7).

## 7. The mockup and the simulation
The real store page shows the weighted and unweighted % Slaps and the rating over time (`web/src/components/RatingPanel.tsx`), from real votes. Invented data lives in two places only: the original mockup (`mockup/`, saved in your browser, and without the weighted figure) and the simulation sandbox (`docs/SIMULATION.md`), which checks these numbers against a known truth.
