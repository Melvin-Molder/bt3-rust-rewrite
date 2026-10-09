# BT3 Recomp

An original Rust implementation intended to read data from a user-provided copy of Dragon Ball Z: Budokai Tenkaichi 3. The repository contains no game assets or decompiled game code.

## Data-loading proof

The `bt3-data` crate reads the ISO9660 primary volume descriptor, root `SYSTEM.CNF`, and a bounded prefix of game file ID 2 from `DATA/PZS3US1.AFS`. Run it from the workspace root:

```powershell
cargo run -p bt3-data --bin bt3-inspect
```

By default it reads `rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso`; pass another ISO path as the first argument to override it. The ISO path reader resolves nested directories, and the AFS reader seeks directly to its header and entry table rather than loading the 1.46 GB archive. The file ID to partition mapping and ID 2's boot-time use are taken from the decomp documentation/source listed below. Data remains in process memory and is not extracted to disk.

## Sources and credit

- The crate-oriented separation of game/data concerns is informed by [gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) (MIT-licensed upstream project). No upstream source code or game data was copied.
- BT3 format research sources are listed in [bt3-format-research.md](bt3-format-research.md), including the BT3 community research archive, AFS tools, map MDL unpacker, animation tools, and PS2 executable decompilation.
- The global file-ID mapping and IDs 2-5's boot-time role are based on `C:\Users\User\.code\BT3-Decompiled\docs\systems\files_and_assets.md` and `src\sys\file.c` (CC0-1.0 for decomp project contributions). No decompiled code was copied.

The source and format references are credited for research only; their licenses remain with their respective projects. The game and its data remain the property of their rights holders.