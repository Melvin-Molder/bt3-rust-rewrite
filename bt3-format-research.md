# BT3 Asset Format Research

Research date: 2026-10-09
Scope: public community documentation and open-source repositories for Dragon Ball Z: Budokai Tenkaichi 3 (PS2/Wii), focused on maps, models, and animation data. This is a source review only; no binaries or tools were run.

## Findings

There is meaningful community tooling and research, so the game's executable is not the only available source of information and Ghidra is not the necessary next step.


## Practical takeaway

The existing public material is enough to begin with archive/member inventory and focused map or animation inspection without installing a disassembler. Map section extraction and animation swapping are concretely supported. Character model structures, skeletal/animation encoding details, and a unified end-to-end asset specification remain incomplete or need validation against the precise regional/platform build. The local workspace is PS2 USA-oriented according to its existing project notes; tools should be checked for region/version compatibility before use.

During the initial public-source research, no tools or dependencies were installed, no Ghidra analysis was performed, and no game executable or ISO data was read.

## Local ISO Read Check

The workspace's ignored PS2 USA ISO was read-only probed to verify the boot path for the first Rust loader. Its ISO9660 primary volume descriptor is at sector 16 and has the `CD001` signature. The volume ID is `SLUS_21678`; the root contains `SYSTEM.CNF`, whose contents name `cdrom0:\SLUS_216.78;1` as `BOOT2`, and the `SLUS_216.78` executable. No asset files or executable contents were copied into the repository.

## Decomp-Grounded File Loading

The BT3 decompilation project (a read-only reference) documents the file ID mapping in `docs/systems/files_and_assets.md` and implements it in `src/sys/file.c` (`File_OpenById`): ID 0 maps to partition 0 entry 0; IDs 1 through `0xD47` map to partition 1 entries `id - 1`; IDs from `0xD48` map to partition 2 entries `id - 0xD48`. `Common_LoadBoot` reads file 5 and sizes files 2-4; `Common_Reload` later requests files 2-4.

The actual USA ISO's `DATA/PZS3US1.AFS` is 1,462,016,000 bytes. Its header declares 3,399 (`0xD47`) entries. Thus global ID 2 resolves to AFS1 entry 1; the observed little-endian table row gives offset `0x9800` and size `0x15B00`, which is in bounds. The first 16 bytes were read in memory for the loader proof, not written to the repository. The AFS table shape follows the community format writeup linked above; this implementation bounds-checks its archive and table reads and does not attempt to decode the entry payload.

The decomp's `Load_InitScreen` loads file ID `0x3C1 + type` and passes it to `Sprite_Unpack`, which calls `Bpe_Decode`. For type 0, real file ID `0x3C1` maps to AFS1 entry `0x3C0`: its AFS size is `0x1EA0`, while the BPE header declares raw size `0x4320` and packed size `0x1E89`. The declared packed payload fits inside the entry, with 15 trailing bytes beyond the BPE payload. The Rust loader now decodes this resource in memory and checks the decoded size; the independent BPE implementation is covered by literal/pair fixtures and the actual disc resource.

The same runtime path also decodes loading-screen types 1 and 2 (IDs `0x3C2` and `0x3C3`) from the actual ISO. Their AFS member sizes are `0x2B00` and `0x2460`; their BPE headers declare output sizes 25,760 bytes each and packed sizes 10,999 and 9,296 bytes, respectively. All three leave only short trailing regions outside the BPE payload and decode to their declared output lengths. The Rust texture-resource parser follows `Res_RelocateOffsets`' verified layout: entry-table offsets and pixel/CLUT offsets are word offsets from the resource base, and each entry is 0x40 bytes.

The real loading textures use TEX0 PSMT4 (`PSM=0x14`), dimensions 128x128 and `TBW=2`. Their GIF IMAGE packets upload one PSMCT32 page (64x32) for pixels and an 8x2 PSMCT32 transfer for the 16-color CSM1 CLUT. Stored TEX0 lacks TCC, but `Sprite_DrawList` sets bit 34 at draw time (`4 << 32`), enabling RGBA: CLUT alpha must be retained, scaled from GS 0..128 to host 0..255. The background's GS vertex alpha `0x80` is full opacity, so its Bevy color is opaque.

Palette colors in the GIF IMAGE payload are in transfer order, not GS physical-memory order. The PSMCT32 upload places those 16 colors at physical word offsets `0,1,4,5,8,9,12,13,2,3,6,7,10,11,14,15`. CSM1 lookup uses those same offsets. Composing upload and lookup therefore preserves the original transfer index. Applying the lookup permutation directly to the IMAGE bytes selects the wrong RGB and alpha together, causing the beige sprite and incorrect transparency. The converter now simulates both steps; tests cover all 16 indices and a selected color with zero alpha.

The addressing references are the BT3 loading/draw path, [PS2Tek GS textures](https://israpps.github.io/ps2tek/PS2/GS/Textures.html), and the MIT-licensed [hacktools PS2 utilities](https://github.com/Illidanz/hacktools); PCSX2's GPL-3.0+ GS tables were consulted as a format cross-check. No third-party or decompiled source code was copied.

## Stage 0 Data Read

The Rust inspector reads stage 0 from global file ID `0x171` (partition 1, entry `0x170`) in the installed USA ISO. The member is 6,231,936 bytes. The decomp's battle loader requests `0x171 + stage`; `StgModel_InitStage` takes the stage block at file-header word 1. The Rust parser reads stage bounds from the stage header, environment and placement arrays from the stage-data block, and applies the decomp's Y/Z sign changes to positions and water level.

The first real member reports stage ID 0, 11 destructible-object definitions, 100 zones, bounds radius 1200 and vertical limits -350 to 1000, and two starting placements. Stage-header relocation pointers are word-relative (`s32 * base + offset` in `BtlStage_Relocate`), while pointers inside the stage-data block are byte-relative (`u8 * base + offset` in `BtlStage_BindData`); the Rust parser keeps those units distinct and bounds-checks each range. Alternate starting placements are parsed separately when present. The file is read at runtime from the ISO and is not extracted.

## Disc Identification

`Disc_Identify` (`src/sys/file.c`) looks for three boot files in the root of the disc in the drive, in the order of the executable's `gDiscExeNameTbl` (data at `0x2C70D8`, strings at `0x2F34D0..0x2F34FF`): `\SLUS_212.27;1` (Budokai Tenkaichi, index 0), `\SLUS_214.41;1` (Budokai Tenkaichi 2, index 1) and `\SLUS_216.78;1` (this game, index 2). Disc Fusion (mode 24) asks for index 0 or 1 and then for index 2 back; the port's drive answers from the root directory of the disc image it holds.
