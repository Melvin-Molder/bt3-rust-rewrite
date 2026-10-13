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

- [ ] Ultimate Battle menu (`UbMenu_Run`, mode 13)
- [ ] Mission 100: select, team / solo select, result (modes 14..16)
- [~] Survival: the battle runs (`--mode 3`); the course select and result screens do not (modes 17..19)
- [ ] Sim Dragon: top, day, events, result (modes 20..23; src/menu/sim_*.c)
- [ ] Disc Fusion (mode 24)
- [ ] Ultimate Battle Z ranking and course select (modes 25..30; src/menu/ub_*.c, ubz_select.c)

### World Tournament (`Tour_Main`, modes 33..35)

- [ ] Entry select, tournament menu, bracket and its battles (src/menu/entry_select.c, tour_*.c, bracket*.c)

### Training (`TrainMode_Main`, modes 44..45)

- [x] Training menu (`Train_Run`, mode 44; src/menu/training.c): classes, lessons, pages, the lesson battles and their "cleared" bits
- [x] Free training's character select (`CharSel_Run(2)`, mode 45) and battle
- [x] The tutorials' goals (the fighters' frame bits, `BtlCtrl_TestTutorialFrameBit` → `HudNotice_Show(8)`)

### Evolution Z (`EvoMode_Main`, modes 48..50)

- [ ] Top menu, character customising, item shop (src/menu/evo_*.c, shop.c)

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
- [ ] The objects' frame-alpha passes of `BtlObj_BeginDraw` (`GfxAlphaKey`, `ObjOutline`, `ObjGlow`: each material's alpha id in the frame, the glow of `BtlObj_SetAlphaAdd`; src/sys/gfx_post.c, gfx_screen.c)

## By function

For each decompiled C file of `../BT3-Decompiled/src`, the functions whose names the Rust code cites (the port names the original function in its doc comments). A rough measure: the system layer (heap, DMA, lists, timers) has no counterpart by design, a few files are ported under other names (the script command handlers, `BtlScriptCmd_*`), and the decompilation itself does not cover every module of the executable.

Measured 2026-10-12:

| Area | Files | Functions | Cited by the port |
| --- | ---: | ---: | ---: |
| src/battle | 124 | 5049 | 2579 (51 %) |
| src/menu | 65 | 757 | 236 (31 %) |
| src/ui | 11 | 182 | 63 (35 %) |
| src/sys | 51 | 1133 | 338 (30 %) |
| src/main.c | 1 | 5 | 0 |
| Total | 252 | 7126 | 3216 (45 %) |

Menu files with no function cited yet: bracket.c, bracket_clips.c, bracket_guide.c, bracket_logic.c, char_reference.c, disc_fusion.c, entry_select.c, evo_mode.c, evo_top.c, evo_z_1.c, evo_z_2.c, evo_z_3.c, mission_result.c, mission_select.c, shop.c, sim_day.c, sim_event.c, sim_event_1.c, sim_event_card.c, sim_event_rock.c, sim_event_shell.c, sim_result.c, sim_top.c, solo_select.c, survival_result.c, tour_background.c, tour_menu.c, ub_menu.c, ub_rank.c, ub_result.c, ub_score.c, ub_team_select.c, ubz_select.c.
