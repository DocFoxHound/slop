# Slop Pong

Pong in Godot 4: you against the computer, first to 7. It's also SLAPorSLOP's test game, using every
feature a game on SLAPorSLOP can: saves in SLAPorSLOP's save folder, mods from SLAPorSLOP (one with code), asking
the SLAPorSLOP app for a server's mods, and player counts. Copy what you need.

- `slop/slop.gd`: everything SLAPorSLOP, as one autoload named `Slop`. Drop it into your own game.
- `main.gd`: the game, and how it uses `Slop`.
- `../slop-pong-mods/`: two mods. **Neon** is data only (colours and a faster ball); **Rainbow
  ball** has code (`mod.gd`), so SLAPorSLOP's source check marks it "mod with code".
- `tests/`: headless tests, `godot --headless --path . res://tests/tests.tscn` (exits 0 when
  they pass).

Arrow keys or W/S move your paddle.

## How it uses SLAPorSLOP

| Feature | In the game |
|---|---|
| Saves (`SLOP_SAVE_DIR`) | Wins, losses and best rally in `pong.json`, written to a new file and renamed, so a crash never leaves half a save |
| Its own mods folder | `mods/` next to the game, each mod in its own folder; `SLOP_MODS` as well |
| Mods with code | A mod's `mod.json` can name a `script`; the game calls its `setup(game)` and `process(game, delta)` |
| Slop Link (`SLOP_LINK`) | **Play online (test server)** asks for the Neon mod, then plays with it |
| Slop Pulse | Start and heartbeats, from `slop/slop.gd` |

A mod's `mod.json`:

```json
{"name": "Neon", "colors": {"background": "#0b0221", "paddle": "#ff2bd6", "ball": "#39ff14"}, "ball_speed": 1.2, "script": "mod.gd"}
```

Every key but `name` is optional. Speeds are multiplied by 0.5 to 2; colours are `#rrggbb`.

## Publishing it on SLAPorSLOP

1. Put this folder in its own GitHub repository named `slop-pong` (the workflow in
   `.github/workflows/slop.yml` builds it with Slop Build), and each mod in its own repository.
2. On SLAPorSLOP, create the store page `slop-pong`: a game for Windows and Linux. Under **Mods**,
   choose **A folder in the game's files**, folder `mods`, **each mod in its own folder**. Tick
   **Works in sandbox mode**. Leave **Online** empty: the test server is pretend.
3. Push, and approve the first release.
4. On the game's page, **Publish a mod** twice: address `slop-pong-neon` for Neon (the test
   server asks for exactly this address) and `slop-pong-rainbow-ball` for Rainbow ball. Each
   release is a `.zip` of the mod's folder contents.

## Checklist on a real PC

Install the SLAPorSLOP app, sign in, then for Windows and for Linux:

1. Install Slop Pong from its page and press **Play**. Win or lose a match; quit; play again:
   the wins count stayed. The menu shows where the saves are: SLAPorSLOP's save folder for the game.
2. Turn **Sandbox mode** on for it. Play again: same saves; the game runs.
3. In the library, add **Rainbow ball**. Play: the ball changes colour. The game's page and your
   library say **CONTAINS CODE**.
4. Put any file in the game's `mods` folder by hand (open its install folder), then press Play:
   the SLAPorSLOP app says it moved it out, and that mods must be published on SLAPorSLOP.
5. Press **Play online (test server)**. The SLAPorSLOP app comes to the front and asks to add Neon:
   say **No** (the game says you can't join), then try again and say **Add 1 mod** (the game
   turns neon and starts). Do this in sandbox mode too.
6. With **Share my playtime** on, the game's page shows 1 playing now.
7. Uninstall it, keeping saves; reinstall: your wins are back. Uninstall with **Delete my
   saves too**: they're gone.

Report anything that went differently, with what the app or game said.
