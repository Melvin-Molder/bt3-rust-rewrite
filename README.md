# BT3 Recomp

An original Rust implementation intended to read data from a user-provided copy of Dragon Ball Z: Budokai Tenkaichi 3. The repository contains no game assets or decompiled game code.

## Data-loading proof

The `bt3-data` crate reads the ISO9660 primary volume descriptor and root `SYSTEM.CNF`, then locates, BPE-decodes, and converts the three loading-screen resources (global file IDs `0x3C1`–`0x3C3`) from `DATA/PZS3US1.AFS` into in-memory RGBA textures. Run it from the workspace root:

```powershell
cargo run -p bt3-data --bin bt3-inspect
```

By default it reads `rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso`; pass another ISO path as the first argument to override it. The ISO path reader resolves nested directories, and the AFS reader seeks directly to requested entries rather than loading the 1.46 GB archive. The global file-ID mapping, loading-screen behavior, and asset layouts are grounded in the decomp sources listed below. Packed data, decoded resources, and RGBA textures stay in memory and are not extracted to disk.

Run the Bevy loading-screen prototype with `cargo run -p bt3-game`; it accepts the ISO path as its first argument as well. This prototype displays loading-screen type 0 at native 512x448 resolution and advances its original two-frame animation/counter. Space or Enter counts as the original action button for now.

## Sources and credit

- The crate-oriented separation of game/data concerns is informed by [gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) (MIT-licensed upstream project). No upstream source code or game data was copied.
- BT3 format research sources are listed in [bt3-format-research.md](bt3-format-research.md), including the BT3 community research archive, AFS tools, map MDL unpacker, animation tools, and PS2 executable decompilation.
- The global file-ID mapping and BPE loading-screen behavior are based on `C:\Users\User\.code\BT3-Decompiled\src\sys\file.c`, `src\sys\loading.c`, `src\sys\bpe.c`, and `include\sys\loading.h` (CC0-1.0 for decomp project contributions). The Rust decoder is an independent implementation; no decompiled source was copied.

The source and format references are credited for research only; their licenses remain with their respective projects. The game and its data remain the property of their rights holders.