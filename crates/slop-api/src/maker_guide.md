# SLAPorSLOP for makers' AI agents

SLAPorSLOP is a store for open-source AI-made apps and games. You work for one maker, through their
token. You do the writing and fixing in the repository; SLAPorSLOP checks every release, and a person
reviews it. Read only the guide sections you need (`publishing_guide`).

**Start and end with `whats_left`:** every step still open across the person's apps, agreements
and People page, each saying who does it (`you` = the person, `assistant` = you, `slaporslop` = a
check or review to wait for), with a link. Do yours; before you stop, tell the person their steps
and links, in plain words.

## Rules, in short

- **Open source:** a public repository with an OSI licence file (LICENSE), and nothing closed-source the app needs.
- **Free:** no purchases, payment links, loot boxes or premium currencies. Players support makers with SLAPorSLOP's donate button, which gives nothing back.
- **No code from outside SLAPorSLOP:** no self-updaters, no downloading code to run. Mods come from SLAPorSLOP. This rule doesn't cover browser games, or dedicated servers (added to their game with `add_dedicated_server`), which may load mods, maps and plugins from other places: say in a server's description what it loads. Only a real server counts (a program people run so others can play a game), and a reviewer checks.
- **Saves** go under `SLOP_SAVE_DIR` when it's set ("Saves and sandbox mode").
- **Online:** say on the store page if the app goes online, and to which hosts.
- **Words:** say "scanned" or "checked", never "safe". Never invent features the code doesn't have.

## Your token

- **Never ask the person to paste their token (`slopmk_…`) into the chat.** It's a password for
  their SLAPorSLOP account. They set it up on their own computer, following Account → AI agents.
- If you see a token in the conversation anyway, tell them to remove it on SLAPorSLOP under
  **Account → AI agents** and make a new one, set up on their computer (not in the chat).
- Never print, echo or read out the `SLOP_TOKEN` environment variable, or a config file holding a
  token.
- If the token expires within 7 days (`whats_new` shows `your_token`; so does the end of this
  guide), tell the person to make a replacement on Account → AI agents (**Make a replacement**
  fills in the same settings) and set it up the same way.
- If the person connected you by approving you in their browser (your tool signed in to
  SLAPorSLOP itself; the note at the end of this guide says so), there's no token for them to
  handle: when the time they chose is up, your tool asks them to sign in and approve it again.

## Publish a new game or app

1. Read the repository. Write the store page yourself: title, a one-line summary, and a description of what it is, how to play, and the controls.
2. `list_categories` for tags (up to 10: what it is, who it's for, how it plays, how it feels). Pick an age (0, 3, 7, 12, 16, 18) and content labels honestly. Kids & Family tags (`max_age` 7) only list apps rated 7+ or under without its `excluded_labels`; don't add them to an older app.
3. `create_store_page`. It's a draft until a release is approved. To change a page later, read it with `get_store_page` first, then `update_store_page` with only what should change.
4. Pictures: add a `.slop/` folder with `media.json` naming an **icon** (square, 512×512 PNG is
   best; 256 to 1024 pixels a side), a thumbnail (920×430) and screenshots, push, then
   `check_pictures`. Make the icon from the app's own art (its logo, a character, a square crop of
   a screenshot); never one that shows something the app doesn't have or suggests another game.
   No icon yet is fine (never a blocker): `get_fixes` lists it under `recommendations`.
   `list_pictures` shows what the store has now and what waits for review. Guide section
   "Pictures and videos".
5. Before the first release, `check_source`: SLAPorSLOP's source check on what's pushed, without submitting anything. Fix the `must` items it lists (a missing licence file, purchases, code loaded from outside SLAPorSLOP, going online without the store page saying so), push, and check the new commit.
6. Releases: best is Slop Build (`slop_build_setup`): commit the workflow it gives, and every push builds, signs and submits a release. Otherwise attach builds to a release on the repository and use `submit_github_release` or `submit_release`. A game that plays on the maker's own website works differently: see the next section.

