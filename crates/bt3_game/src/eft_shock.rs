//! The shock (`EftShock_*`, src/battle/eft_zap.c): fighter effect request 6. At node 3 a screen shock wave, then
//! two animated parts of the character pack's entry 6 two frames apart; it ends when both are gone. A kind 4
//! manager of the fighters' effect layer, two shocks per fighter.

use bt3_data::eft_pack::SprPack;

use crate::eft::{Modules, SetRef, V3};
use crate::rng::Vu0Random;
use crate::scr_warp::ScrWarps;

/// `EftShockMgr_Init`: two shocks per fighter.
const SHOCKS: usize = 2;
/// The `SetRef::list` of a fighter's shock parts.
pub(crate) const LIST: usize = 8;

// EFT_SHOCK_*.
const WARPED: u32 = 0x1;
const WAIT: u32 = 0x2;
const DONE: u32 = 0x4;

/// What a shock reads of its fighter.
pub(crate) struct ShockChr {
    pub side: usize,
    /// Node 3.
    pub pos: V3,
    /// `BtlCharApi_GetDir`.
    pub dir: V3,
    /// `BtlScene_GetCharScale`.
    pub scale: f32,
}

/// `EftShock`.
#[derive(Clone, Copy, Debug)]
struct Shock {
    flags: u32,
    timer: i32,
    count: usize,
    parts: [Option<u32>; 2],
}

/// A fighter's shocks.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shocks {
    list: Vec<Shock>,
}

impl Shocks {
    /// Live shocks.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.list.len()
    }

    /// `EftShock_Start`: nothing when both are in use.
    pub(crate) fn start(&mut self) {
        if self.list.len() < SHOCKS {
            self.list.push(Shock {
                flags: 0,
                timer: 2,
                count: 0,
                parts: [None; 2],
            });
        }
    }

    /// `EftShock_Update` of every shock (not while paused: `BtlScene_IsEffectStopped(.., 3)`): the shock wave
    /// once, a part (mode 4, 0.8 s, twice the fighter's scale) on its first and third updates; done once both
    /// parts are gone. `pack`: the character pack's entry 6's animation.
    pub(crate) fn update(
        &mut self,
        fx: &mut Modules,
        warps: &mut ScrWarps,
        c: &ShockChr,
        pack: &SprPack,
        paused: bool,
        vu0: &mut Vu0Random,
    ) {
        let set = SetRef {
            side: c.side,
            list: LIST,
        };
        let mut i = 0;
        while i < self.list.len() {
            let s = &mut self.list[i];
            if !paused {
                if s.flags & WARPED == 0 {
                    warps.spawn(c.side, c.pos, 0.8, 10.0, 50.0, 8.0, 0.4, vu0);
                    s.flags |= WARPED;
                }
                if s.flags & WAIT == 0 {
                    let id = fx.anim_create(
                        c.side,
                        4,
                        0.8,
                        c.pos,
                        c.dir,
                        c.scale * 2.0,
                        set,
                        0,
                        0,
                        pack,
                    );
                    if let Some(p) = s.parts.get_mut(s.count) {
                        *p = id;
                    }
                    s.count += 1;
                    s.flags |= WAIT;
                }
                if s.timer > 0 {
                    s.timer -= 1;
                    if s.timer == 0 {
                        s.flags &= !WAIT;
                    }
                }
                if s.count == 2
                    && !s
                        .parts
                        .iter()
                        .flatten()
                        .any(|&p| fx.alive(crate::eft::Handle::Anim(p)))
                {
                    s.flags |= DONE;
                }
            }
            if s.flags & DONE != 0 {
                // EftShock_Term: the parts are killed.
                for p in s.parts.iter().flatten() {
                    fx.kill(crate::eft::Handle::Anim(*p));
                }
                self.list.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// `EftShock_Reset`.
    pub(crate) fn reset(&mut self, fx: &mut Modules) {
        for s in &self.list {
            for p in s.parts.iter().flatten() {
                fx.kill(crate::eft::Handle::Anim(*p));
            }
        }
        self.list.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shock_starts_two_parts_two_frames_apart_and_ends_with_them() {
        let pack = SprPack::default();
        let mut s = Shocks::default();
        let mut fx = Modules::default();
        let mut warps = ScrWarps::default();
        let mut vu0 = Vu0Random::default();
        let c = ShockChr {
            side: 1,
            pos: [0.0, 0.0, 100.0],
            dir: [0.0, 0.0, 1.0],
            scale: 1.0,
        };
        s.start();
        s.start();
        s.start();
        assert_eq!(s.count(), 2);
        s.update(&mut fx, &mut warps, &c, &pack, false, &mut vu0);
        // One wave and one part each.
        assert_eq!(warps.count(), 2);
        assert_eq!(s.list[0].count, 1);
        s.update(&mut fx, &mut warps, &c, &pack, false, &mut vu0);
        assert_eq!(s.list[0].count, 1);
        s.update(&mut fx, &mut warps, &c, &pack, false, &mut vu0);
        assert_eq!(s.list[0].count, 2);
        assert!(s.list[0].parts.iter().all(Option::is_some));
        // A pause holds them.
        s.update(&mut fx, &mut warps, &c, &pack, true, &mut vu0);
        assert_eq!(s.count(), 2);
        s.reset(&mut fx);
        assert_eq!(s.count(), 0);
    }
}
