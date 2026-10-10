# BT3 Recomp

An original Rust implementation intended to read data from a user-provided copy of Dragon Ball Z: Budokai Tenkaichi 3. The repository contains no game assets or decompiled game code.

## Data-loading proof

The `bt3-data` crate reads the ISO9660 primary volume descriptor and root `SYSTEM.CNF`, then locates and interprets real game resources in the AFS archives. It converts the three loading-screen resources (global file IDs `0x3C1`–`0x3C3`) to in-memory RGBA textures and reads stage gameplay metadata from stage file `0x171`. Run its inspector from the workspace root:

```powershell
cargo run -p bt3-data --bin bt3-inspect
```

By default it reads `rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso`; pass another ISO path as the first argument to override it. The ISO path reader resolves nested directories, and the AFS reader seeks directly to requested entries rather than loading the 1.46 GB archive. The global file-ID mapping, loading-screen behavior, stage layout, and coordinate transforms are grounded in the decomp sources listed below. Packed data, decoded resources, and RGBA textures stay in memory and are not extracted to disk.

Run the Bevy prototype with `cargo run -p bt3-game`; it accepts the ISO path as its first argument as well. By default it plays the boot pictures of `FirstRun_Main`: the six logos from menu archive `0x1DA`, at 60 Hz, with the original timing, fade colours and skip rules (Space / Enter or a gamepad's South / Start as pad 0 Cross / Start). The memory-card check before them is not ported yet. After them it plays the opening movie as `Movie_Run` does: `DATA/ZS3USOP.PSS` streamed from the ISO through `ffmpeg` (which must be on `PATH`) at one picture per two NTSC fields, with the `DATA/ZS3USOP.ADX` soundtrack decoded in Rust and played at the original's -12.9 dB; Start skips it with a half-second fade. Then it runs the title screen as `Title_Run(1)` does: the still picture for 300 frames, then the title movie (a FOD file, the game's converted Flash format) played by a port of the game's Flash player (`src/sys/flash.c`), with the scrolling clouds, the menu plates, the "Press START button" plate and the New Game / Continue menu (arrows or the d-pad move the cursor, with the original key repeat). The title music (`0x10B16`) loops at -12.9 dB, the announcer lines play at -1.6 dB, and both fade out by 12 dB a frame as the screen fades. `--title` starts there directly. Not ported yet: sound effects, the memory-card flow behind New Game, the main menu, and the attract battle; when the title ends with those, the screen stays black. After the third idle timeout the title replays the opening movie, as the original does. `--screen 0`, `1` or `2` runs a loading screen instead, driven by the decomp-ported `Load_*` logic at 30 Hz; Space stands in for Cross (`PADG_CROSS`, game bit `0x200`).

## Sources and credit

- The crate-oriented separation of game/data concerns is informed by [gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) (MIT-licensed upstream project). No upstream source code or game data was copied.
- BT3 format research sources are listed in [bt3-format-research.md](bt3-format-research.md), including the BT3 community research archive, AFS tools, map MDL unpacker, animation tools, and PS2 executable decompilation.
- The global file-ID mapping and BPE loading-screen behavior are based on `C:\Users\User\.code\BT3-Decompiled\src\sys\file.c`, `src\sys\loading.c`, `src\sys\bpe.c`, and `include\sys\loading.h` (CC0-1.0 for decomp project contributions). The Rust decoder is an independent implementation; no decompiled source was copied.
- Stage file IDs, relative-offset units, gameplay metadata, and coordinate conversion are based on the decomp stage loader, relocation, and stage-data readers. The Rust reader is an independent, bounds-checked implementation.

The source and format references are credited for research only; their licenses remain with their respective projects. The game and its data remain the property of their rights holders.