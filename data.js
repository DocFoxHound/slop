// Fake catalog for the Slop mockup. Everything here is invented.

// Tags are grouped so the store can browse broad -> specific.
const TAG_GROUPS = [
  { name: "Games", icon: "🎮", desc: "Things you play", tags: ["Roguelike", "Puzzle", "Platformer", "Idle", "Simulation", "Visual Novel", "Strategy", "Racing", "Card Game", "Tycoon"] },
  { name: "Tools", icon: "🛠️", desc: "Things that get work done", tags: ["Productivity", "Developer Tools", "File Management", "Note-taking", "Automation", "Finance"] },
  { name: "Creative", icon: "🎨", desc: "Make art, music and words", tags: ["Music", "Pixel Art", "3D", "Writing", "Generative Art", "Video"] },
  { name: "Utilities", icon: "⚙️", desc: "Keep your machine happy", tags: ["System Monitor", "Launcher", "Backup", "Privacy"] },
  { name: "Learning", icon: "📚", desc: "Get smarter, slowly", tags: ["Language", "Math", "Typing", "Kids"] },
  { name: "Play style", icon: "🕹️", desc: "How you play", tags: ["Single-player", "Co-op", "Multiplayer", "Controller Support", "Touch Controls", "Offline", "Short Sessions"] },
  { name: "Vibe", icon: "✨", desc: "How it feels", tags: ["Cozy", "Retro", "Dark", "Minimalist", "Chaotic", "Relaxing", "Funny"] },
];

const PLATFORMS = {
  windows: { label: "Windows", group: "desktop" },
  mac: { label: "macOS", group: "desktop" },
  linux: { label: "Linux", group: "desktop" },
  ios: { label: "iOS", group: "phone" },
  android: { label: "Android", group: "phone" },
};

// Every build is one of these. A publisher needs at least one to publish.
const BUILD_KINDS = {
  installer: "Installer",
  executable: "Executable",
  runner: "Portable runner",
  apk: "APK",
  ipa: "iOS app",
};
// Which build kinds make sense per platform.
const KINDS_FOR = { windows: ["installer", "executable", "runner"], mac: ["installer", "executable", "runner"], linux: ["installer", "executable", "runner"], ios: ["ipa"], android: ["apk"] };

const SCAN_CHECKS = [
  { id: "malware", name: "Malware scan", detail: "Multi-engine scan of every binary, installer and archive" },
  { id: "behavior", name: "Behavior sandbox", detail: "Runs each build in an isolated VM and watches file, network and process activity" },
  { id: "reproducible", name: "Build matches source", detail: "Checks the publisher's signature and that each build came from the tagged commit" },
  { id: "deps", name: "Dependency audit", detail: "Checks every dependency against known vulnerabilities and hijacked packages" },
  { id: "secrets", name: "Secrets & telemetry", detail: "Looks for leaked keys, hidden trackers and undisclosed network endpoints" },
  { id: "license", name: "License compliance", detail: "Matches code and assets against known projects to catch license violations" },
  { id: "content", name: "Illegal content screening", detail: "Screens text, images and audio for illegal or prohibited content" },
  { id: "monetization", name: "Monetization rules", detail: "Detects in-app purchases, loot boxes, pay-to-win, donor perks, fake currencies and scam patterns" },
  { id: "pulse", name: "Slop Pulse", detail: "Games only: confirms the player-count ping is wired up and sends nothing but a signed heartbeat", games: true },
];

// Release stages. Days are the limits a publisher has before they must move on.
const STAGE_RULES = { soonDays: 30, betaDays: 365, deadAfterDays: 90 };
const DAY = 864e5;
const daysAgo = n => new Date(Date.now() - n * DAY).toISOString().slice(0, 10);

// Shorthand for a clean scan with optional notes per check.
function scan(date, notes = {}) {
  return { date, results: SCAN_CHECKS.map(c => ({ id: c.id, status: notes[c.id]?.status || "pass", note: notes[c.id]?.note || "" })) };
}

const USERS = ["pixelpriest", "tuxfan", "mossbyte", "kettle_of_fish", "quietmode", "ramsticks", "yuki_dev", "honkhonk", "bramble", "nullpointer_nan", "sockmonkey", "lumen", "fernando_codes", "glitchwitch"];

