# Progress

What of the game is rewritten, measured two ways: by feature (what a player reaches) and by the decompiled functions the Rust code ports. Update both when a piece lands.

## By feature

The game's screens hang off `Progress_Main`'s mode dispatcher (src/menu/progress.c); each group below is one of its handlers.

### Boot and front end

- [x] Memory-card check at boot (`BootCard_Run`)
- [x] Logos (`Logo_Show`, `FirstRun_Main`)
- [x] Opening movie (`Movie_PlayOpening`, mode 2)
- [x] Title screen and attract battle (`Title_Run`, mode 1)
- [x] Main menu (`MainMenu_Run`, mode 4)
- [x] Saves: load, save, sound options taking effect
- [x] Options screen (`OptMode_Main`, mode 62; src/menu/option.c)
- [x] Character reference (`CharRefMode_Main`, mode 60; src/menu/src/ui/char_viewer.c)

### Story: Dragon History (`Hist_Main`, modes 3, 6..10)

- [x] Saga select with the guide script (mode 6)
- [x] Episode list (mode 7)
- [x] Story battles: the GSC script engine, setup, scenes, triggers, result
- [x] Result screen and rewards (mode 8)
- [x] Saga outro (mode 9)
- [x] Save screen (mode 10)
- [x] Ending movie (mode 3)
- [x] Dragon balls hidden on the stages (`BattleResult.dragonBallFound`)
- [x] HUD slide out / in around a scripted scene, and the event cue (`HudPrompt_ShowCue`)

### Duel (`Duel_Main`, modes 38..41)

- [x] Duel menu, character select, team select
- [x] Versus battle: 1P vs COM, 2P split screen, pause and result menus, rematch

### Ultimate Battle (`Ub_Main`, modes 13..30)

- [x] Ultimate Battle menu (`UbMenu_Run`, mode 13)
- [x] Mission 100: select, team / solo select, result (modes 14..16)
- [x] Survival: course select, fighter select, battle, result (modes 17..19)
- [x] Sim Dragon: top, day, events, result (modes 20..23; src/menu/sim_*.c)
- [x] Disc Fusion (mode 24), with the drive's tray and discs on the host (F1 / F2, `--disc`)
- [x] Ultimate Battle Z ranking and course select (modes 25..30; src/menu/ub_*.c, ubz_select.c)

### World Tournament (`Tour_Main`, modes 33..35)

- [x] Tournament menu with the clock and the invitations (`TourMenu_Run`, mode 33; src/menu/tour_menu.c)
- [x] Entrant select (`EntrySel_Run`, mode 34; src/menu/tour_background.c)
- [x] Bracket and its battles, prizes (`Bracket_Run`, mode 35; src/menu/bracket*.c)

### Training (`TrainMode_Main`, modes 44..45)

- [x] Training menu (`Train_Run`, mode 44; src/menu/training.c): classes, lessons, pages, the lesson battles and their "cleared" bits
- [x] Free training's character select (`CharSel_Run(2)`, mode 45) and battle
- [x] The tutorials' goals (the fighters' frame bits, `BtlCtrl_TestTutorialFrameBit` → `HudNotice_Show(8)`)

### Evolution Z (`EvoMode_Main`, modes 48..50)

- [x] Top menu with the guide's explanation and the save on leaving (`EvoTop_Run`, mode 48; src/menu/evo_top.c, evo_mode.c)
- [x] Character customising (`EvoZ_Run`, mode 49; src/menu/evo_z_*.c)
- [x] Item shop (`Shop_Run`, mode 50; src/menu/shop.c)

### Data Center (`Dc_Main`, modes 53..56)

- [x] Data center menu and item lists
- [x] Passwords
- [x] Replay list and replay playback

### Shenron (mode 70)

- [x] The wish screen with its 3D summoning, the confirmation and reward windows, and the save after it (modes 70, 71; src/ui/shen_*.c)

### Battle engine

- [x] Loading screen and battle loading, character and stage changes
- [x] Fighters: the action state machine, movement, combat, techniques, transformations, fusions, switches, clashes A and B
- [x] Clash C: started by the story scripts' forced dashes (actions 5 / 6)
- [x] Computer opponent (`BtlAi*`)
- [x] Effects, stages and their effects, cameras, the demo camera
- [x] HUD, pause and result menus, replays
- [x] Sound: the fighters' sounds and voices, music, announcer
- [x] The objects' frame-alpha passes of `BtlObj_BeginDraw` (`GfxAlphaKey`, `ObjOutline`, `ObjGlow`: each material's alpha id in the frame, the glow of `BtlObj_SetAlphaAdd`; src/sys/gfx_post.c, gfx_screen.c)

