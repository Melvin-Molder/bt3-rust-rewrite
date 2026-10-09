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

No tools or dependencies were installed, no Ghidra analysis was performed, and no game executable or ISO data was read during this research.
