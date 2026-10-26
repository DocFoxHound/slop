# Your Slop portfolio

Slop's **People** pages are where artists, animators, composers, sound designers, writers, voice
actors and testers offer their services to the people making games. Your People page is your
**shop window**: it shows makers who might hire you who you are, your style, your best work and
your rates. The **portfolio** is the "best work" part, and your page can't go public without one.

This page says how to build a portfolio, everything Slop checks, and how to send it for review.
It's written so that a person or an AI assistant can follow it from start to finish.

> **Using an AI assistant?** Give it this line: *"Read https://slopai.games/portfolio.md and help
> me build my Slop portfolio."* (Use the address of the Slop site you're on; this page is
> `/portfolio.md` on every Slop site.) Agents: **this page is everything you need.** Nothing on
> the person's Slop screens is meant for you: no token, no code, no other line to paste, so don't
> ask for one. The [checklist](#checklist-for-agents) near the end lists every step in order, and
> [what to ask the person](#what-to-ask-the-person).

## What a portfolio is

- A **public GitHub repository of your own** (for example `github.com/you/portfolio`) with a
  folder called `.slop/` at its top level.
- In that folder, a file called **`.slop/portfolio.json`** lists your works: pictures, tracks
  and YouTube videos, each with an optional title, caption and skill. It can also name an
  **avatar** and a **banner** for your page.
- Your pictures and tracks sit in the same `.slop/` folder. Videos stay on YouTube; you only
  link them.

Slop doesn't copy or host your files. It reads `portfolio.json` once, checks every file, and
then shows your pictures and plays your tracks straight from your repository, **at the commit it
read**. Pushing changes later doesn't change your page until you check again and the new
portfolio is approved.

Your rates aren't in the portfolio: you set them as **services** on your page's editor
(`/people/me`), with a "from" price, a currency and typical days for each.

## Before you start

1. **A People page.** On Slop, go to **People → Offer your services** (`/people/me`), describe
   yourself, add your services and choose **Create my page**. People is for adults (18+). The
   **Portfolio** section appears in the same editor once the page exists.
2. **A GitHub account linked to your Slop account.** The portfolio repository must belong to it.
   The editor's Portfolio section shows which GitHub account is linked, or a **Link GitHub**
   button if none is. (A repository owned by a GitHub organization works if you're one of the
   organization's owners and Slop's GitHub App is installed on it.)

## Step by step

1. **Make a new public repository** on GitHub, e.g. `portfolio` (github.com/new). Public, because
   visitors' browsers load your pictures and tracks from it.
2. **Start from a template** if one fits. Slop has four ready-made portfolio repositories that
   already pass every check, with placeholder pictures to replace:
   [examples/portfolio-templates](https://github.com/DocFoxHound/slop/tree/main/examples/portfolio-templates)
   - **gallery**: illustration, concept art, pixel art (pictures)
   - **showreel**: animation and video (YouTube videos with posters)
   - **tracklist**: music and sound (audio tracks with cover pictures)
   - **studio**: teams offering several services (a mix)

   Copy the template's `.slop/` folder into your repository, or write `.slop/portfolio.json`
   from the [example](#a-worked-example) below.
3. **Add your work** to the `.slop/` folder: your best pictures and tracks, and the YouTube links
   to your videos. Fewer, better works beat many average ones; put your best first.
4. **Doll it up.** A banner and an avatar make the page yours. Give every work a title, a one-line
   caption that says what it was for and what you did, and the skill it shows.
5. **Commit and push** to the repository's default branch (`main`, usually). Slop reads the newest
   commit there.
6. **On Slop**, open your page's editor (`/people/me`), go to **Portfolio**, paste the
   repository's address (`https://github.com/you/portfolio`) and choose
   **Check my portfolio now**.
   - If something needs fixing, Slop lists every problem, one per line, each naming the field or
     the work (`work 3 "image": …`), what's wrong and the limit. Fix them, push, and check again.
   - If it all passes, the portfolio **waits for review**. A person at Slop looks at every work,
     and you get a notification when it's approved or turned down (with the reason, so you can
     fix it and check again).
7. **Approved:** your page goes public on People. Then set up payments under **Get paid** on the
   same editor so makers can hire you.

Later changes work the same way: push, then **Check my portfolio now**. Everyone else keeps seeing
the approved portfolio until the new one is approved. You see the new one on your own page right
away, marked **Preview** (only you can see it), with a **View your live page** link to the page as
everyone else sees it. You can put works in a different order in the editor at any time; that isn't
reviewed.

## `.slop/portfolio.json`

One JSON object, at most **64 KB**. Slop refuses any key it doesn't know, at the top level and in
each work, so check the spelling.

```
your-portfolio/
  README.md            (anything you like; Slop doesn't read it)
  .slop/
    portfolio.json
    avatar.png
    banner.png
    works/
      harbour.png
      knight-sprite.png
    tracks/
      main-theme.ogg
    covers/
      main-theme.png
```

### Top level

| Key | Required | What it is |
|---|---|---|
| `avatar` | No | Path to your picture: your face, logo or mark. At least **128×128**; square is best (e.g. 512×512) |
| `banner` | No | Path to the wide picture across the top of your page. At least **1200×300**; about **1600×400** is best. It's shown about 180 pixels tall across the page and cropped to fit, so keep the important part in the middle |
| `works` | Yes | A list of **1 to 30** works, in the order you'd like them shown first |

### Each work

Every work has **exactly one** of `image`, `youtube` or `audio`.

| Key | What it is |
|---|---|
| `image` | Path to a picture. At least **32×32** (pixel art is shown scaled up with crisp edges) |
| `youtube` | A link to one YouTube video (forms below) |
| `audio` | Path to a track: MP3, Ogg (including Opus), WAV, FLAC or M4A, up to **15 MB** |
| `poster` | Optional, for `youtube` and `audio` works only: a picture shown with it (at least **32×32**). Without one, a video shows a card with its title and a track shows a ♪. Not allowed on `image` works |
| `title` | Optional, one line of up to **80** characters. Without one, the work is called "Picture", "Video" or "Track" |
| `caption` | Optional, one line of up to **200** characters |
| `skill` | Optional, one of the skills below |

**Skills:** `2d-art`, `pixel-art`, `3d`, `animation`, `ui-ux`, `music`, `sound`, `writing`,
`voice`, `localization`, `playtesting`. (A Slop site lists them at `/v1/people/skills`.)

### Paths

- Every path is **relative to the `.slop/` folder**, and the file must be inside it:
  `"works/harbour.png"` means `.slop/works/harbour.png`. A leading `./` is ignored.
- Letters, numbers, `.`, `_`, `-` and `/` only: **no spaces**, no `..`, no leading `/`, no empty
  parts (`a//b`). Up to 200 characters.
- Pictures end in `.png`, `.jpg`, `.jpeg`, `.webp` or `.gif`; tracks in `.mp3`, `.ogg`, `.oga`,
  `.opus`, `.wav`, `.flac` or `.m4a` (upper or lower case).
- Paths are case-sensitive on GitHub: `Knight.png` and `knight.png` are different files.

### Pictures

- PNG, JPEG, WebP or GIF, read from the file itself (a JPEG renamed to `.png` is still fine; a
  text file renamed to `.png` isn't).
- Up to **8 MB** each, and at most **8192** pixels on each side.
- Minimums: avatar **128×128**, banner **1200×300**, works and posters **32×32**.
- The same picture can be used more than once (for example as two posters).

### Tracks

- MP3, Ogg, WAV, FLAC or M4A, recognised from the file's first bytes, up to **15 MB** each.
- Nothing is loaded until a visitor presses play. For long pieces, a strong excerpt is better than
  the full track.

### YouTube links

Any of these, with the video's 11-character ID:

- `https://www.youtube.com/watch?v=VIDEO_ID` (also `m.youtube.com` and `music.youtube.com`)
- `https://youtu.be/VIDEO_ID`
- `https://www.youtube.com/shorts/VIDEO_ID`, `/embed/VIDEO_ID`, `/live/VIDEO_ID`
- `https://www.youtube-nocookie.com/embed/VIDEO_ID`

Playlists and channels aren't works: link one video each. Extra parts like `&t=30s` are fine.
Videos play in YouTube's privacy-enhanced player, only when a visitor presses play. The video
must be public or unlisted for visitors to see it.

### Other rules

- **Each work once:** the same picture path, track path or YouTube video can't be listed twice.
- **Your own work.** Only list work you made or had a real part in, and say what your part was
  in the caption when it was a team effort. Work you made for a client can usually be shown, but
  check your agreement.
- **Your AI use** is set on your page (No AI, AI-assisted or Mostly AI-generated), not in the
  portfolio. Every way of working is welcome on Slop; just say honestly how you work, and let the
  works match.
- Slop's content rules apply: nothing sexually explicit and no advertising, and titles and
  captions count as content too. A person reviews every new portfolio.
- Slop reads the repository's **default branch**, at its newest commit, when you press **Check
  my portfolio now**.
- Keep the repository public, and don't delete or rename files that are live: your page loads
  them from GitHub.

## A worked example

A pixel artist who also makes chiptune music:

```json
{
  "avatar": "avatar.png",
  "banner": "banner.png",
  "works": [
    {
      "image": "works/harbour.png",
      "title": "Harbour at dusk",
      "caption": "Key art for a cosy fishing game. 320×180, scaled up.",
      "skill": "pixel-art"
    },
    {
      "image": "works/knight-sprite.png",
      "title": "Knight sprite sheet",
      "caption": "Idle, walk, attack and hurt: 48 frames at 32×32.",
      "skill": "pixel-art"
    },
    {
      "youtube": "https://youtu.be/VIDEO_ID",
      "poster": "covers/reel.png",
      "title": "Animation reel",
      "caption": "Sixty seconds of sprite animation from three shipped games.",
      "skill": "animation"
    },
    {
      "audio": "tracks/main-theme.ogg",
      "poster": "covers/main-theme.png",
      "title": "Main theme",
      "caption": "Chiptune title music, loops at 1:20.",
      "skill": "music"
    }
  ]
}
```

With the files at `.slop/avatar.png`, `.slop/banner.png`, `.slop/works/harbour.png`,
`.slop/works/knight-sprite.png`, `.slop/covers/reel.png`, `.slop/tracks/main-theme.ogg` and
`.slop/covers/main-theme.png`.

## When the check finds a problem

Every problem names where it is and what to do. Some examples:

| Slop says | What to do |
|---|---|
| `"avatar": me.png is 64×64; it needs to be at least 128×128` | Use a bigger picture |
| `work 2 "image": .slop/works/knight.png isn't in the repository at that commit` | Check the path and its case, that the file is inside `.slop/`, and that it's pushed to the default branch |
| `work 3 needs exactly one of "image", "youtube" or "audio"; it has "image" and "youtube"` | Split it into two works, or remove one |
| `work 4 "youtube": "…" isn't a YouTube video link; …` | Use one video's link, in a form listed above |
| `work 5 "skill": "art" isn't one of the skills; use one of …` | Pick a skill from the list |
| `work 1: "title" can be up to 80 characters; this one has 95` | Shorten it; put the rest in the caption |
| `work 6 is the same as work 2 (image:works/a.png); list each work once` | Remove the repeat |
| ``.slop/portfolio.json isn't valid: unknown field `img`, expected one of … at line 4 column 12`` | Fix the JSON at that line (here: `image`, not `img`) |
| `There's no .slop/portfolio.json on that repository's default branch yet` | Add it, push, and check again |
| `link the GitHub account that owns this repository (…)` | Press **Link GitHub** next to the message, then check again |
| `this is the portfolio that was turned down: …` | Change what the reviewer asked for first |

## What to ask the person

Only what the repository needs:

- **The GitHub account** that will own the repository (it must be the one linked to their Slop
  account), and whether to make a new repository or use one they have.
- **What they do** and the work they want to be hired for, to pick a template and skills.
- **Their works**: the pictures and tracks (files they give you or that are already in a
  repository) and the YouTube links to their videos, best first, with titles and captions if
  they have them. An avatar and a banner if they want them.
- **If you build a page design** (below): the look they want, in their own words.

Not for you to collect: their name on Slop, rates and services, AI-use label, location and
whether they're taking work are set by the person in the editor (`/people/me`), not in the
repository. Offer to suggest wording for them if they'd like, but don't wait for them.

## Checklist for agents

To help someone build their portfolio, in order. Steps marked **(person)** need the person
themself: Slop's agent tokens can't check or send a portfolio, and there's no token or code to
ask them for.

1. Ask what they do, what kind of work they want to be hired for, and which works they're proudest
   of. Suggest a template: gallery (pictures), showreel (videos), tracklist (audio) or studio (a
   team or several skills).
2. **(person)** Make sure they have a People page (`/people/me`) and that the GitHub account that
   will own the repository is linked to their Slop account (the editor's Portfolio section shows
   it, or a **Link GitHub** button).
3. Create or use a **public** GitHub repository they own, e.g. `portfolio`, and a `.slop/` folder
   at its top level. Copy the chosen template's `.slop/` folder if they picked one.
4. Put their files in `.slop/`, with paths of letters, numbers, `.`, `_`, `-` and `/` only.
   Replace **every** placeholder picture, track and `REPLACE_ME…` YouTube link from a template;
   reviewers turn down placeholders.
5. Check every picture's real format and size (e.g. `file`, or a few lines of Python with Pillow):
   PNG/JPEG/WebP/GIF, up to 8 MB, up to 8192 px a side; avatar ≥128×128, banner ≥1200×300,
   works and posters ≥32×32. Check every track is MP3/Ogg/WAV/FLAC/M4A and up to 15 MB. Shrink
   or convert what isn't.
6. Write `.slop/portfolio.json`: 1 to 30 works, each with exactly one of `image`, `youtube` or
   `audio`; `poster` only on videos and tracks; titles ≤80 and captions ≤200 characters on one
   line; skills from the list; nothing listed twice; no other keys. Validate it as JSON.
7. Make it good, not just valid: best work first, a title and a caption on every work saying what
   it was for and what the person did, a banner and an avatar, and the skills they want to be
   hired for. Ask them before writing anything about clients or projects.
8. Commit and push to the default branch.
9. **(person)** In the editor's **Portfolio** section, paste the repository address and choose
   **Check my portfolio now**. If Slop lists problems, they can paste them to you: each line names
   the work and field. Fix, push, and check again.
10. **(person)** Wait for review; set up **Get paid** meanwhile, so makers can hire them once the
    page is live.

## Your page's design

Your People page can have a design of your own: your layout, colours, fonts and words, around
your works. It's optional; without it your page uses Slop's standard layout.

**Where it goes.** Two files in the same repository as `.slop/portfolio.json`:

| File | Up to | What it is |
| --- | --- | --- |
| `.slop/page/index.html` | 64 KB | The page: HTML, written as a fragment (no `<html>`, `<head>` or `<body>` needed) |
| `.slop/page/style.css` | 32 KB | Its style (optional) |

Both must be UTF-8 text. `style.css` without `index.html` is an error.

**How it's checked.** "Check my portfolio now" reads both files at the same commit as your
portfolio, and they're part of it: the design is reviewed and approved together with your works,
and what was reviewed is what shows. Changing the design means a new portfolio to review; until
it's approved your page keeps showing the approved one. Slop rebuilds both files from what's
allowed below and **removes everything else**, listing each removal as a warning (file, line,
what and why) in your editor, above a preview of the result. CSS that hides something
(`display: none`, `opacity: 0`, tiny or transparent text, moving it far out of the box) is kept
but noted too, so reviewers check what's hidden. Things that can't work (a picture
that isn't in the repository, a `<slop-work>` that isn't one of your works, a file over its
limit) are errors: nothing is sent for review until they're fixed.

**Where it shows.** Your page is, from the top: Slop's header (your name, avatar, AI-use badge,
rating, Hire and Report), then your design in its own box, then Slop's Services (always: your
services and their Hire buttons are never inside a design), Reviews and "Worked on". Your design
can't change, cover or hide anything outside its box, and the page's CSS doesn't touch your
design. It must work at phone width (about 360 pixels): use `max-width`, `flex-wrap`, grids with
`minmax()` and width queries (`@media (max-width: 600px)`, which is about your design's box, see
below). The desktop app shows the same page.

### HTML

Allowed tags: `section`, `div`, `header`, `footer`, `article`, `aside`, `h2`–`h6`, `p`, `span`,
`strong`, `em`, `b`, `i`, `u`, `s`, `small`, `mark`, `code`, `pre`, `sub`, `sup`, `br`, `hr`, `ul`,
`ol`, `li`, `dl`, `dt`, `dd`, `blockquote`, `figure`, `figcaption`, `table`, `caption`, `thead`,
`tbody`, `tfoot`, `tr`, `th`, `td`, `a`, `img`.

- No `<h1>`: your name is the page's `<h1>`, in Slop's header. An `<h1>` becomes `<h2>`.
- Other tags are taken out, keeping what's inside them (`<nav>`, `<main>`, `<form>`, `<font>`…).
  Scripts, styles, frames and objects, form fields and buttons (`<input>`, `<button>`,
  `<select>`, `<textarea>`…), `<svg>`, `<math>`, `<canvas>`, `<video>`, `<audio>`, `<picture>`,
  `<meta>`, `<base>`, `<link>`, `<title>`, `<template>` and `<dialog>` are removed with what's
  inside them. Comments are dropped.
- Attributes: `class` on any tag (up to 8 classes; letters, digits, `-` and `_`, up to 40
  characters, not starting with a digit) and `lang`; `href` on `a`; `src` and `alt` on `img`;
  `start` and `reversed` on `ol`; `colspan` and `rowspan` (1–50) on `td`/`th`; `scope` on `th`.
  Nothing else: no `id`, `style`, `on…` handlers, `data-…`, `aria-…`, `role`, `tabindex`,
  `srcset`, `target` or `rel`. A picture's `width` and `height` come from the file itself.
- Links are full `https://` or `mailto:` addresses. Slop opens them in a new tab with
  `rel="nofollow noopener noreferrer ugc"`. Other links are removed (the text stays).
- Pictures (`<img src>`) come from your repository, at the same commit, with paths as a browser
  would read them from `.slop/page/`: `images/hero.png` is `.slop/page/images/hero.png`,
  `../works/knight.png` is `.slop/works/knight.png`; nothing above `.slop/`, no addresses, no
  `data:`. They're checked like your works: PNG, JPEG, WebP or GIF, up to 8 MB and 8192 pixels a
  side. Give each one `alt` text saying what's in it (`alt=""` if it's only decoration).
- Your works, plus the pictures and tracks the design adds that aren't already works, can be up
  to 30 files.
- Invisible direction-changing and control characters are removed from text.
- At most 2,000 elements, nested at most 32 deep.

### Slop's parts

These tags are replaced by Slop's own components, which keep Slop's look, behaviour and
accessibility (your CSS can't restyle them). Write them with a closing tag, empty: HTML has no
self-closing custom tags (anything inside one is moved after it, with a warning).

| Tag | Attributes | Shows |
| --- | --- | --- |
| `<slop-works></slop-works>` | none | All your works as a grid, in the order you set in your editor; pictures open large |
| `<slop-work ref="1"></slop-work>` | `ref`: the work's number in `portfolio.json`'s `works` list, counting from 1 | One work |
| `<slop-video youtube="https://youtu.be/…" title="Showreel"></slop-video>` | `youtube`: a YouTube link or 11-character video ID; `title` (optional, up to 80 characters) | A YouTube video, played from youtube-nocookie.com when pressed |
| `<slop-track src="tracks/theme.ogg" title="Main theme"></slop-track>` | `src`: an MP3, Ogg, WAV, FLAC or M4A file up to 15 MB, a path like an `<img>`'s; `title` (optional) | Slop's audio player |

`<slop-works>` shows once. Slop's parts can't be inside a link. There's no part for your
services: they always appear below your design, where Slop shows them (a `<slop-services>` tag
is removed, with a warning).
To size or place one, wrap it: `<div class="feature"><slop-work ref="1"></slop-work></div>`.

### CSS

Your design sits inside a box whose outermost element is `<div class="page">`: style `.page` for
the whole area (its background, padding, default font). Selectors, rules and values are checked
one by one; anything not listed here is removed.

- **Selectors:** the allowed tags, classes (`.name`), `*`, combinators (` `, `>`, `+`, `~`),
  `:hover`, `:focus`, `:focus-visible`, `:focus-within`, `:active`, `:first-child`,
  `:last-child`, `:only-child`, `:first-of-type`, `:last-of-type`, `:only-of-type`, `:empty`,
  `:not()`, `:is()`, `:where()`, `:nth-child()`, `:nth-last-child()`, `:nth-of-type()`,
  `:nth-last-of-type()`, and `::before`, `::after`, `::first-letter`, `::first-line`, `::marker`,
  `::selection`. No ids, attribute selectors, `html`, `body`, `:root`, `:host`, `slop-…` or
  nesting.
- **Rules:** style rules, `@media` (with `screen`, `print`, `all`, `and`, `not`, `only`, `or` and
  features such as `(max-width: 600px)` or `(prefers-color-scheme: dark)`) and `@keyframes`. No
  `@import`, `@font-face` or any other at-rule. A query only about width (`(max-width: 600px)`,
  `screen and (min-width: 900px)`) is about **your design's box**, not the whole screen (Slop
  turns it into a container query): your design is never as wide as the screen, and this way the
  reviewers' phone-width preview shows what a phone shows.
- **Properties:** text (`color`, `font-family`, `font-size`, `font-weight`, `font-style`,
  `font-variant…`, `line-height`, `letter-spacing`, `word-spacing`, `text-align`,
  `text-decoration…`, `text-transform`, `text-shadow`, `text-indent`, `text-wrap`,
  `white-space`, `word-break`, `overflow-wrap`, `hyphens`, `vertical-align`, `list-style…`,
  `quotes`, `content`, `counter-reset`, `counter-increment`, `columns`/`column-…`,
  `image-rendering`); backgrounds (`background` and every `background-…`); spacing and size
  (`margin…`, `padding…`, `width`, `height`, `min-`/`max-` sizes, `box-sizing`, `aspect-ratio`,
  `overflow`, `display`, `visibility`, `opacity`, `float`, `clear`, `object-fit`,
  `object-position`); borders and effects (`border…`, `border-radius…`, `border-image…`,
  `outline…`, `box-shadow`, `filter`, `backdrop-filter`, `mix-blend-mode`, `clip-path`,
  `isolation`); layout (`position`, `top`/`right`/`bottom`/`left`, `inset`, `z-index`, `flex…`,
  `grid…`, `gap`, `order`, `align-…`, `justify-…`, `place-…`, table properties); movement
  (`transform…`, `translate`, `rotate`, `scale`, `perspective…`, `transition…`,
  `animation…`); `cursor` (keywords only).
- **Values:** keywords, numbers, lengths (`px`, `em`, `rem`, `%`, `vw`, `vh`, `ch`, `fr`…),
  angles, times, `#rgb`/`#rrggbb` colours, and the functions `rgb()`, `hsl()`, `hwb()`, `lab()`,
  `lch()`, `oklab()`, `oklch()`, `color-mix()`, the `linear-`/`radial-`/`conic-gradient()` families,
  `calc()`, `min()`, `max()`, `clamp()`, transform functions, `cubic-bezier()`, `steps()`,
  `repeat()`, `minmax()`, `fit-content()`, filter functions, `counter()`, `counters()` and shapes
  (`inset()`, `circle()`, `ellipse()`, `polygon()`). Quoted text only in `content`, `quotes`,
  `grid-template-areas` and `list-style-type`.
- **Pictures:** `url()` only in `background`, `background-image`, `border-image` and
  `list-style`, naming a picture in your repository like an `<img>` does (`url(images/tile.png)`).
- **Not allowed:** `position: fixed` and `sticky` (`static`, `relative` and `absolute` are fine);
  `z-index` beyond ±100, and functions (like `calc()`) in `position`, `display` and `z-index`;
  `blur()` over 40px; `!important` (dropped, the rest kept); custom properties (`--x`) and
  `var()`; `attr()`, `env()`, `expression()`, `image-set()`; `url()` anywhere else (so no cursor
  pictures and no `content: url()`); browser-prefixed properties (`-webkit-…`); the `font`
  shorthand (use `font-family`, `font-size`, `font-weight`, `line-height`).
- **Movement:** people whose system asks for reduced motion see no animations or transitions:
  Slop turns them off, so don't hide anything behind one.

### Fonts

`font-family` can name these, which Slop serves itself (no requests to other sites), and the
generic `serif`, `sans-serif`, `monospace`, `cursive` and `system-ui`. Other names are removed.

| Name | Style |
| --- | --- |
| `Inter` | A clean sans-serif |
| `Fraunces` | An editorial serif |
| `Pixelify Sans` | A pixel font |
| `Caveat` | Handwriting |
| `JetBrains Mono` | Monospace |
| `Bebas Neue` | A tall display face, for headings |

### What reviewers look for

Your design is turned down (with what to change) if it imitates Slop's own interface (fake Hire,
Pay or Sign in buttons, ratings, reviews, badges or notices), links somewhere other than its text
says, has sexually explicit or otherwise not-allowed content, is hard to read (small text, low
contrast), flashes, hides Slop's parts you placed, or doesn't match your declared AI use. Taste
is yours.

### Example

`.slop/page/index.html`:

```html
<header class="intro">
  <h2>Hi, I'm Vera</h2>
  <p>Pixel art and tiny animations for cosy games.</p>
</header>
<div class="feature"><slop-work ref="1"></slop-work></div>
<h3>Everything</h3>
<slop-works></slop-works>
<p class="contact">Elsewhere: <a href="https://www.artstation.com/vera">ArtStation</a></p>
```

`.slop/page/style.css`:

```css
.page { background: linear-gradient(160deg, #241a3a, #120d1f); color: #f3ecff; padding: 32px; }
.intro h2 { font-family: "Pixelify Sans", monospace; font-size: 40px; margin: 0; color: #ffd166; }
.feature { max-width: 640px; margin: 24px auto; }
h3 { font-family: "Pixelify Sans", monospace; color: #ffd166; }
@media (max-width: 600px) { .page { padding: 16px; } .intro h2 { font-size: 28px; } }
```

The templates in `examples/portfolio-templates/` (Gallery, Showreel, Tracklist, Studio) each
have a complete `.slop/page/` to start from.
