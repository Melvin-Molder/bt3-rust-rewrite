# Repository Guidance

## Project Structure

- This is a Rust 2024 Cargo workspace. `bt3-data` (`crates/bt3_data`) owns ISO/AFS parsing and game-resource decoding; `bt3-game` (`crates/bt3_game`) owns the Bevy runtime and rendering/gameplay.
- Keep format parsing and resource decoding in `bt3-data`; keep runtime state, input, and presentation in `bt3-game`.
- Preserve the independent-implementation approach. Do not copy decompiled game code or check in game assets. Research references and attribution belong in `bt3-format-research.md` and `README.md`.

## Changes

- Keep changes focused and follow nearby Rust and Bevy patterns. Prefer bounds-checked parsing for binary data and avoid large per-frame allocations in runtime paths.
- `.gitignore` is an allowlist for repository files. When adding a source file, add its path to `.gitignore` or it will remain ignored.
- Treat ROMs, extracted assets, and generated build output as local-only; do not add them to source control.
- Preserve behavior grounded in the game and format research. If a format or runtime detail is uncertain, verify it against the documented research or nearby tests before encoding an assumption.

## Build And Test

Run these commands from the workspace root:

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

Useful local runs:

```powershell
cargo run -p bt3-data --bin bt3-inspect -- <path-to-iso>
cargo run -p bt3-game -- --title
cargo run -p bt3-game -- --battle
```

The game reads a user-provided ISO; movie playback also requires `ffmpeg` on `PATH`. Avoid depending on a specific local ROM path in code or tests.