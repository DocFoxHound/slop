# Tracklist portfolio template

For composers and sound designers: tracks that play right on your page, each with a cover.

## Use it

1. **Make a new public repository** on GitHub, e.g. `portfolio`
   ([github.com/new](https://github.com/new)). It has to belong to the GitHub account linked to
   your Slop account (your page's editor on Slop shows which, or a **Link GitHub** button).
2. **Copy this folder's `.slop/` folder** into it, keeping the name (it starts with a dot).
   - With git: copy the folder into your clone, then commit and push.
   - In the browser: on your new repository choose **Add file → Create new file**, type
     `.slop/portfolio.json` as the name, paste this template's `portfolio.json` and commit. Then
     open the `.slop` folder, choose **Add file → Upload files**, and upload your own pictures.
     You don't need the placeholder pictures at all: just make the paths in `portfolio.json` match
     your files.
3. **Replace everything below**, then commit and push.
4. **On Slop**, open your page's editor (People → your page → Edit, or `/people/me`), go to
   **Portfolio**, paste your repository's address and choose **Check my portfolio now**. If
   anything needs fixing, Slop lists each problem with the work and field it's in. When it all
   passes, a person at Slop reviews it, and your page goes public once it's approved.

Paths in `portfolio.json` are relative to `.slop/`: `"works/painting.png"` is
`.slop/works/painting.png`. Use letters, numbers, `.`, `_`, `-` and `/` only (no spaces).

## Replace

- `.slop/avatar.png` (256×256 placeholder) and `.slop/banner.png` (1600×400 placeholder).
- `.slop/tracks/placeholder.wav` and `placeholder-2.wav`: one-second test tones, only there so the
  template passes Slop's check. Replace them with your tracks: MP3, Ogg, WAV, FLAC or M4A, up to
  15 MB each (an Ogg or MP3 excerpt of a minute or two is plenty), and update the paths.
- `.slop/covers/*.png`: a cover for each track (optional: remove `"poster"` and Slop shows a ♪).
- The `"youtube"` work: a sound design breakdown or a gameplay video with your audio. Replace the
  `REPLACE_ME4` placeholder with your video's link, or remove the work.
- In `.slop/portfolio.json`: every `title`, `caption` (the game, the mood, loops or not) and
  `skill` (`music` or `sound`).

## The rules in short

- 1 to 30 works; each has exactly one of `image`, `youtube` or `audio`.
- Pictures: PNG, JPEG, WebP or GIF, up to 8 MB and 8192 px a side. Avatar at least 128×128,
  banner at least 1200×300 (about 1600×400 is best; it's cropped to a strip), works and posters
  at least 32×32.
- Tracks: MP3, Ogg, WAV, FLAC or M4A, up to 15 MB.
- `poster` only on videos and tracks. Titles up to 80 characters, captions up to 200, one line
  each.
- Skills: `2d-art`, `pixel-art`, `3d`, `animation`, `ui-ux`, `music`, `sound`, `writing`,
  `voice`, `localization`, `playtesting`.

Everything else is in the [portfolio guide](https://slopai.games/portfolio.md).
