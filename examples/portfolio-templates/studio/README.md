# Studio portfolio template

For teams, and for anyone offering several kinds of work: a mix of pictures and video, each work
tagged with the skill it shows.

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

- `.slop/avatar.png` (256×256 placeholder): your studio's logo. `.slop/banner.png` (1600×400
  placeholder): your team's best piece, or your logo on a background.
- `.slop/works/*.png`: work that shows each service you offer. Rename them and update the paths.
- The `"youtube"` work: replace the `REPLACE_ME5` placeholder with your trailer or reel's link,
  and `works/trailer.png` with a still from it.
- In `.slop/portfolio.json`: every `title`, `caption` (for a team, say who did what) and `skill`.
  Your services and rates go on your page itself, in the editor's **Services**.

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
