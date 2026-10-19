# Simulation sandbox

A made-up store, for testing **"For you"** recommendations, **ratings** (% Not Slop, weighted and
unweighted, over time) and **player counts** against a truth we know. The code is
`crates/slop-api/src/sim.rs`.

## What it simulates

- **Apps** (default 400), each built around a persona's tags: two thirds games, the rest tools.
  - Stages: 60% released, 25% beta, 15% coming soon. A quarter of the upcoming ones are dead
    projects.
  - Age ratings: 0, 7, 12, 16 and 18+.
  - Each app has a hidden **quality**. 15% of released apps get an update partway through that
    makes them better (two thirds of those) or worse.
  - 70% of games report playtime through Slop Pulse.
- **Users** (default 1,500) with a hidden **taste**:
  - a third are gamers who also use a few tools (one game persona, plus a tool persona at about a
    quarter of the weight);
  - the rest have one persona or two;
  - 10% are in the UK, so they start in the under-18 view.
- **Days** (default 120). On each one, active users:
  - **find apps:** likelier when the app fits their taste and others have it;
  - **play them:** more when they enjoy them;
  - **vote:** 👍 with a probability that rises with quality and fit, after playing at least
    twice;
  - **think again after updates:** some change or withdraw their vote.

  Player counts are recorded per 3 hours, busiest in the evening.

- **People** (`sim_people.rs`): 24 made-up creatives (artists, pixel artists, 3D modellers,
  composers, sound designers, writers, voice actors, testers and studios, including two that
  mostly generate their work with AI) plus 2 whose first portfolio waits for review. Each has a
  hidden **quality** and **reliability**, a page with services, links and a portfolio (pictures
  made on the spot; the sandbox has no audio files).
  - About half the released apps' updates ship **hired work**: the update's true quality moves
    with the creative's quality, and players vote on what they get.
  - The makers hire for their apps in every state: requested, offered, waiting for payment, in
    progress, delivered (some over 7 days ago, finished by the real auto-complete), completed
    (some with the last half still owed), declined, cancelled and withdrawn.
  - Payments go through the real `payment_succeeded_at`, marked **simulated**; credits come from
    the real `maybe_credit`. Clients review most completed jobs, 👍 likelier the better the
    creative.
  - A fifth of requests are **inquiries**, and the creative opens a recorded conversation (with
    fingerprints from the real code). About a third of offers ask for a **sign-off**. Some jobs get
    a **Change to Agreement** (accepted and paid, offered, or requested). Deliveries with money owed
    are **previews**, and the real code sends the **final files** after payment. Some jobs have a
    **problem** (agreed refunds, a mediator's suggestion still open, or closed unresolved), and
    creatives dispute some bad reviews (kept, noted, removed or pending). The real cleanup deletes
    the text of conversations closed over 30 days ago, keeping the fingerprints.

The truth lives in `sim_users`, `sim_apps` and `sim_creatives`. Only the report reads it; the real
code never does.

## Running it

It refuses any database whose name doesn't end in `_sim`.

```sh
export DATABASE_URL=postgres://slop:slop@127.0.0.1:5433/slop_sim
slop-api simulate generate --apps 400 --users 1500 --days 120 --seed 42   # wipes and rebuilds
slop-api simulate report --sample 400                                      # JSON scores
slop-api simulate pulse                                                    # keeps "playing now" alive
```

The test `the_simulation_scores_the_real_code_against_its_truth` (in `tests/api.rs`) runs a
smaller simulation on every test run and checks the guarantees below.

## What the report checks

| Field | Meaning | Expect |
|---|---|---|
| `for_you.hit_rate` | Share of For you cards the user would probably 👍 (truth ≥ 50%) | Above `popular_baseline`, which is above `random_baseline` |
| `for_you.variety` | 1 − tag overlap between cards in one feed | Not far below the baselines |
| `for_you.popularity_percentile` | How mainstream the cards are | Below the popular feed's: room for the long tail |
| `discovery_share` | Share of discovery cards | 0.25 (every fourth card) |
| `discovery_fit` / `discovery_like_chance` | How far outside their taste discoveries are, and how often they'd still be liked | Low fit, but liked a fair share of the time |
| `gamer_with_tools_tool_share` | Gamers who use a few tools: share of tool cards | Above 0, well below half |
| `dislike_roguelike_share_*` | Roguelike fans' roguelike share, before and after 👎 on three | Drops |
| `like_tool_share_*` | Gamers' tool share, before and after 👍 on two tools | Rises |
| `under_18_*` | Under-18 view: personal feeds, 18+ cards | 0 and 0 |
| `shown_dead`, `shown_in_library_or_played`, `shown_own_apps` | Never recommended | 0 |
| `ratings_matching` / `monthly_matching_now` | Current rating and this month's rating equal a recount from raw vote events | All of `ratings_checked` |
| `updates_moved_right_way` | After an update, the rating moved the way quality did | Most of `updates_checked` |
| `rating_truth_correlation` | % Not Slop against true quality | Clearly positive |
| `players_matching` | "Playing now" equals the live sessions made | All of `players_checked` |
| `people.fees_not_5_percent` | Payments whose Slop fee isn't 5% | 0 |
| `people.credit_rule_violations` | Jobs credited without being completed, fully paid and $10+, or the reverse | 0 |
| `people.reviews_not_completed` / `reviews_not_by_client` | Reviews that shouldn't exist | 0 and 0 |
| `people.overall_rating_untouched` | Badged apps whose rating is still every vote (HUM-13) | All of `badge_apps` |
| `people.creative_rating_truth_correlation` | Creatives' % Not Slop against their true quality | Clearly positive; `no_ai_average_percent` above `heavy_ai_average_percent` |
| `people.conversations_failing_verification` | Copies rebuilt from the sandbox that don't match the fingerprints | 0 |
| `people.final_files_before_payment` | Final files sent before the delivery payment | 0 |
| `people.hidden_reviews_counted` | Creatives whose rating counts a removed review | 0 |
| `people.since_new_work_moved_right_way` | "Since the new work" moved the way the hired work moved quality, compared with the votes before | Most of `hired_updates_checked` |

## The sandbox site

`slop-sim.service` runs the API on the `slop_sim` database with `SLOP_SIMULATION=true`. The site
shows a "Simulation sandbox" banner and offers `/sim`, where anyone can **view as** a simulated
user to see their For you feed. The server refuses `SLOP_SIMULATION` on any database not named
`…_sim`, so this can't be switched on in production by mistake.
