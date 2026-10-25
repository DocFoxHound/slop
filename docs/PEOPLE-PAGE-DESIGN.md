<!--
  The "Your page's design" section of the portfolio guide (web/public/portfolio.md), which replaces
  its "## Your page's design (coming soon)" placeholder when the two branches meet. The rules are
  implemented in crates/slop-api/src/people/design.rs; keep the two in step.
-->

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
what and why) in your editor, above a preview of the result. Things that can't work (a picture
that isn't in the repository, a `<slop-work>` that isn't one of your works, a file over its
limit) are errors: nothing is sent for review until they're fixed.

**Where it shows.** Your page is, from the top: Slop's header (your name, avatar, AI-use badge,
rating, Hire and Report), then your design in its own box, then Slop's Services (unless your
design places them), Reviews and "Worked on". Your design can't change, cover or hide anything
outside its box, and the page's CSS doesn't touch your design. It must work at phone width
(about 360 pixels): use `max-width`, `flex-wrap`, grids with `minmax()` and `@media`. The
desktop app shows the same page.

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
| `<slop-services></slop-services>` | none | Your services, each with its Hire button |

`<slop-works>` and `<slop-services>` show once each. If your design doesn't place
`<slop-services>`, Slop's Services section shows below it. Slop's parts can't be inside a link.
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
  `@import`, `@font-face` or any other at-rule.
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
  `z-index` beyond ±100; `!important` (dropped, the rest kept); custom properties (`--x`) and
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
<h3>Hire me</h3>
<slop-services></slop-services>
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
