# BT3 Recomp

An independent reimplementation of **Dragon Ball Z: Budokai Tenkaichi 3** for the PlayStation 2 (USA release,
SLUS-21678), written in Rust with [Bevy](https://bevyengine.org/). It runs on a PC and reads everything it needs
(models, stages, textures, sounds, scripts) from your own disc image as it runs.

This repository contains **no game data, no copy of the game and no decompiled code**. To run it you need your
own disc image of that release.

## Status

Every mode reachable from the main menu is rebuilt, from the boot screens to the ending:

| Area | State |
|---|---|
| Boot, logos, opening movie, title, main menu, options, saves | Done |
| Dragon History (story): sagas, episodes, scripted battles, results, ending | Done |
| Duel: 1P vs COM, 2P split screen, team and DP battles | Done |
| Ultimate Battle: Mission 100, Survival, Sim Dragon, Disc Fusion, Ultimate Battle Z | Done |
| World Tournament, Training, Evolution Z (customising, shop), Data Center, Shenron | Done |
| Battle engine: fighters, techniques, transformations, clashes, computer opponent, effects, cameras, HUD, replays | Done |
| Sound: the game's sound driver emulated (sequenced effects, music, voices, announcer) | Done |

The port follows the original's code function by function, and each Rust function names the original one it
rebuilds. A few visuals that depend on exactly how the PS2's graphics chip draws (a shadow's destination-alpha
test, the objects' distance fade) are left out rather than guessed. The opening and ending movies are decoded by
`ffmpeg`. [PROGRESS.md](PROGRESS.md) has the full list, and the details are in
[IMPLEMENTATION.md](IMPLEMENTATION.md).

## Supported disc

| Title | Region | Product code | Executable |
|---|---|---|---|
| Dragon Ball Z: Budokai Tenkaichi 3 | USA (English / Japanese) | SLUS-21678 | `SLUS_216.78` |

Only this release is supported: the port reads the game's files by their places on this disc. Other releases
(PAL, Japanese) are untested and not expected to work. At start the port checks the disc's executable and warns
when it is not `SLUS_216.78`.

## Running

You need:

- your disc image of the supported release (above), in a `rom/` folder at the top of the repository. Any file
  name works; to run an image from elsewhere, pass its path as the first argument;
- a [Rust](https://rustup.rs/) toolchain (edition 2024);
- `ffmpeg` on your `PATH`, for the movies.

```
cargo run --release -p bt3-game
```

Saves go to the `memcard/` folder. The game is drawn at the PS2's own 512 x 448 and only the finished frame is
scaled to the window, at 4:3 as a television shows it.

| Option | Effect |
|---|---|
| `--fullscreen` | Start in full screen (F11 switches at any time) |
| `--aspect tv \| pixel \| stretch` | 4:3 (the default), square pixels, or fill the window |
| `--filter linear \| nearest` | Smooth (the default) or sharp scaling |
| `--scale N` | The window's starting size, 1 to 8 (default 2) |
| `--title`, `--menu` | Skip to the title screen or the main menu |
| `--battle` | Go straight to a battle (`--stage N`, `--mode N`, `--split` for 2P) |
| `--memcard DIR` | Use another save folder |
| `--disc FILE` | A second disc for Disc Fusion's disc swap (F1 opens the tray, F2 changes the disc) |

### Controls

A gamepad works as the PS2 pad. The keyboard and mouse follow Dragon Ball: Sparking! ZERO's PC defaults: each key
does the same action here (it presses the PS2 button that does that action in this game).

| Battle | Key | PS2 |
|---|---|---|
| Move | W A S D | D-pad |
| Swing the camera to a side (hold) / pick the member a switch brings in | Arrow keys | Right stick |
| Melee (rush) | Left mouse button | Square |
| Ki blast | Right mouse button | Triangle |
| Guard | E | Circle |
| Dash / step | F | Cross |
| Ascend / descend | Space / Left Ctrl | R1 / R2 |
| Ki charge | Left Shift | L2 |
| Lock-on | Q | L1 |
| Transform (hold: fuse) | Z | R3 |
| Switch member | X | L3 + R3 |
| Skill 1 / 2 | 1 / 2 | L2 + Circle / L2 + Up + Circle |
| Blast 2 (first / second) | 3 / 4 | L2 + Triangle / L2 + Up + Triangle |
| Ultimate Blast | 5 | L2 + Down + Triangle |
| Pause | Tab or Enter | Start |
| Camera distance (its "switch camera") | C | Select |

Sparking! ZERO's PC key list has no lock-on, so Q is this port's choice. In the menus, including the battle's pause and result menus: W A S D or the arrows move, Space confirms, Escape or Backspace goes
back, Tab or Enter is Start, Q / E are L1 / R1, 1 / 3 are L2 / R2, and C is Circle.

## Building and testing

```
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

Many tests run against the real game: they read the disc image from `rom/` and are skipped when it is not
there. `cargo run -p bt3-data --bin bt3-inspect` prints what the reader finds on the disc.

## Layout

| Path | Contents |
|---|---|
| `crates/bt3_data` | Reading the disc: ISO9660, the AFS archives, compressed packs, and each resource format (models, motions, stages, textures, fonts, sound banks, Flash screens, ADX audio) |
| `crates/bt3_game` | The game: the battle engine, every menu and mode, rendering and sound, in Bevy |
| [PROGRESS.md](PROGRESS.md) | What is done, what is left, and the audits of the port against the original |
| [IMPLEMENTATION.md](IMPLEMENTATION.md) | How each part of the game is rebuilt, in detail |
| [bt3-format-research.md](bt3-format-research.md) | The research the file formats are based on |

## Related

- [z3xox/BT3-Decompiled](https://github.com/z3xox/BT3-Decompiled), a matching decompilation of the game (CC0
  1.0). The port was written against it as a reference: its names for the original functions are the ones the
  Rust code cites. No decompiled code was copied.
- [gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) (MIT) informed the split between the data
  and game crates, and [hacktools](https://github.com/Illidanz/hacktools) (MIT) and
  [PS2Tek](https://israpps.github.io/ps2tek/PS2/GS/Textures.html) the texture formats. No code was copied from
  either.
- The rest of the format research is credited in [bt3-format-research.md](bt3-format-research.md).

This project was made with the help of AI.

## Licence

The code in this repository is released under the [MIT Licence](LICENSE). That covers this project's own work only:
the game, its data and its trademarks belong to their rights holders.

## Legal

This project is not affiliated with or endorsed by the game's developers, publishers or rights holders. It
exists for preservation, study and interoperability. All rights to the game belong to their owners. You need
your own legally obtained copy of the game to use anything here.