### Battle details left (from the port's "not ported" notes)

Visuals whose accuracy depends on how the PS2 renders (the shadows' destination-alpha test, a fading debris piece's
depth, blends in the GS's colour space) are left out.

- [x] Hit sounds and voices (`btl_hit.rs`), the clash voices
- [x] The techniques' fire sounds and the rush sparks' sounds
- [x] Streamed sounds: volume, pan and player per owner; voice lines of kinds 5..7 (`btl_snd.rs`). The ADX players' pan: `adxt_SetOutPan` gives every channel the player's pan (no file on the disc has `AINF` pans, the game never asks for the automatic ones, mono centres it), CRI_ADXI.IRX puts each channel's voice to either side by its 31-step cosine law (180 / 255 a side at the centre); the balance is never set
- [x] `BtlAct_QueueComboFinish` and the attack records 0x77 / 0x87 / 0x8D..0x8F (`BtlDecide_QueueAttack`)
- [x] A paid evasion's attack (flag 0x7C), guard kind 4 from a guard action
- [x] Throws: the motions played from the opponent's table (table flag 0x20000000), a clash win's throw, the throw break (`BtlHit_CheckThrowBreak`)
- [x] The battle events fired by hits, clashes and techniques (all of `BtlEvent_Raise`'s ids, the KO's 0x51..0x53 by the damage's kind)
- [x] The technique camera cuts
- [x] Technique effects: the rush swarm's models; the disc's model (`EftDisc_Draw`, `EftDisc_StepTex`), the multi's piece scale, roll and abort flag
- [x] The ki objects' models, fragments and explosions (`EftKiObj_Draw`, `EftKiObj_Break`, `EftKiBomb_UpdateModel`; a full object pool kills the object)
- [x] The stage timer of stages 4 / 27 (read only by `BtlCharApi_GetMgrTimerRatio`, which nothing on the disc calls: left out), the body sphere's stage push-out, a stage change's transition effects (`StgFx_Reset`'s blurs, cross-fade and warps); the sight flag 0xBA (`BtlChars_UpdateSightFlag`)
- [x] The camera's stage trace (`BtlCam_TraceStage`: already ported as `trace_camera`; a stale note)
- [x] The jaw and the look-at rotations of nodes 0x2E / 0x2F (already ported; a stale note)
- [x] The story mode's AI override (`BtlCharApi_GetStoryAiForce`): fighter word +0x1290 is never written in the executable, so it is always 0
- [x] The HUD's technique names above the command row (never shown on the PS2: pad status 1 never occurs), `Hud_SlideOut` (already ported)
- [x] Story script command 1701 (a button wait of pad 0) and command 8's stage change (`BtlStage_RequestChange`); none of the 50 story scripts uses them
- [x] `SndOpt_Apply` for the sound options loaded from the card (the main loop takes the save's options every frame: the music volume, stereo or mono)
- [x] `EftZap` (effect kind 12) and the water swirls (`EftWaterMist_*`), if anything reaches them: nothing does (no emitter set on the disc has kind 12; nothing calls `EftWater_AddBurst`)
- [x] The body effect of request 0x1A (`EftBodyFx`, common effect set 0x23F on the fighter's body)
- [x] The particle modules' hidden test when they draw (`BtlScene_IsEffectHidden` of each object's fighter by the kind it was made with; the rings and links draw regardless)

## Left to port

Gathered 2026-10-13 from the decompiled functions the port does not cite, checked one by one against the C, the
executable (callers and address references) and the ISO, and from the port's own "not ported" notes.

### Battle

- [x] Blade trails (`EftBlade_*`, src/battle/eft_char_parts.c, layer 3 entry 6): the sword trails of the eight characters in `gEftBladeDef` (0x3E, 0x8A, 0x27, 0x28, ...). Its manager's init draws libc `rand()` three times per fighter at the scene's start, after the stage effects' kinds
- [x] The ki blast's charge glow, the fighter effect kind 3 (`EftBlastCharge_*`, src/battle/eft_struggle.c): request 0x17 of a ki blast that is not a disc (`BtlFx_SpawnHitSparkReq17`'s other branch)
- [x] The character body effect, the fighter effect kind 7 (`EftCharaFx_*`, src/battle/eft_trail.c, `BtlFx_UpdateCharaFx`): characters 0x37, 0x98 and 0x99, from their pack's entry 9
- [x] The lost clash: `BtlAnim_EnableHandle` (the root's axes zeroed) when the motion plays again on the damaged model (src/battle/btl_act_super.c, `BtlAct_ClashLostHandler`)
- [x] The result poses' placement: flag 0xF3 puts the fighter at its second start place (`BtlChar_PlaceRestart`); the win and lose poses set it
- [x] `BtlOpp_MirrorFlags`: the opponent's flag 0x27 ends this fighter's camera cut (`ChrCam_EndCut`)
- [x] `BtlCharApi_IsCamShown`'s clash test: a fighter in clash A, B or C is not the one on screen (`BtlScene_IsCharInView`)
- [x] Model seams (`ObjSeam_*`, src/battle/obj_render.c): the CPU-skinned triangles between two nodes of a model with a seam section (header +0x70), drawn with the base texture and the shading ramp (73 of the disc's 767 models have one)
- [x] The objects' distance fade (`BtlObjDraw_DrawModelFade`): left out, as it depends on how the PS2 renders: a model with flag 1 draws its fade texture into the frame's alpha alone (FRAME_1 masking RGB), alpha-tested against the view's fade (40..120 units), blended through GS context 2's destination alpha
- [x] The streamed sounds' pan on the ADX players: every channel at the player's pan, so stereo files play centred

### Sound driver

- [x] SE tones with a noise voice, LFO, exclusive group or transpose, and the SE sequence events the port stops on: none of the 361 banks the game loads (the common, menu, stage and both voice sets' character banks) has any, which a real-ISO test checks; nothing to port

### System and front end

- [x] `PadWatch_*` (src/sys/pad_watch.c): the debounced readings, the warning (darkened screen, boot file text) and the battle pause for the missing controller (`pad_watch.rs`)
- [x] The character viewer's backdrop ambience (`StgAmb_Init` once a model is shown, `StgAmb_Term` at init and on hide): the backdrop's data says stage 5, its loop at volume 0x50
- [x] Movies: the PSS streams stay decoded by `ffmpeg`, not by a port of the IPU MPEG-2 path (`src/sys/movie.c`): MPEG-2 decoding is set by the standard, and how the IPU's colour conversion and dither differ from it is the PS2's own rendering, left out by the rule for visuals; the playback around it (timing, skip, fade, the soundtrack) is ported

### Notes to correct (the work is ported)

- [x] btl_act.rs: the volley dash's and the homing strikes' vibration, the rush start's sound and head look, the guard's head look
- [x] btl_chr.rs: `on_stage_loaded`'s head tracking (and its vibration stop, now ported), `damage`'s abilities
- [x] btl_blast.rs: the header's particles and impact effects; "the other kinds are not ported"
- [x] btl_tech_fx.rs ("abilities": every ability number the C tests is tested), btl_tech_shots.rs (its model, camera cut and connected-rush effects), btl_clash.rs (clash A's throw win; clash C's level now takes the story's difficulty), btl_mgr.rs (`BtlFx_UpdateLate`'s occlusion flag, now ported: a fighter whose camera trace hits the stage checks whether the camera is inside it)
- [x] main.rs: the story mode notes (`--story-result`, `--mode`), `leave_battle`'s "team select" and "replay screen"; README: "the menus that lead to these modes ... are not ported"

### Unreachable in the original (nothing to port)

Checked against the executable and the ISO: `GfxLens_*` (no caller), `EftTrail_*` (no caller), `EftCharNull_Start` (no caller), `EftZap` (no emitter set has kind 12), the water swirls (`EftWater_AddBurst` has no caller), the geyser's steam (no stage has the steam manager), the stage timer of stages 4 / 27 (read only by `BtlCharApi_GetMgrTimerRatio`, never called), the story AI override (fighter word +0x1290 never written), the technique names of pad status 1, story script command 1701 and command 8's stage change (no story script uses them; ported anyway).

## By function

For each decompiled C file of `../BT3-Decompiled/src`, the functions whose names the Rust code cites (the port names the original function in its doc comments). A rough measure: the system layer (heap, DMA, lists, timers) has no counterpart by design, a few files are ported under other names (the script command handlers, `BtlScriptCmd_*`), and the decompilation itself does not cover every module of the executable.

Measured 2026-10-13 (a function counts when its name appears anywhere in the Rust sources; many uncited ones are accessors or task callbacks folded into their callers):

| Area | Files | Functions | Cited by the port |
| --- | ---: | ---: | ---: |
| src/battle | 124 | 5051 | 2668 (52 %) |
| src/menu | 69 | 794 | 601 (75 %) |
| src/ui | 11 | 182 | 125 (68 %) |
| src/sys | 51 | 1133 | 359 (31 %) |
| src/cri, src/main.c | 2 | 7 | 1 |
| Total | 257 | 7167 | 3754 (52 %) |

Every menu file now has functions cited. The system layer (heap, DMA, lists, timers, the VU1 and GS packet builders) has no counterpart by design.
