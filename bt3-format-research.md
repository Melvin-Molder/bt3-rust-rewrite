# BT3 Asset Format Research

Research date: 2026-10-09
Scope: public community documentation and open-source repositories for Dragon Ball Z: Budokai Tenkaichi 3 (PS2/Wii), focused on maps, models, and animation data. This is a source review only; no binaries or tools were run.

## Findings

There is meaningful community tooling and research, so the game's executable is not the only available source of information and Ghidra is not the necessary next step.

- **Maps:** [tenkaichi-map-mdl-unpacker](https://github.com/ViveTheModder/tenkaichi-map-mdl-unpacker) targets the PS2 version of BT3 (and BT2). It exports one or all sections of `00_map_main_data.mdl` inside a map PAK, and says it also supports custom maps. This establishes a practical map container/member relationship, but is not a complete public binary specification of all stage data. The repository is archived and has downloadable releases.
- **Animations:** [tenkaichi-anm-swap](https://github.com/ViveTheModder/tenkaichi-anm-swap) works with BT2/BT3 on PS2 and Wii and swaps ranges of animations between two ANM PAK files. [BT3AnimationWorker](https://github.com/KkCap/BT3AnimationWorker) is a small BSD-3-Clause Python project describing animation speed changes, concatenation, and per-bone mixing; its README calls it work in progress/pre-beta. Together these show useful animation manipulation support, not a full standalone specification of the ANM encoding or rig semantics.
- **General format research:** [dbzbt3-research](https://github.com/ViveTheModder/dbzbt3-research) collects community research and links material for AFS, LPS lip-sync data, MDL (including skirt-model notes), and other parameter/file types. It is a useful index, but the README does not claim comprehensive map/model/animation format coverage.
- **Archives and naming:** [MatrixDJ96/DBZBT3](https://github.com/MatrixDJ96/DBZBT3) provides open-source AFS management and AFL list conversion tools. AFL is described as an AFS file list containing member names. [The mod installation guide](https://vivethemodder.github.io/guide_en.html) documents region-specific AFS names and importing AFL filename lists. This is relevant to locating assets in the disc's archives, not parsing every asset payload.
- **Character/model adjacent tooling:** [Budokai-Modding-Tool](https://github.com/SamuelDBZMAAM/Budokai-Modding-Tool) describes AMB/AMO0/AMG/AMT model/texture tooling, a model viewer, model-part editing, and import/export helpers for the Budokai series. Treat this as adjacent evidence rather than a verified BT3 character-model parser without checking its code and exact game/version support. A community model forum search result refers to BT3/BT2 content and PMDL, but the thread itself asks for remaining format/bone-export knowledge; that is not a complete spec.
- **Decompilation:** [z3xox/BT3-Decompiled](https://github.com/z3xox/BT3-Decompiled) is an active matching decompilation of the PS2 USA executable `SLUS-21678`; its README reports near-complete code matching. It reconstructs executable code, and its stated project purpose/layout is not an asset extractor or complete asset-format specification. The related PC port is separate.

## Practical takeaway

The existing public material is enough to begin with archive/member inventory and focused map or animation inspection without installing a disassembler. Map section extraction and animation swapping are concretely supported. Character model structures, skeletal/animation encoding details, and a unified end-to-end asset specification remain incomplete or need validation against the precise regional/platform build. The local workspace is PS2 USA-oriented according to its existing project notes; tools should be checked for region/version compatibility before use.

During the initial public-source research, no tools or dependencies were installed, no Ghidra analysis was performed, and no game executable or ISO data was read.

## Local ISO Read Check

The workspace's ignored PS2 USA ISO was read-only probed to verify the boot path for the first Rust loader. Its ISO9660 primary volume descriptor is at sector 16 and has the `CD001` signature. The volume ID is `SLUS_21678`; the root contains `SYSTEM.CNF`, whose contents name `cdrom0:\SLUS_216.78;1` as `BOOT2`, and the `SLUS_216.78` executable. No asset files or executable contents were copied into the repository.

## Decomp-Grounded File Loading

The local read-only reference at `C:\Users\User\.code\BT3-Decompiled` documents the file ID mapping in `docs/systems/files_and_assets.md` and implements it in `src/sys/file.c` (`File_OpenById`): ID 0 maps to partition 0 entry 0; IDs 1 through `0xD47` map to partition 1 entries `id - 1`; IDs from `0xD48` map to partition 2 entries `id - 0xD48`. The same system note identifies files 2, 3, 4, and 5 as common files loaded at boot.

The actual USA ISO's `DATA/PZS3US1.AFS` is 1,462,016,000 bytes. Its header declares 3,399 (`0xD47`) entries. Thus global ID 2 resolves to AFS1 entry 1; the observed little-endian table row gives offset `0x9800` and size `0x15B00`, which is in bounds. The first 16 bytes were read in memory for the loader proof, not written to the repository. The AFS table shape follows the community format writeup linked above; this implementation bounds-checks its archive and table reads and does not attempt to decode the entry payload.