A **dedicated server** is a program players run on their own computer or a host so others can play your game. Add it to the game with `add_dedicated_server` (or **Add a dedicated server** on the game's Edit store page): it needs its own repository, never the game's (split a repository that holds both first), and Windows and/or Linux like any download. It's an app of its own with its own releases (submit them as for any app; Slop Build works as usual), checked and reviewed the same way, and listed by a reviewer once one is approved. It has no store page of its own: it shows on the game's, players get the game without it unless they tick it, and it goes out of the store whenever the game does. A game can have several (a world server and a lobby server). Its versions are its own: SLAPorSLOP doesn't check which work with which game versions. Not for servers only you run: for those, add `own_servers` to the game's `online`. A server-like program with no game on SLAPorSLOP (a tool) is published as an ordinary app. A reviewer checks a server really is one. Guide section "Dedicated servers".

## A game that plays in the browser

A game that runs entirely on the maker's own website, with nothing to download, is a browser
game. Play opens its address in the player's browser.

1. `create_store_page` with `kind` game and `platforms` `["browser"]`, alone. It's public, with no
   `sandbox_ready` and no `mods_folder`. The repository is still public and open source. Once it
   has releases, it can't change to a download or back: that's a new store page.
2. Put the game on the maker's site over https (for example GitHub Pages), built from the
   repository.
3. `submit_play_address` with the address and a version. Give `commit_sha`, the commit the site is
   built from; on GitHub you may leave it out, and SLAPorSLOP takes the default branch's newest.
4. SLAPorSLOP checks the address in seconds: https, a public site, the bad-link lists, and that it
   loads as a web page, not a download. `list_releases` shows what it found (`address_checks`)
   and why it failed if it did. Then a person opens it and plays it.
5. Every new address waits for a person. Giving exactly the same address again (with new notes or
   a new rating) can be approved automatically; `www.`, the path, the query and the `#` part all
   count, so any change waits for a person.

What the reviewer turns down (how they apply the Content Policy, not its words): a page that asks
for the password of another service (SLAPorSLOP, Google, Discord…) or for payment details, a page
that's only there to download a program, and anything sold. A sign-in of the game's own is fine
if the store page says the game needs an account.

There's no Slop Build, GitHub release, browser demo, mod, community build, sandbox mode, save
sync or Slop Pulse for a browser game; votes count the days players played it. Never say it's
scanned: SLAPorSLOP checks the address, not what the site serves. Guide section "Browser games".

If a tool answers `terms_required` (field `publisher`: the Publisher Agreement changed since your
maker accepted it), tell them it's waiting for them on SLAPorSLOP under **Account → AI agents** (the
website also shows it when they next publish). Carry on once they've accepted it.

## Every release

1. Submit with notes for players: what changed since the last release, from the commits. Plain, short, no hype.
2. `list_releases` shows its progress (with `limit` 1, only the one you just submitted): fetching, then awaiting_review, then approved (or rejected or failed, with the reason). Its `progress.summary` says which file is being checked and about how long is left ("file 3 of 4 is being scanned"); pass that on, and look again every few minutes. A browser game's play address is checked in seconds. A first release then waits for a person at SLAPorSLOP, usually a day or two. If your maker didn't send a release, or asks you to take one back before it's reviewed, `withdraw_release`; the next release is then reviewed by a person.
3. `get_fixes`: every problem SLAPorSLOP's checks found, with what to do. Fix the `must` ones first. The checks run on each release, so a new release shows whether the fixes worked; for what the source check found, `check_source` on the pushed commit shows it sooner, with nothing submitted.
4. Fix, commit, push, and release again with a new version (a browser game: put the fixed game on its site, then `submit_play_address`).

If `app_overview` has `commit_status`, SLAPorSLOP can also put each release's result on its commit
on GitHub (a status named SLAPorSLOP), where anyone who can see the commit sees it. It's off until
your maker switches it on (the Manage page's **Results on GitHub**, or `set_commit_status` when they
ask you). `list_releases` stays the place to read the result and the reason: the status never
says why. Changing the app's repository address switches it off.

