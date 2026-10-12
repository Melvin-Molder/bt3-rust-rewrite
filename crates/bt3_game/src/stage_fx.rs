//! The stage effects of the scene's layer 0 (`EftStage_*`, src/battle/eft_stage_2.c): one task per kind of
//! `gEftStageKinds` the loaded stage has data for, updated and drawn in the table's order. Their inits and resets
//! draw random numbers (the sheet's direction, the smoke's first updates): those of the battle's start, of a
//! rematch and of a stage change run before the next update, in the order the original runs them (every init
//! in kind order, then the resets).

use crate::btl_chr::{Chr, SideData};
use crate::eft::{DrawCtx, OrderTable, V3};
use crate::rng::{LibcRandom, Vu0Random};

/// The tasks of the kinds this stage has.
#[derive(Clone, Debug, Default)]
pub(crate) struct StageFx {
    /// Kind 0.
    pub scroll: Option<crate::stg_scroll::Scroll>,
    /// Kind 4.
    pub smoke: Option<crate::eft_smoke::Smoke>,
    /// Kind 7: the water, then the bubbles.
    pub water: Option<crate::eft_water::Water>,
    pub bubbles: Option<crate::eft_bubble::Bubbles>,
    /// Kind 8.
    pub bound: Option<crate::eft_bound::Bound>,
    /// The inits still to run, the resets after them, and draws of the effects these replaced (taken first).
    init_pending: bool,
    resets_pending: u32,
    pub burn: u32,
}

impl StageFx {
    /// `EftStage_CreateKinds`: the tasks made, their inits to run.
    pub(crate) fn created(mut self) -> Self {
        self.init_pending = true;
        self
    }

    /// `BtlScene_Reset` of the kinds: the water's wakes stop now; the sheet's reload waits for the generator.
    pub(crate) fn reset(&mut self) {
        if let Some(w) = &mut self.water {
            w.reset();
        }
        self.resets_pending += 1;
    }

    /// The random draws a reset of these tasks makes (the sheet's, with a random direction).
    pub(crate) fn reset_draws(&self) -> u32 {
        u32::from(self.scroll.as_ref().is_some_and(|s| s.random()))
    }

    /// The inits and resets waiting, with the frame's generators.
    pub(crate) fn run_pending(&mut self, libc: &mut LibcRandom, vu0: &mut Vu0Random) {
        for _ in 0..std::mem::take(&mut self.burn) {
            libc.rand();
        }
        if std::mem::take(&mut self.init_pending) {
            if let Some(s) = &mut self.scroll {
                s.load(libc);
            }
            if let Some(s) = &mut self.smoke {
                s.init(libc, vu0);
            }
        }
        for _ in 0..std::mem::take(&mut self.resets_pending) {
            if let Some(s) = &mut self.scroll {
                s.load(libc);
            }
        }
    }

    /// The kinds' updates in the table's order (`paused`: the battle's pause, `BtlScene_IsTimeStopped`).
    pub(crate) fn update(
        &mut self,
        chars: &[Chr; 2],
        data: [&SideData; 2],
        paused: bool,
        libc: &mut LibcRandom,
        vu0: &mut Vu0Random,
    ) {
        if let Some(s) = &mut self.scroll {
            s.update(paused);
        }
        if let Some(s) = &mut self.smoke {
            s.update(paused, libc, vu0);
        }
        if let Some(water) = &mut self.water {
            water.update(&crate::btl_blast::water_chars(chars, data), paused, libc);
        }
        if let Some(b) = &mut self.bubbles {
            let chars = std::array::from_fn(|i| {
                let c = &chars[i];
                crate::eft_bubble::BubbleChr {
                    pos: c.pose.pos,
                    dir: c.pose.dir,
                    height: data[i].height(),
                    in_water: c.flags.test(0x11),
                    nodes: std::array::from_fn(|n| c.obj.node_pos(n as u32)),
                }
            });
            b.update(&chars, paused, libc);
        }
        if let Some(b) = &mut self.bound {
            let chars = chars.each_ref().map(|c| crate::eft_bound::BoundChr {
                node3: c.obj.node_pos(3),
                in_technique: crate::btl_act::is_technique_id(c.act.current),
                rush_connected: c.flags.test(0xA1),
            });
            b.update(&chars, paused);
        }
    }

    /// The kinds' draws into the effect pass (the sheet is drawn with the stage). `light`: the stage's light
    /// direction.
    pub(crate) fn draw(&self, ot: &mut OrderTable, ctx: &DrawCtx, chars: &[Chr; 2], light: V3) {
        if let Some(s) = &self.smoke {
            s.draw(ot, ctx, light);
        }
        if let Some(w) = &self.water {
            w.draw(ot, ctx);
        }
        if let Some(b) = &self.bubbles {
            b.draw(ot, &ctx.world2view, &ctx.world2screen);
        }
        if let Some(b) = &self.bound {
            let show = chars.each_ref().map(|c| crate::eft_bound::BoundShow {
                hidden: c.obj.flags & 2 == 0,
                locked_on: c.flags.test(5),
                rush_connected: c.flags.test(0xA1),
            });
            b.draw(ot, ctx, &show);
        }
    }
}