const APPS = [
  {
    id: "pixel-dungeon-delver", name: "Pixel Dungeon Delver", dev: "nightcoder", type: "Games",
    art: { emoji: "🗝️", from: "#3b1d60", to: "#0f766e" },
    short: "A tight, turn-based roguelike where every floor is hand-tuned by the AI that wrote it - and then fixed by humans.",
    long: "Descend through 40 floors of procedurally generated dungeon. Each run mixes 200+ items, 60 enemies and a crafting system that actually makes sense.\n\nThe original game was generated over a weekend; the community has since rewritten the combat balance, added a hardcore mode and translated it into six languages.",
    tags: ["Roguelike", "Single-player", "Retro", "Controller Support", "Offline", "Dark"],
    slop: 12, reviews: 4812, license: "MIT", ai: ["Claude", "Stable Diffusion"], version: "1.6.2", updated: "2026-10-09",
    donation: 5, raised: 18240, donors: 2961, iap: "none",
    builds: { windows: { kind: "installer", size: "84 MB" }, mac: { kind: "installer", size: "91 MB" }, linux: { kind: "runner", size: "79 MB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "nightcoder", desc: "Official release branch.", stars: 3410, updated: "2026-10-09", builds: {} },
      { name: "hardcore-mode", author: "bramble", desc: "Permadeath across saves, no healing fountains, doubled enemy damage.", stars: 822, updated: "2026-10-02", builds: { windows: { kind: "executable", size: "84 MB" }, linux: { kind: "runner", size: "79 MB" } } },
      { name: "touch-port", author: "pocketdev", desc: "Android port with touch controls and a portrait UI. Saves sync with desktop via a file.", stars: 540, updated: "2026-10-03", builds: { android: { kind: "apk", size: "62 MB" } },
        readme: "# Pixel Dungeon Delver - touch port\n\nPlays on Android phones and tablets.\n\n- Tap to move, long-press to inspect\n- Portrait and landscape layouts\n- Export/import your save file to carry runs between desktop and phone\n\n**Not maintained by the publisher.**" },
      { name: "i18n-ja-ru-pt", author: "yuki_dev", desc: "Japanese, Russian and Portuguese translations with font fixes.", stars: 301, updated: "2026-09-21", builds: { windows: { kind: "installer", size: "88 MB" } } },
    ],
    queue: { version: "1.6.3", commits: ["Fix shopkeeper softlock on floor 17", "Rebalance poison daggers", "Add colourblind palette"], scan: 6 },
    scan: scan("2026-10-09"),
    reviewList: [
      { user: "glitchwitch", slop: 8, hours: 112, helpful: 640, date: "2026-10-01", text: "Started as AI slop, became a genuinely great roguelike thanks to the branch system. The hardcore-mode branch is the way to play." },
      { user: "ramsticks", slop: 15, hours: 34, helpful: 210, date: "2026-09-18", text: "Pixel art is a bit samey in the late floors, but the item synergies are clever. Low slop, would delve again." },
      { user: "honkhonk", slop: 30, hours: 6, helpful: 44, date: "2026-08-30", text: "Fun but the tutorial text reads like a chatbot apologising to me. Someone please branch the tutorial." },
    ],
  },
  {
    id: "tidy-files", name: "TidyFiles", dev: "orderly_otter", type: "Tools",
    art: { emoji: "🗂️", from: "#0c4a6e", to: "#38bdf8" },
    short: "Point it at your Downloads folder and it sorts 10 years of chaos into sensible folders. Undo for everything.",
    long: "TidyFiles groups files by type, project and date, finds duplicates and suggests names for files called 'final_FINAL_v2'. Every action is logged and reversible.\n\nThe publisher only ships a Windows build - Linux and macOS builds come from community branches.",
    tags: ["File Management", "Productivity", "Minimalist", "Offline"],
    slop: 22, reviews: 1290, license: "Apache-2.0", ai: ["GPT", "Cursor"], version: "2.1.0", updated: "2026-10-04",
    donation: 3, raised: 4420, donors: 1203, iap: "none",
    builds: { windows: { kind: "installer", size: "22 MB" } },
    tracked: "stable",
    branches: [
      { name: "stable", author: "orderly_otter", desc: "Official release branch.", stars: 1104, updated: "2026-10-04", builds: {} },
      { name: "linux-port", author: "tuxfan", desc: "Native Linux build using the same core, with XDG folder support and an AppImage runner.", stars: 486, updated: "2026-10-06", builds: { linux: { kind: "runner", size: "25 MB" } },
        readme: "# TidyFiles - Linux port\n\nThis branch builds TidyFiles natively on Linux. It is **not** maintained by the original publisher.\n\n## What's different\n\n- Uses XDG user folders (`~/Downloads`, `~/Documents`) instead of Windows known folders\n- Ships as a single AppImage - no install needed\n- Trash integration uses the freedesktop spec, so undo works from your file manager too\n\n## Known issues\n\n- The duplicate finder is slower on Btrfs with compression enabled\n- No system tray icon on GNOME without the AppIndicator extension\n\n## Staying in sync\n\nI rebase on the publisher's `stable` branch within a few days of every release." },
      { name: "macos-build", author: "applecrumble", desc: "Universal macOS build (Apple Silicon + Intel). Signed with a community certificate.", stars: 212, updated: "2026-09-28", builds: { mac: { kind: "installer", size: "31 MB" } },
        readme: "# TidyFiles for macOS\n\nCommunity build of TidyFiles for macOS 13 and newer.\n\n## Notes\n\n- First launch: right-click > Open, because this is signed with a community certificate rather than the publisher's\n- Spotlight metadata is preserved when files are moved\n\n## Known issues\n\n- iCloud Drive folders are skipped for now" },
      { name: "ai-rename-local", author: "mossbyte", desc: "Experimental: suggests file names with a small on-device model. No cloud calls.", stars: 97, updated: "2026-09-12", builds: {} },
    ],
    queue: { version: "2.1.1", commits: ["Faster duplicate hashing", "Respect .tidyignore files"], scan: 8 },
    scan: scan("2026-10-04"),
    reviewList: [
      { user: "kettle_of_fish", slop: 14, hours: 9, helpful: 302, date: "2026-09-30", text: "Sorted 40,000 files without losing a single one. The undo log saved me twice. Using the linux-port branch and it's flawless." },
      { user: "quietmode", slop: 35, hours: 2, helpful: 41, date: "2026-09-02", text: "Works, but the 'smart' folder names are occasionally hilarious. 'Misc Stuff (Probably Taxes)'." },
    ],
  },
  {
    id: "lofi-loom", name: "Lo-Fi Loom", dev: "beatsmith", type: "Creative",
    art: { emoji: "🎧", from: "#7c2d12", to: "#f472b6" },
    short: "Weave endless lo-fi loops from a few knobs. Export stems, loop forever, study harder.",
    long: "Pick a mood, a key and a tempo; Lo-Fi Loom generates layered loops you can tweak track by track. Everything renders locally, and all samples in the bundled library are CC0.",
    tags: ["Music", "Relaxing", "Cozy", "Offline"],
    slop: 31, reviews: 870, license: "GPL-3.0", ai: ["Claude", "MusicGen"], version: "0.9.4", updated: "2026-09-25",
    donation: 4, raised: 2310, donors: 640, iap: "none",
    builds: { windows: { kind: "installer", size: "140 MB" }, mac: { kind: "installer", size: "152 MB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "beatsmith", desc: "Official release branch.", stars: 640, updated: "2026-09-25", builds: {} },
      { name: "flatpak", author: "tuxfan", desc: "Flatpak packaging for Linux with PipeWire audio.", stars: 133, updated: "2026-09-27", builds: { linux: { kind: "installer", size: "148 MB" } },
        readme: "# Lo-Fi Loom (Flatpak)\n\nCommunity Flatpak for Lo-Fi Loom.\n\n- Uses PipeWire for audio; PulseAudio works through the compatibility layer\n- Sandboxed: only has access to `~/Music` by default\n\n**Not affiliated with the publisher.**" },
    ],
    queue: null,
    scan: scan("2026-09-25", { license: { status: "pass", note: "All 412 bundled samples verified CC0" } }),
    reviewList: [
      { user: "lumen", slop: 25, hours: 51, helpful: 120, date: "2026-09-20", text: "My study soundtrack now. Loops get a bit repetitive after an hour but the stem export is great." },
      { user: "sockmonkey", slop: 44, hours: 3, helpful: 18, date: "2026-08-11", text: "Every track sounds like the same rainy café. Which is the point, I guess." },
    ],
  },
  {
    id: "starfreight-idle", name: "Starfreight Idle", dev: "orbital_ops", type: "Games",
    art: { emoji: "🚀", from: "#1e1b4b", to: "#6366f1" },
    short: "Run a shipping empire across a galaxy that keeps growing while you sleep.",
    long: "Buy freighters, set trade routes, automate everything. Prestige to unlock new star systems.",
    tags: ["Idle", "Strategy", "Short Sessions", "Single-player"],
    slop: 58, reviews: 2210, license: "MIT", ai: ["GPT"], version: "3.0.1", updated: "2026-10-11",
    donation: 2, raised: 1880, donors: 910, iap: "none",
    builds: { windows: { kind: "executable", size: "48 MB" }, linux: { kind: "executable", size: "51 MB" } },
    tracked: "release",
    branches: [
      { name: "release", author: "orbital_ops", desc: "Official release branch.", stars: 420, updated: "2026-10-11", builds: {} },
      { name: "no-grind-rebalance", author: "nullpointer_nan", desc: "Cuts the mid-game grind roughly in half.", stars: 388, updated: "2026-10-01", builds: { windows: { kind: "executable", size: "48 MB" } } },
    ],
    queue: null,
    scan: scan("2026-10-11", { monetization: { status: "warn", note: "Previously flagged 'premium credits' removed in 3.0.0 - verified" } }),
    reviewList: [
      { user: "nullpointer_nan", slop: 62, hours: 140, helpful: 260, date: "2026-10-03", text: "The numbers go up. That's... it. Play the no-grind-rebalance branch or don't bother." },
      { user: "bramble", slop: 50, hours: 22, helpful: 61, date: "2026-09-14", text: "Surprisingly relaxing, wildly unbalanced. Classic mid-slop." },
    ],
  },
  {
    id: "snapnote", name: "Snapnote", dev: "linenfold", type: "Tools",
    art: { emoji: "📝", from: "#365314", to: "#a3e635" },
    short: "Notes that link themselves. Plain markdown files on your disk, no account, no cloud.",
    long: "Snapnote watches a folder of markdown files and builds a live graph of how your notes connect. The publisher develops on Linux only; a community branch provides the Windows build.",
    tags: ["Note-taking", "Productivity", "Minimalist", "Offline", "Writing"],
    slop: 18, reviews: 640, license: "MIT", ai: ["Claude"], version: "1.2.0", updated: "2026-09-30",
    donation: 5, raised: 3105, donors: 520, iap: "none",
    builds: { linux: { kind: "installer", size: "38 MB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "linenfold", desc: "Official release branch.", stars: 760, updated: "2026-09-30", builds: {} },
      { name: "android", author: "mossbyte", desc: "Android app that reads the same markdown folder through your sync tool of choice.", stars: 201, updated: "2026-10-01", builds: { android: { kind: "apk", size: "22 MB" } } },
      { name: "windows-build", author: "fernando_codes", desc: "Windows installer with Explorer context-menu integration.", stars: 154, updated: "2026-10-02", builds: { windows: { kind: "installer", size: "41 MB" } },
        readme: "# Snapnote for Windows\n\nCommunity Windows build. Adds **Open folder in Snapnote** to the Explorer right-click menu.\n\n## Known issues\n\n- OneDrive placeholders are indexed only after they are downloaded" },
    ],
    queue: { version: "1.3.0", commits: ["Daily notes template", "Graph view zoom presets"], scan: 3 },
    scan: scan("2026-09-30"),
    reviewList: [
      { user: "mossbyte", slop: 12, hours: 80, helpful: 190, date: "2026-09-25", text: "The graph view is gorgeous and it never locks my files into a format. Lowest slop note app I've used." },
    ],
  },
  {
    id: "neon-drift-2077", name: "Neon Drift 2077", dev: "hypersynth", type: "Games",
    art: { emoji: "🏎️", from: "#831843", to: "#22d3ee" },
    short: "Synthwave street racing with 3 tracks, 1 car and an unreasonable amount of bloom.",
    long: "Drift through a neon city. The physics were generated, the soundtrack was generated, and it shows. Still: the lights are very pretty.",
    tags: ["Racing", "Retro", "Chaotic", "Controller Support", "Single-player"],
    slop: 74, reviews: 3320, license: "MIT", ai: ["GPT", "Midjourney", "Suno"], version: "1.0.3", updated: "2026-08-14",
    donation: 1, raised: 610, donors: 404, iap: "none",
    builds: { windows: { kind: "installer", size: "1.2 GB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "hypersynth", desc: "Official release branch.", stars: 150, updated: "2026-08-14", builds: {} },
      { name: "physics-fix", author: "ramsticks", desc: "Rewrites the car physics so the car stops occasionally flying into space.", stars: 902, updated: "2026-10-08", builds: { windows: { kind: "installer", size: "1.2 GB" }, linux: { kind: "runner", size: "1.1 GB" } },
        readme: "# Neon Drift 2077 - physics fix\n\nReplaces the generated physics with a simple, tuned arcade model.\n\n- Car no longer achieves orbit at 140 mph\n- Proper drift angle and counter-steer\n- Adds a Linux runner\n\nHonestly this branch is the game now." },
    ],
    queue: null,
    scan: scan("2026-08-14", { license: { status: "warn", note: "Soundtrack generated with Suno - publisher attests commercial-use rights" } }),
    reviewList: [
      { user: "honkhonk", slop: 88, hours: 1, helpful: 1200, date: "2026-08-20", text: "My car achieved escape velocity on the first corner. 10/10 for space program, 88 slop for racing." },
      { user: "ramsticks", slop: 55, hours: 30, helpful: 400, date: "2026-10-08", text: "Wrote the physics-fix branch. With it, it's a decent little racer. Without it, pure chaos." },
    ],
  },
  {
    id: "cat-cafe-tycoon", name: "Cat Café Tycoon", dev: "whiskerworks", type: "Games",
    art: { emoji: "🐈", from: "#9a3412", to: "#fde68a" },
    short: "Run a tiny café full of cats who absolutely do not care about your business plan.",
    long: "Design your café, adopt cats with distinct personalities and keep customers happy.",
    tags: ["Tycoon", "Simulation", "Cozy", "Relaxing", "Single-player", "Kids"],
    slop: 27, reviews: 5120, license: "MPL-2.0", ai: ["Claude", "Stable Diffusion"], version: "2.4.0", updated: "2026-10-12",
    donation: 5, raised: 22950, donors: 4108, iap: "none",
    builds: { windows: { kind: "installer", size: "310 MB" }, mac: { kind: "installer", size: "322 MB" }, linux: { kind: "installer", size: "305 MB" }, ios: { kind: "ipa", size: "180 MB" }, android: { kind: "apk", size: "164 MB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "whiskerworks", desc: "Official release branch.", stars: 2900, updated: "2026-10-12", builds: {} },
      { name: "more-cats", author: "glitchwitch", desc: "Adds 30 community-drawn cats. All art CC-BY with credits in-game.", stars: 1530, updated: "2026-10-05", builds: { windows: { kind: "installer", size: "340 MB" }, mac: { kind: "installer", size: "352 MB" }, linux: { kind: "installer", size: "336 MB" } } },
    ],
    queue: { version: "2.5.0", commits: ["Rainy-day weather", "Cat nap animations", "Fix: cats no longer stand inside tables"], scan: 2 },
    scan: scan("2026-10-12", { monetization: { status: "pass", note: "Nothing for sale, no randomized rewards, no donor perks" } }),
    reviewList: [
      { user: "lumen", slop: 20, hours: 210, helpful: 820, date: "2026-10-12", text: "The cat AI is genuinely charming. Bought the hat pack because I wanted to, not because the game nagged me. That's how it should be." },
      { user: "kettle_of_fish", slop: 32, hours: 40, helpful: 90, date: "2026-09-01", text: "A few of the generated cats have six toes on one paw and none on the other. Install more-cats for the hand-drawn ones." },
    ],
  },
  {
    id: "spell-bee-arena", name: "Spell Bee Arena", dev: "hivemind_edu", type: "Learning",
    art: { emoji: "🐝", from: "#713f12", to: "#facc15" },
    short: "Head-to-head spelling battles for ages 7 to 107. Offline, no ads, no chat.",
    long: "Kids (and adults) battle through spelling rounds that adapt to their level. No chat, no accounts, no ads - parents can verify all of that in the source.",
    tags: ["Language", "Kids", "Multiplayer", "Funny", "Offline"],
    slop: 35, reviews: 410, license: "MIT", ai: ["Claude"], version: "1.1.0", updated: "2026-09-08",
    donation: 3, raised: 980, donors: 302, iap: "none",
    builds: { windows: { kind: "installer", size: "60 MB" }, mac: { kind: "installer", size: "64 MB" }, ios: { kind: "ipa", size: "48 MB" }, android: { kind: "apk", size: "41 MB" } },
    tracked: "main",
    branches: [{ name: "main", author: "hivemind_edu", desc: "Official release branch.", stars: 210, updated: "2026-09-08", builds: {} }],
    queue: null,
    scan: scan("2026-09-08", { secrets: { status: "pass", note: "No network access at all - verified in sandbox" } }),
    reviewList: [{ user: "quietmode", slop: 35, hours: 12, helpful: 66, date: "2026-09-10", text: "My kids love it. A couple of the 'hard' words are made up, which is a very AI thing to do." }],
  },
  {
    id: "fractal-forge", name: "Fractal Forge", dev: "mandelbro", type: "Creative",
    art: { emoji: "🌀", from: "#164e63", to: "#a855f7" },
    short: "Real-time 3D fractal explorer with 8K renders and animation export.",
    long: "Fly through Mandelbulbs and Julia sets on the GPU, keyframe a camera path and export video. Shader code is fully commented, which is rare for anything, let alone AI output.",
    tags: ["Generative Art", "3D", "Video", "Relaxing"],
    slop: 15, reviews: 1980, license: "BSD-3-Clause", ai: ["Claude", "Copilot"], version: "4.0.0", updated: "2026-10-10",
    donation: 8, raised: 15400, donors: 1630, iap: "none",
    builds: { windows: { kind: "installer", size: "95 MB" }, mac: { kind: "installer", size: "99 MB" }, linux: { kind: "runner", size: "92 MB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "mandelbro", desc: "Official release branch.", stars: 2400, updated: "2026-10-10", builds: {} },
      { name: "vr-mode", author: "lumen", desc: "OpenXR support - fly through fractals in VR.", stars: 640, updated: "2026-09-29", builds: { windows: { kind: "executable", size: "102 MB" } } },
    ],
    queue: null,
    scan: scan("2026-10-10"),
    reviewList: [{ user: "pixelpriest", slop: 10, hours: 66, helpful: 310, date: "2026-10-10", text: "Hypnotic. The renders are print quality and it runs well on a mid-range GPU." }],
  },
  {
    id: "taskgoblin", name: "TaskGoblin", dev: "goblin_hq", type: "Tools",
    art: { emoji: "👺", from: "#14532d", to: "#84cc16" },
    short: "A to-do list with a goblin that steals your tasks if you procrastinate too long.",
    long: "Finish tasks, earn gold. Leave them too long and the goblin takes them to his hoard. Weirdly effective.",
    tags: ["Productivity", "Funny", "Chaotic", "Short Sessions"],
    slop: 41, reviews: 760, license: "MIT", ai: ["GPT", "Cursor"], version: "0.7.0", updated: "2026-09-19",
    donation: 2, raised: 720, donors: 280, iap: "none",
    builds: { mac: { kind: "installer", size: "28 MB" }, ios: { kind: "ipa", size: "19 MB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "goblin_hq", desc: "Official release branch.", stars: 300, updated: "2026-09-19", builds: {} },
      { name: "electron-everywhere", author: "sockmonkey", desc: "Cross-platform build for Windows and Linux.", stars: 88, updated: "2026-09-22", builds: { windows: { kind: "executable", size: "92 MB" }, linux: { kind: "runner", size: "95 MB" } },
        readme: "# TaskGoblin everywhere\n\nWraps TaskGoblin so it runs on Windows and Linux.\n\n- Bigger download than the native macOS build\n- Notifications use the OS native system" },
    ],
    queue: null,
    scan: scan("2026-09-19"),
    reviewList: [{ user: "bramble", slop: 41, hours: 15, helpful: 52, date: "2026-09-21", text: "The goblin is motivating. The UI is pure AI-default purple gradients though." }],
  },
  {
    id: "ultimate-clicker-ultimate", name: "Ultimate Clicker Ultimate", dev: "clickfarm9000", type: "Games",
    art: { emoji: "🖱️", from: "#3f3f46", to: "#a16207" },
    short: "Click the button. The button gets bigger. You click the bigger button.",
    long: "Generated in one prompt, apparently. Published because it passes the rules - it's open source, it builds, it passed every scan. Whether it's good is what the slop score is for.",
    tags: ["Idle", "Chaotic", "Funny", "Short Sessions"],
    slop: 92, reviews: 6800, license: "Unlicense", ai: ["GPT"], version: "1.0.0", updated: "2026-07-01",
    donation: 1, raised: 41, donors: 38, iap: "none",
    builds: { windows: { kind: "executable", size: "3 MB" }, mac: { kind: "executable", size: "3 MB" }, linux: { kind: "executable", size: "3 MB" } },
    tracked: "main",
    branches: [
      { name: "main", author: "clickfarm9000", desc: "Official release branch.", stars: 12, updated: "2026-07-01", builds: {} },
      { name: "actually-a-game", author: "honkhonk", desc: "Adds upgrades, a story, and a reason to click.", stars: 1440, updated: "2026-10-01", builds: { windows: { kind: "executable", size: "6 MB" }, mac: { kind: "executable", size: "6 MB" }, linux: { kind: "executable", size: "6 MB" } } },
    ],
    queue: null,
    scan: scan("2026-07-01"),
    reviewList: [
      { user: "honkhonk", slop: 97, hours: 0.2, helpful: 2400, date: "2026-07-02", text: "Pure, uncut slop. I made a branch out of spite and it's now more popular than the original." },
      { user: "yuki_dev", slop: 85, hours: 0.5, helpful: 300, date: "2026-07-10", text: "The button is very nice. That's all there is." },
    ],
  },
  {
    id: "stepquest", name: "StepQuest", dev: "trailmix", type: "Games",
    art: { emoji: "👟", from: "#065f46", to: "#f59e0b" },
    short: "A phone-only RPG where your real-world steps are your experience points. Walk the dog, level up.",
    long: "StepQuest turns your daily steps into dungeon progress. It reads your phone's step counter on-device; nothing about where you walk ever leaves the phone.\n\nPhone only - there's no desktop build. On a computer, Slop can send it straight to your linked phone.",
    tags: ["Simulation", "Touch Controls", "Short Sessions", "Funny", "Single-player"],
    slop: 24, reviews: 1980, license: "MIT", ai: ["Claude", "Stable Diffusion"], version: "1.3.0", updated: "2026-10-07",
    donation: 2, raised: 5120, donors: 2210, iap: "none",
    builds: { ios: { kind: "ipa", size: "72 MB" }, android: { kind: "apk", size: "64 MB" } },
    tracked: "main",
    branches: [{ name: "main", author: "trailmix", desc: "Official release branch.", stars: 760, updated: "2026-10-07", builds: {} }],
    queue: null,
    scan: scan("2026-10-07", { secrets: { status: "pass", note: "Step data stays on-device - verified in sandbox" } }),
    reviewList: [{ user: "bramble", slop: 20, hours: 60, helpful: 240, date: "2026-10-08", text: "Got me walking 3km a day. The dungeon art is generic but the loop is genuinely motivating." }],
  },
  {
    id: "bytebench", name: "ByteBench", dev: "kernel_kat", type: "Utilities",
    art: { emoji: "📈", from: "#0f172a", to: "#10b981" },
    short: "A tiny system monitor that shows what's eating your CPU, RAM and battery - in plain English.",
    long: "ByteBench explains what each process is and whether it's normal, so you don't have to search the internet for 'svchost why'. Uses under 20 MB of RAM.",
    tags: ["System Monitor", "Privacy", "Minimalist", "Offline"],
    slop: 9, reviews: 2750, license: "MIT", ai: ["Claude"], version: "5.2.1", updated: "2026-10-13",
    donation: 3, raised: 9800, donors: 2580, iap: "none",
    builds: { windows: { kind: "installer", size: "9 MB" }, mac: { kind: "installer", size: "11 MB" }, linux: { kind: "runner", size: "8 MB" }, android: { kind: "apk", size: "6 MB" } },
    tracked: "main",
    branches: [{ name: "main", author: "kernel_kat", desc: "Official release branch.", stars: 4100, updated: "2026-10-13", builds: {} }],
    queue: { version: "5.2.2", commits: ["Explain GPU processes on Apple Silicon"], scan: 8 },
    scan: scan("2026-10-13"),
    reviewList: [{ user: "nullpointer_nan", slop: 6, hours: 300, helpful: 980, date: "2026-10-01", text: "Best utility on the store. Small, fast, honest. This is what low slop looks like." }],
  },
  {
    id: "hollow-pines", name: "The Hollow Pines", dev: "inkwell_tales", type: "Games",
    art: { emoji: "🌲", from: "#052e16", to: "#57534e" },
    short: "A quiet mystery visual novel about a town where the radio only plays tomorrow's news.",
    long: "Six chapters, three endings, one very strange radio. Written with an AI co-writer and then edited heavily by hand.",
    tags: ["Visual Novel", "Dark", "Writing", "Single-player"],
    slop: 46, reviews: 520, license: "CC-BY-SA-4.0 / MIT", ai: ["Claude", "Stable Diffusion"], version: "1.0.0", updated: "2026-09-02",
    donation: 4, raised: 1350, donors: 390, iap: "none",
    builds: { windows: { kind: "installer", size: "410 MB" }, mac: { kind: "installer", size: "420 MB" }, linux: { kind: "installer", size: "405 MB" } },
    tracked: "main",
    branches: [{ name: "main", author: "inkwell_tales", desc: "Official release branch.", stars: 180, updated: "2026-09-02", builds: {} }],
    queue: null,
    scan: scan("2026-09-02"),
    reviewList: [{ user: "kettle_of_fish", slop: 46, hours: 7, helpful: 70, date: "2026-09-05", text: "Great premise, uneven middle. Chapter 4 feels like it was written by a different model." }],
  },
];

// Submissions the current user has made - shows what the scanners catch.
const MY_SUBMISSIONS = [
  { name: "Tiny Tavern", status: "live", note: "Coming soon page live · all checks passed", appId: "tiny-tavern", project: true },
  { name: "Cat Café Tycoon", status: "live", note: "v2.4.0 passed all 9 checks · live since Oct 12", appId: "cat-cafe-tycoon", project: true },
  { name: "Lootbox Legends", status: "rejected", check: "monetization", note: "Randomized paid rewards detected in src/shop/crate.ts:88 - 'mystery crate' sells random items for real money. Loot boxes are not allowed on Slop." },
  { name: "Sprite Studio Pro", status: "held", check: "license", note: "14 files match 'pixelkit' (GPL-3.0) but this repo is licensed MIT. Relicense to GPL-3.0 or remove the files, then resubmit." },
  { name: "Speedy PC Cleaner", status: "blocked", check: "malware", note: "Uploaded installer does not match source build, and it contacts an undisclosed server on first run. Submission blocked and account flagged for review." },
];

const REPORT_REASONS = [
  { id: "malware", label: "Malware or suspicious behavior", hint: "Crashes your system, phones home, installs extra software" },
  { id: "license", label: "Stolen code or license violation", hint: "Copies someone else's code or art without following its license" },
  { id: "illegal", label: "Illegal content", hint: "Content that breaks the law" },
  { id: "scam", label: "Scam or deceptive monetization", hint: "Fake giveaways, misleading prices, phishing" },
  { id: "p2w", label: "Pay-to-win or loot boxes", hint: "Paying gives a gameplay advantage, or paid random rewards" },
  { id: "notopen", label: "Source doesn't match the build", hint: "The GitHub code isn't what actually runs" },
  { id: "misleading", label: "Misleading store page", hint: "Screenshots or descriptions don't match the app" },
  { id: "broken", label: "Broken build", hint: "Doesn't install or doesn't launch" },
  { id: "other", label: "Something else", hint: "" },
];

// Monthly library adds, players right now, and this month's peak (games only).
const METRICS = {
  "pixel-dungeon-delver": [5400, 1204, 3980], "tidy-files": [2100], "lofi-loom": [1300], "starfreight-idle": [3900, 2210, 4100],
  "snapnote": [900], "neon-drift-2077": [2600, 88, 1500], "cat-cafe-tycoon": [9800, 3120, 7400], "spell-bee-arena": [600, 140, 400],
  "fractal-forge": [3100], "taskgoblin": [1100], "ultimate-clicker-ultimate": [7300, 12, 2900], "bytebench": [4200], "hollow-pines": [800, 95, 610], "stepquest": [6100, 2840, 5300],
};
APPS.forEach(a => {
  const [adds, playing, peak] = METRICS[a.id] || [0];
  Object.assign(a, { stage: "released", monthAdds: adds, playing, peak, lastCommit: a.updated });
});

// Upcoming: "Coming soon" needs source + scans only; "Beta" needs at least one build.
const mainBranch = (author, updated, desc = "Official development branch.") => ({ name: "main", author, desc, stars: 0, updated, builds: {} });
const UPCOMING = [
  {
    id: "moonlit-orchard", name: "Moonlit Orchard", dev: "fernleaf", type: "Games", stage: "soon", stageSince: daysAgo(17), lastCommit: daysAgo(1), interested: 8420,
    art: { emoji: "🌙", from: "#1e1b4b", to: "#065f46" },
    short: "A cozy farming sim that only happens at night. Grow glowing fruit, befriend moths, avoid the sun.",
    long: "Moonlit Orchard flips the farming sim: everything happens between dusk and dawn. The source is public and already passes every scan - builds arrive when it enters beta.",
    tags: ["Simulation", "Cozy", "Relaxing", "Single-player", "Controller Support"],
    devlog: ["Night-time harvesting", "Fireflies now follow your lantern", "First pass on the moth friendship system"],
  },
  {
    id: "deckbound", name: "Deckbound", dev: "cardsharp", type: "Games", stage: "beta", stageSince: daysAgo(120), lastCommit: daysAgo(3), interested: 12800, playing: 640, peak: 1900,
    art: { emoji: "🃏", from: "#7f1d1d", to: "#d97706" },
    short: "A roguelike deckbuilder where your deck is also your map. Beta builds out now.",
    long: "Every card you play reveals a room. Build a deck that's strong in combat and lays out a good dungeon. In open beta on Windows and Linux.",
    tags: ["Card Game", "Roguelike", "Strategy", "Single-player", "Short Sessions"],
    builds: { windows: { kind: "installer", size: "210 MB" }, linux: { kind: "runner", size: "198 MB" }, android: { kind: "apk", size: "120 MB" } },
    devlog: ["New act 3 boss", "Deck-to-map generator 2.0", "Steam Deck controls"],
  },
  {
    id: "tiny-tavern", name: "Tiny Tavern", dev: "whiskerworks", type: "Games", stage: "soon", stageSince: daysAgo(2), lastCommit: daysAgo(2), interested: 3900,
    art: { emoji: "🍺", from: "#78350f", to: "#fbbf24" },
    short: "From the makers of Cat Café Tycoon: run a fantasy tavern for adventurers with very specific dietary needs.",
    long: "Cook for orcs, elves and one extremely picky dragon. Same team, same rules: no loot boxes, nothing for sale.",
    tags: ["Tycoon", "Simulation", "Cozy", "Funny", "Single-player"],
    devlog: ["Recipe system prototype", "The dragon now complains about everything"],
  },
  {
    id: "promptpaint", name: "PromptPaint", dev: "pixelpriest", type: "Creative", stage: "soon", stageSince: daysAgo(6), lastCommit: daysAgo(0), interested: 2100,
    art: { emoji: "🖌️", from: "#4c1d95", to: "#ec4899" },
    short: "A pixel-art editor with an assistant that suggests palettes and cleans up your linework - never draws for you.",
    long: "Hand-drawn pixel art with a helpful sidekick. Everything runs locally.",
    tags: ["Pixel Art", "Generative Art", "Minimalist", "Offline"],
    devlog: ["Palette suggestions", "Onion-skin animation preview"],
  },
  {
    id: "orbit-notes", name: "Orbit Notes", dev: "linenfold", type: "Tools", stage: "beta", stageSince: daysAgo(200), lastCommit: daysAgo(9), interested: 3400,
    art: { emoji: "🪐", from: "#0c4a6e", to: "#a3e635" },
    short: "Snapnote's big sibling: shared notebooks that sync peer-to-peer, no server in the middle.",
    long: "Real-time shared notes without an account or a cloud. Beta builds for macOS and Linux.",
    tags: ["Note-taking", "Productivity", "Writing", "Privacy", "Offline"],
    builds: { mac: { kind: "installer", size: "52 MB" }, linux: { kind: "runner", size: "48 MB" } },
    devlog: ["Conflict-free merging", "LAN discovery"],
  },
  {
    id: "lingo-lantern", name: "Lingo Lantern", dev: "hivemind_edu", type: "Learning", stage: "soon", stageSince: daysAgo(25), lastCommit: daysAgo(4), interested: 940,
    art: { emoji: "🏮", from: "#7c2d12", to: "#facc15" },
    short: "Learn a language by translating a lantern-lit story, one glowing word at a time.",
    long: "A story-driven language app for kids and adults. Japanese and Spanish first.",
    tags: ["Language", "Kids", "Cozy", "Offline"],
    devlog: ["Spanish chapter 1", "Pronunciation hints"],
  },
  {
    id: "hexfall-tactics", name: "Hexfall Tactics", dev: "gridlord", type: "Games", stage: "beta", stageSince: daysAgo(250), lastCommit: daysAgo(131), interested: 5100, playing: 0, peak: 3,
    art: { emoji: "⬡", from: "#27272a", to: "#475569" },
    short: "Hex-grid tactics with online battles. Promising beta - but nothing has been pushed in months.",
    long: "A turn-based tactics game with ranked online play. The beta was popular, then the commits stopped.",
    tags: ["Strategy", "Retro", "Multiplayer"],
    builds: { windows: { kind: "installer", size: "130 MB" } },
    devlog: ["Ranked matchmaking (last update)"],
  },
];
UPCOMING.forEach(u => {
  APPS.push({
    license: "MIT", ai: ["Claude"], version: u.stage === "beta" ? "0.9.0-beta" : "0.1.0-dev", updated: u.lastCommit,
    donation: 3, raised: Math.round(u.interested * 0.4), donors: Math.round(u.interested / 12), iap: "none",
    builds: {}, tracked: "main", queue: null, slop: null, reviews: 0, reviewList: [], monthAdds: 0,
    ...u,
    branches: [mainBranch(u.dev, u.lastCommit)],
    scan: scan(u.lastCommit, u.stage === "soon" && u.type === "Games" ? { pulse: { status: "pass", note: "Pulse SDK found in source - live handshake will be tested when the first build is uploaded" } } : {}),
  });
});

// ---- Social: friends, teams, Private Slop ----------------------------------
const FRIENDS = ["mossbyte", "bramble", "lumen", "glitchwitch"];

// Official teams are verified developer studios. Player teams are anyone.
const TEAMS = [
  { id: "whiskerworks", name: "Whiskerworks", kind: "official", icon: "🐾", members: 4, open: false, apps: ["cat-cafe-tycoon", "tiny-tavern"],
    desc: "The small studio behind Cat Café Tycoon and Tiny Tavern. Cosy games, no loot boxes, ever." },
  { id: "nightcoder-games", name: "Nightcoder Games", kind: "official", icon: "🗝️", members: 2, open: false, apps: ["pixel-dungeon-delver"],
    desc: "Two people, one roguelike, a lot of community branches." },
  { id: "linenfold-labs", name: "Linenfold Labs", kind: "official", icon: "🧵", members: 3, open: false, apps: ["snapnote", "orbit-notes"],
    desc: "Local-first tools for writing and thinking." },
  { id: "cozy-crew", name: "Cozy Crew", kind: "player", icon: "🍵", members: 1240, open: true, tags: ["Cozy", "Relaxing", "Simulation"],
    desc: "Players who like slow, comfy games. Cozy game night every Friday.", picks: ["cat-cafe-tycoon", "lofi-loom", "moonlit-orchard"] },
  { id: "roguelike-runners", name: "Roguelike Runners", kind: "player", icon: "💀", members: 860, open: true, tags: ["Roguelike", "Card Game"],
    desc: "Daily seeds, weekly races, endless arguments about item tiers.", picks: ["pixel-dungeon-delver", "deckbound"] },
  { id: "weekend-jam-club", name: "Weekend Jam Club", kind: "player", icon: "🛠️", members: 12, open: false, tags: ["Short Sessions"],
    desc: "We make tiny games every weekend and share them with each other as Private Slop before anything goes public.", picks: [] },
  { id: "pure-slop-society", name: "Pure Slop Appreciation Society", kind: "player", icon: "🫠", members: 3300, open: true, tags: ["Chaotic", "Funny"],
    desc: "We play the sloppiest things on the store so you don't have to. Then we write branches to fix them.", picks: ["ultimate-clicker-ultimate", "neon-drift-2077"] },
];

// Private Slop: same requirements as public apps (minus Slop Pulse), visible only to the owner
// unless shared with friends or player teams. Never with official teams.
const PRIVATE_SEED = [
  { id: "recipe-box", name: "Grandma's Recipe Box", owner: "you", type: "Tools", art: { emoji: "📖", from: "#7c2d12", to: "#fcd34d" },
    short: "Every recipe from Grandma's card box, scanned and searchable. Just for the family.",
    repo: "github.com/you/recipe-box", builds: { windows: { kind: "installer", size: "18 MB" }, android: { kind: "apk", size: "12 MB" } },
    shared: { friends: ["bramble"], teams: [] }, scanned: "2026-10-10" },
  { id: "jam-dungeon", name: "Jam Dungeon (prototype)", owner: "you", type: "Games", art: { emoji: "🧪", from: "#312e81", to: "#22d3ee" },
    short: "This weekend's jam game. Three rooms, one boss, zero polish.",
    repo: "github.com/you/jam-dungeon", builds: { linux: { kind: "runner", size: "40 MB" }, windows: { kind: "executable", size: "42 MB" } },
    shared: { friends: [], teams: ["weekend-jam-club"] }, scanned: "2026-10-13" },
  { id: "moss-timer", name: "Moss's Focus Timer", owner: "mossbyte", type: "Tools", art: { emoji: "🌿", from: "#14532d", to: "#86efac" },
    short: "A tiny focus timer mossbyte made for themselves - shared with friends.",
    repo: "github.com/mossbyte/moss-timer", builds: { windows: { kind: "executable", size: "5 MB" }, mac: { kind: "executable", size: "6 MB" }, linux: { kind: "runner", size: "5 MB" } },
    shared: { friends: ["you"], teams: [] }, scanned: "2026-10-02" },
  { id: "jam-racer", name: "Jam Racer", owner: "glitchwitch", type: "Games", art: { emoji: "🏁", from: "#701a75", to: "#f472b6" },
    short: "glitchwitch's entry for last weekend's jam.",
    repo: "github.com/glitchwitch/jam-racer", builds: { windows: { kind: "executable", size: "55 MB" } },
    shared: { friends: [], teams: ["weekend-jam-club"] }, scanned: "2026-10-06" },
];

// ---- Humans: creative people and studios developers can hire --------------
const HUMAN_SKILLS = ["2D Art", "Pixel Art", "3D Modelling", "Animation", "Music", "Sound Design", "Writing", "UI / UX", "Voice Acting", "Localization", "Playtesting"];
const AI_USE = {
  none: ["No AI", "Everything is made by hand"],
  assisted: ["AI-assisted", "Uses AI for references or drafts - disclosed per job"],
  heavy: ["Mostly AI-generated", "Output is largely generated, then edited"],
};
const HUMANS = [
  { id: "inkwell-and-ivy", name: "Inkwell & Ivy", kind: "studio", team: 3, avatar: "🎨", art: ["#7c2d12", "#f472b6"], location: "Lisbon, PT",
    skills: ["2D Art", "Animation", "UI / UX"], tagline: "Hand-painted characters, portraits and UI kits for cosy games.",
    rate: "from $60 / character", turnaround: "1-2 weeks", available: true, ai: "none", slop: 8, clients: 42,
    portfolio: ["🧙", "🐉", "🏰", "🍄", "🦊", "🌸"], worked: ["cat-cafe-tycoon"],
    services: [["Character portrait", "$60"], ["Character sheet (3 poses)", "$180"], ["UI kit - buttons, panels, icons", "$450"]],
    reviews: [{ user: "whiskerworks", slop: 6, project: "Cat Café Tycoon - 24 cat portraits", text: "Every cat has personality. Delivered early, revisions were painless. Zero slop." },
              { user: "nightcoder", slop: 12, project: "Pixel Dungeon Delver - shop UI", text: "Lovely work, one round of revisions to match our pixel style." }] },
  { id: "voxelvera", name: "Vera Okafor", kind: "individual", avatar: "🧊", art: ["#1e3a8a", "#22d3ee"], location: "Lagos, NG",
    skills: ["3D Modelling", "Pixel Art"], tagline: "Low-poly and voxel models, game-ready, rigged on request.",
    rate: "$35 / hour", turnaround: "3-10 days", available: true, ai: "assisted", slop: 14, clients: 27,
    portfolio: ["🚀", "🏎️", "🌲", "🗿", "🛖", "🐙"], worked: ["neon-drift-2077"],
    services: [["Low-poly prop", "$40"], ["Rigged character", "$220"], ["Environment kit (20 pieces)", "$600"]],
    reviews: [{ user: "ramsticks", slop: 10, project: "Neon Drift 2077 (physics-fix) - car models", text: "Replaced the generated cars with proper low-poly ones. Clean topology, great communication." }] },
  { id: "loopsmith", name: "Loopsmith Audio", kind: "studio", team: 2, avatar: "🎛️", art: ["#3b0764", "#a3e635"], location: "Montréal, CA",
    skills: ["Music", "Sound Design"], tagline: "Adaptive soundtracks and punchy SFX. We make loops that don't get old.",
    rate: "from $150 / track", turnaround: "1-3 weeks", available: true, ai: "none", slop: 11, clients: 61,
    portfolio: ["🎹", "🥁", "🎻", "🔊", "🎺", "🎧"], worked: ["pixel-dungeon-delver"],
    services: [["Loopable track (2-3 min)", "$150"], ["SFX pack (40 sounds)", "$300"], ["Adaptive score (5 layers)", "$900"]],
    reviews: [{ user: "nightcoder", slop: 9, project: "Pixel Dungeon Delver - floor themes", text: "Replaced our generated music. Players noticed immediately - slop score dropped 8 points." }] },
  { id: "pixelpriest", name: "pixelpriest", kind: "individual", avatar: "👾", art: ["#4c1d95", "#ec4899"], location: "Online",
    skills: ["Pixel Art", "Animation"], tagline: "Sprite sheets, tilesets and tiny animated things. Also building PromptPaint.",
    rate: "from $25 / sprite sheet", turnaround: "2-5 days", available: true, ai: "none", slop: 22, clients: 33,
    portfolio: ["🗡️", "🧝", "🪙", "🕯️", "🐸", "🌋"], worked: [],
    services: [["Character sprite sheet", "$25"], ["16x16 tileset (64 tiles)", "$120"]],
    reviews: [{ user: "bramble", slop: 22, project: "hardcore-mode - new enemy sprites", text: "Great sprites, a bit slow to reply on weekends." }] },
  { id: "sam-quill", name: "Sam Quill", kind: "individual", avatar: "🪶", art: ["#14532d", "#fde68a"], location: "Edinburgh, UK",
    skills: ["Writing", "Localization"], tagline: "Narrative design, dialogue and item descriptions that don't read like a chatbot.",
    rate: "$0.08 / word", turnaround: "1 week", available: true, ai: "assisted", slop: 19, clients: 18,
    portfolio: ["📜", "🗺️", "💬", "📖", "🎭", "✒️"], worked: ["hollow-pines"],
    services: [["Dialogue pass", "$0.08 / word"], ["Item & lore descriptions", "$0.10 / word"], ["Narrative outline", "$300"]],
    reviews: [{ user: "inkwell_tales", slop: 18, project: "The Hollow Pines - chapter 4 rewrite", text: "Fixed our weakest chapter. Honest about where they used AI for first drafts." }] },
  { id: "vox-populi", name: "Vox Populi Voices", kind: "studio", team: 9, avatar: "🎙️", art: ["#7f1d1d", "#fb923c"], location: "Berlin, DE",
    skills: ["Voice Acting", "Localization"], tagline: "A roster of 9 voice actors in 6 languages. Real people, real performances.",
    rate: "$120 / 100 lines", turnaround: "2 weeks", available: false, ai: "none", slop: 16, clients: 40,
    portfolio: ["🗣️", "🎭", "🌍", "🎬", "📻", "🎤"], worked: [],
    services: [["Character voice (100 lines)", "$120"], ["Narrator, full game", "quote"]],
    reviews: [{ user: "cardsharp", slop: 15, project: "Deckbound - boss barks", text: "Booked up fast for a reason. Great range." }] },
  { id: "bugbusters", name: "BugBusters QA", kind: "studio", team: 14, avatar: "🐞", art: ["#365314", "#facc15"], location: "Remote",
    skills: ["Playtesting"], tagline: "Human playtesters who find the bugs your AI didn't know it wrote.",
    rate: "$25 / tester-hour", turnaround: "48h reports", available: true, ai: "none", slop: 28, clients: 55,
    portfolio: ["🐛", "📋", "🎮", "🧪", "📊", "🔍"], worked: ["deckbound"],
    services: [["Smoke test + report", "$150"], ["Week-long playtest (5 testers)", "$1,000"]],
    reviews: [{ user: "cardsharp", slop: 25, project: "Deckbound beta", text: "Found 140 bugs. Reports could be tidier but the coverage was excellent." }] },
  { id: "promptmonger", name: "PromptMonger", kind: "individual", avatar: "🤖", art: ["#3f3f46", "#a16207"], location: "Online",
    skills: ["2D Art", "3D Modelling", "Music", "Writing"], tagline: "Any asset, any style, delivered in an hour!",
    rate: "$5 / asset", turnaround: "1 hour", available: true, ai: "heavy", slop: 84, clients: 12,
    portfolio: ["🖼️", "🖼️", "🖼️", "🖼️", "🖼️", "🖼️"], worked: [],
    services: [["Any asset", "$5"]],
    reviews: [{ user: "honkhonk", slop: 90, project: "Ultimate Clicker Ultimate - button art", text: "It was straight generator output with the watermark cropped off. At least they disclosed it." }] },
];
