# Load test

How much one SLAPorSLOP server takes, measured 2026-10-21 with `crates/slop-api/examples/load_test.rs`
(see its header to run it again). It starts a throwaway SLAPorSLOP on the test database, fills it with
the simulation (400 games, 1,500 players, 120 days of votes and play), and runs each kind of
request with 1, 10, 50 and 100 clients at once for 8 seconds each. Never run it against a real
SLAPorSLOP.

**Machine:** the dev server: 12 cores, 31 GB, with Postgres (in a container) on the same machine
and the live dev SLAPorSLOP running beside it. Release build, no proxy in front.

## Results

| Request | Clients | Requests/s | p50 ms | p95 ms | p99 ms | Errors |
|---|---|---|---|---|---|---|
| Website (index.html) | 1 | 3,095 | 0.3 | 0.4 | 0.5 | 0 |
| Website (index.html) | 100 | 26,102 | 3.5 | 7.3 | 9.6 | 0 |
| Store front (popular) | 1 | 100 | 9.5 | 13.1 | 15.1 | 0 |
| Store front (popular) | 100 | 815 | 125 | 138 | 144 | 0 |
| Store page | 1 | 188 | 4.8 | 8.1 | 11.5 | 0 |
| Store page | 100 | 1,070 | 93 | 108 | 117 | 0 |
| Search | 1 | 629 | 1.6 | 2.6 | 3.2 | 0 |
| Search | 100 | 3,746 | 27 | 32 | 35 | 0 |
| Library update check (20 apps) | 1 | 17 | 57 | 83 | 90 | 0 |
| Library update check (20 apps) | 100 | 88 | 1,167 | 1,207 | 1,212 | 0 |
| Pulse heartbeat | 1 | 1,362 | 0.7 | 1.0 | 1.4 | 0 |
| Pulse heartbeat | 100 | 7,225 | 13 | 19 | 21 | 0 |

Each kind levels off by 10–50 clients: past that, requests wait their turn and latency grows,
but nothing fails.

## What it means

| Load | One server handles about |
|---|---|
| Players in games (a Pulse heartbeat a minute each) | 400,000 at once |
| SLAPorSLOP apps open (an update check every 30 minutes, 20 installed apps per check) | 150,000 at once |
| People browsing the store (a page view every 10 seconds, about 3 requests each) | 3,000 at once |

**What gives first:** the library update check, at about 90 a second. It runs several database
queries per installed app (latest release, pending release, other builds, mods, sandbox
information). Batching those into a few queries per request is the fix when it matters; not
needed before launch.

**Found and fixed by this test:** every response the website's files streamed in pieces waited
about 40 ms for the client's delayed acknowledgement (Nagle's algorithm). The server now sends
at once (`TCP_NODELAY`): index.html went from 41 ms to 0.3 ms, and from 2,700 to 26,000 a second.