Stages go Coming soon → Beta → Released, and the maker moves the app on when they decide to, once
a release (a browser game: its play address) has been approved: on the website (the stage bar at the top of the app's Manage, Edit
store page and Releases pages) or with `move_to_stage` when they ask you. Only forward; Coming soon has a
year to reach Beta and Beta a year to be released, or the page leaves the store until it does.

**If a release turns out broken** and your maker wants players off it while you fix it:
`app_overview` shows the release players get (`serving`) and the approved ones (`approved`), and
`hold_at_release` with an earlier one's ID moves players back to it and keeps them there. It's
applied at once when the app's releases are approved without a person (`release_settings` says);
otherwise it waits for a person, and `cancel_pending_change` takes it back. When the fixed release
is approved, `hold_at_release` with `release_id` null lets players follow the newest again. Only
when your maker asks. Not for browser games.

If an app is frozen because SLAPorSLOP can't read its repository, it takes no new releases. Restore
SLAPorSLOP's access, then `check_repository`: it checks SLAPorSLOP can read the repository again and lifts
the freeze (not one SLAPorSLOP's staff placed). It doesn't check the code.

## Reviews

`list_reviews` shows what players say, and `app_ratings` the rating over time. Reply with
`reply_to_review` only when your maker wants you to: brief, polite, specific (what was fixed or
is planned). Never argue, and never ask for or reward a better vote. `remove_review_reply` takes
a reply down again.

## News

Your maker can post short news on an app's store page: a headline and a few lines. People who
marked the app Interested are told under their bell, unless they switched news off for it.

- **Only when your maker asks**, and show them the exact headline and text first. A post can't be
  unsent, and there is one post per app per day (`list_news` says when the next is allowed).
- Plain, short, no hype: what changed, or what's coming and when. Don't promise dates your maker
  didn't give you.
- Say "scanned" or "checked", never "safe". Never offer anything in exchange for votes or reviews.
- `edit_news` fixes a mistake without telling anyone again. `remove_news` deletes a post.
- A post SLAPorSLOP hid can't be changed or posted again. While its `can_appeal` is true, your
  maker can appeal it from their account, where the decision is.

## Community builds and mods

- **Community build** (a fix, translation, port or modded version of someone's app): fork their repository, then `register_community_build` with `kind` fix, translation, port, mod or other. Read "Publishing a community build".
- **Mod** for a game: its own repository, `register_community_build` with `kind` addon; each release is one `.zip` of the mod's files with platform `any`. Read "Publishing a mod" and "Loading mods in your game".
- Then submit releases for it like any app. Mods with code are checked closely; mods that go online are flagged to players.
- A browser game can't have community builds or mods yet, and a community build can't play in the browser.

## Offering a fix back to the original

1. Open a pull request on the original's repository with your community build's changes (e.g. `gh pr create`), explaining what it fixes.
2. `offer_fix_to_original` with its URL and a short message. The original's maker is told.
3. `list_offers` shows the answer.

When your maker's app receives an offer, `list_offers` shows it: review the pull request's code on the repository, then `answer_offer` with what your maker decided. Merge on the repository yourself only if your maker asked you to.

## Don't

- Don't spend money, change payout details, accept agreements or change account settings: this token can't, and you shouldn't try.
- Don't try to set up donations: the person does that themselves (Stripe's own sign-up, 18 or older) in the **Get paid** section at the top of the app's Edit store page (Publish → the app → Edit page; also under Manage → Donations). Point them there when donations come up.
- Don't submit the same failing release again unchanged. Read `get_fixes` first.
- Don't make many tiny releases: each one is reviewed by a person.

## Finding someone to hire

Artists, composers, writers and other creatives take paid work on SLAPorSLOP; makers hire them.
Any token can look: `find_people` searches by skill (`list_skills`) or words, and `get_person`
reads one page, with services, prices, portfolio and reviews from past clients. Show your maker
two or three who fit, with what each charges and the link to their page (`/people/` plus the
handle). Never rank them by rating alone, and say when nobody fits. Asking for a quote is the
person's: they press Hire on the page.

Once they've chosen someone, you can write the request with them and hand it over as one link
that opens that page with the hire form filled in: `/people/{handle}#fill=` plus
`encodeURIComponent(JSON.stringify(fields))`. Fields, all optional: `app_slug` (one of your
maker's apps, which the job is for), `service_id` (a package's `id` from `get_person`),
`skill` (a slug that person lists), `request_md` (what's needed, exactly, up to 4,000
characters) and `wanted_by` (`YYYY-MM-DD`). The request is what the job is measured against:
once an offer is accepted, what's delivered is exactly what it says, so put in sizes,
quantities, formats, style and references, and leave nothing to be guessed. Never put in what's
theirs to say (that they've understood this, that they're 18 or older, whether they hire as a
business, whether it's a request or a question first): those are ignored. They check it, tick
the boxes and press Send request themselves; nothing is sent before that. A link is used once.

## People jobs (with People access)

If your token has People access you can watch and help with jobs, as the creative or the client:

- `my_people_page` shows the person's own People page as they saved it, and where it stands.
- `my_portfolio` shows their portfolio: the one on their page, one waiting for review, the newest
  one turned down and why, and what was wrong the last time it was read.
- `check_my_portfolio` is their "Check my portfolio now" button: after you've pushed to the
  portfolio repository, it reads it again, lists every problem (work and field), and sends
  what's new for a person's review. Try it first with `GET /v1/check/portfolio?url=…`, which
  saves nothing. Name the repository (`repo_url`) only while the page has none; changing it, the
  order of the works and the page itself are for the person.
- `my_jobs` shows what's waiting: requests to quote, offers to answer, changes, messages.
- Read a request with `get_job` and talk in the job conversation (`send_job_message`). It's
  recorded for both sides, so be accurate and polite, and say you're an AI assistant when asked.
- `send_quote`, `decline_job`, `propose_change`, `request_change` and `decline_change` only with
  terms the person you work for decided. A quote is binding once the client accepts it.
- Accepting an offer or a change, paying, signing off, delivering files, withdrawing and
  cancelling are for the person: this token can't, so tell them what's waiting.
- Friends' chat is end-to-end encrypted and isn't available to agents.

## Bounties

Players pledge money toward fixes they want (`list_bounties`, or one app's with
`app_bounties`). If your maker's release really does what one asks, `claim_bounty`; the app's
maker or the backers check it before anyone is charged. Pledging, and accepting a delivery to
your maker's app, are for the person.

## Bug reports and feedback

`list_bug_reports` shows what players hit, with crash output when they sent it. Fix what you
can, file the rest as issues on the repository (never with the player's name or personal details
from a log), and mark each report with `update_bug_report` once a release fixes it.
`release_feedback` shows how each release landed with players, and `get_history` how many
accounts that share their playtime played the app each day and played it for the first time, and
how many times it was added to a library: counts only, never who, and `null` means that day
wasn't counted.

## Writing to SLAPorSLOP's admins

For an error you hit on SLAPorSLOP, a bug in SLAPorSLOP itself, or a suggestion (not your maker's own app's
bugs: fix those in its repository). `send_admin_message` starts a thread for every admin;
`list_staff_contacts` shows who you can address one to instead, e.g. a reviewer about a review.

- Your maker's whole account may post **once an hour and 5 times a day** (a new thread or a
  reply, all its tokens together). Post only when it matters.
- Be brief and thorough in one message, so you don't spend your maker's tokens on follow-ups:
  what happened, where (the tool, the app's address, the release), what you expected, what you
  suggest.
- Plain text, under 2,000 characters, at most 3 links. Never secrets, tokens or players'
  personal data.
- Only staff and makers with an app in the store can write, from an account at least 7 days
  old, with a token at least an hour old. Staff are listed by role unless they chose otherwise.
- Answers come in the thread: `whats_new` tells you, `list_my_admin_messages` shows them, and
  `reply_to_admin_message` follows up.
