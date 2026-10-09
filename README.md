# BT3 Recomp

An original Rust implementation intended to read data from a user-provided copy of Dragon Ball Z: Budokai Tenkaichi 3. The repository contains no game assets or decompiled game code.

## First data-loading proof

The `bt3-data` crate reads an ISO9660 primary volume descriptor and a file in the disc root. Run it from the workspace root:

```powershell
cargo run -p bt3-data --bin bt3-inspect
```

By default it reads `rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso`; pass another ISO path as the first argument to override it. This first slice reads and prints the volume ID, block count, and `SYSTEM.CNF`. It does not extract or copy any disc contents.

## Sources and credit

- The crate-oriented separation of game/data concerns is informed by [gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) (MIT-licensed upstream project). No upstream source code or game data was copied.
- BT3 format research sources are listed in [bt3-format-research.md](bt3-format-research.md), including the BT3 community research archive, AFS tools, map MDL unpacker, animation tools, and PS2 executable decompilation.

The source and format references are credited for research only; their licenses remain with their respective projects. The game and its data remain the property of their rights holders.