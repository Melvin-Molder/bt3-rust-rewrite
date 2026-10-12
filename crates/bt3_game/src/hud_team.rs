//! The HUD's team panel (src/battle/hud_team.c): for a side with members in reserve, the face of the member a
//! switch would bring in, its health and the switch gauge, flipped to the next face when the target changes and
//! slid away while the side switches. One tree serves both sides; side 1's is drawn mirrored (x' = 511 - x) with
//! the faces mirrored back.
//!
//! Nodes: 0 root (12, 66) > 1 the current face group > { 3 face A (14, 0) > { 5 health A (46, 4), 7 switch gauge
//! (41, 12) }, 8 plate }, and 2 the previous face group (shown until its flip ends) > { 4 face B > { 6 health B,
//! 7 }, 8 }.

use crate::battle_seq::Ramp;
use crate::hud::{HudSheet, Sprite};
use crate::hud_gauge::{HudTextures, Painter};
use crate::hud_gs::GsLayer;

/// `HUD_TEAM_GAUGE_FULL`.
const GAUGE_FULL: i32 = 100_000;

/// What `Hud_PreUpdate` reads of a side for the panel (`BtlCtrl_*` / `BtlSide_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SideTeam {
    /// Flags 0xE4 / 0xE5: the switch target moved to the next / previous member.
    pub(crate) flip_next: bool,
    pub(crate) flip_prev: bool,
    /// `BtlSide_CountAlive`.
    pub(crate) alive: i32,
    pub(crate) gauge: i32,
    /// The switch target's health and maximum (0 with fewer than two alive) and its index (-1 then).
    pub(crate) hp: i32,
    pub(crate) hp_max: i32,
    pub(crate) target: i32,
    /// `BtlCtrl_IsSwitching`: in one of actions 0xF3..0xF8.
    pub(crate) switching: bool,
}

/// `gHudTeam`.
pub(crate) struct TeamPart {
    spr: [Sprite; 12],
    side: usize,
    hp: [i32; 2],
    hp_max: [i32; 2],
    gauge: [i32; 2],
    reserve: [i32; 2],
    shown: [bool; 2],
    switching: [bool; 2],
    flip_done: [bool; 2],
    target: [i32; 2],
    target_prev: [i32; 2],
    prev_hp: [i32; 2],
    prev_hp_max: [i32; 2],
    flash: [Ramp; 2],
    slide: [Ramp; 2],
    flip_a: [Ramp; 2],
    flip_b: [Ramp; 2],
    /// `hide`: `HudTeam_SlideOut` / `SlideIn` (`Hud_SlideOut` is not ported).
    hide: Ramp,
    /// `BATTLE_FLAG_PAUSE`: the ramps stand still.
    pub(crate) paused: bool,
}

/// A face group's offset for its flip value (`HudTeam_UpdateRoot`).
fn flip_pos(v: f32) -> [i32; 2] {
    let (x, y) = if 0.0 < v {
        (-v, v)
    } else if v < 0.0 {
        (v, v)
    } else {
        (0.0, 0.0)
    };
    [(x * 128.0) as i32, (y * 32.0) as i32]
}

fn add(a: [i32; 2], b: [i32; 2]) -> [i32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

/// `rate * 30` of a bar (`HudTeam_UpdateHpA` / `UpdateGauge`).
fn bar_shift(cur: i32, max: i32) -> i32 {
    ((cur as f32 / max as f32) * 30.0) as i32
}

impl TeamPart {
    /// `HudTeam_SlideOut` / `HudTeam_SlideIn`: both panels.
    pub(crate) fn slide(&mut self, out: bool, seconds: f32) {
        if out {
            self.hide.start(seconds, 0.0, 1.0);
        } else {
            self.hide.start(seconds, 1.0, 0.0);
        }
    }

    /// `HudTeam_Init` (then `HudTeam_Reset`).
    pub(crate) fn new(sheet: &HudSheet) -> Self {
        let mut spr = [Sprite::default(); 12];
        let tex = |spr: &mut Sprite, tex: usize, rect: [i32; 4], uv: [i32; 4]| {
            spr.show(true);
            spr.init_tex_sub(sheet, tex, 0);
            spr.set_rect(rect[0], rect[1], rect[2], rect[3]);
            spr.set_uv(uv[0], uv[1], uv[2], uv[3]);
        };
        tex(&mut spr[0], 0x17, [0, 0x5E, 0, 0x14], [0, 0x5E, 0, 0x14]);
        for i in [8, 9] {
            spr[i].show(true);
            spr[i].init_plain([0x80; 4]);
            spr[i].set_rect(0, 0x20, 0, 0x20);
        }
        // The bars (1 / 2 / 5) over their masks (3 / 4 / 6).
        for i in [1, 2, 5] {
            tex(&mut spr[i], 0x18, [0, 0x20, 0, 0x10], [0, 0x20, 0x10, 0x20]);
        }
        for i in [3, 4] {
            tex(&mut spr[i], 0x18, [0, 0x20, 0, 0x10], [0, 0x20, 0, 0x10]);
        }
        tex(&mut spr[6], 0x18, [0, 0x20, 0, 0x10], [0x40, 0x60, 0, 0x10]);
        tex(&mut spr[7], 0x11, [0, 0x30, 0, 0x10], [0, 0x30, 0x20, 0x30]);
        spr[7].show(false);
        spr[7].move_by(-3, -3);
        tex(
            &mut spr[10],
            0x17,
            [0, 0x1E, 0, 0x14],
            [0x60, 0x7E, 0, 0x14],
        );
        tex(
            &mut spr[11],
            0x18,
            [0, 0x10, 0, 0x10],
            [0x40, 0x50, 0x10, 0x20],
        );
        spr[11].move_by(-1, 3);
        Self {
            spr,
            side: 0,
            hp: [0; 2],
            hp_max: [0; 2],
            gauge: [0; 2],
            reserve: [0; 2],
            shown: [false; 2],
            switching: [false; 2],
            flip_done: [false; 2],
            target: [0; 2],
            target_prev: [0; 2],
            prev_hp: [0; 2],
            prev_hp_max: [0; 2],
            flash: [Ramp::default(); 2],
            slide: [Ramp::default(); 2],
            flip_a: [Ramp::default(); 2],
            flip_b: [Ramp::default(); 2],
            hide: Ramp::default(),
            paused: false,
        }
    }

    /// The `HudTeam_*` calls of `Hud_PreUpdate` for one side (not while paused).
    pub(crate) fn set_side(&mut self, side: usize, t: &SideTeam) {
        if t.flip_next {
            self.start_flip(side, 1.0);
        }
        if t.flip_prev {
            self.start_flip(side, -1.0);
        }
        // HudTeam_SetReserveCount.
        let reserve = t.alive - i32::from(t.alive > 0);
        self.reserve[side] = reserve;
        self.shown[side] = reserve != 0;
        // HudTeam_SetSwitchGauge: becoming full starts the marker's blink.
        if self.gauge[side] != GAUGE_FULL && t.gauge == GAUGE_FULL {
            self.flash[side].start(1.0, 1.0, 0.0);
        }
        self.gauge[side] = t.gauge;
        self.hp[side] = t.hp;
        self.hp_max[side] = t.hp_max;
        self.target[side] = t.target;
        // HudTeam_SetSwitching: the panel slides away over 0.3 s, and back.
        if self.switching[side] != t.switching {
            if t.switching {
                self.slide[side].start(0.3, 0.0, 1.0);
            } else {
                self.slide[side].start(0.3, 1.0, 0.0);
            }
        }
        self.switching[side] = t.switching;
    }

    /// `HudTeam_StartFlipNext` (`dir` 1) / `StartFlipPrev` (-1): the new face comes in from one side, the old one
    /// with the health it had leaves to the other, over 0.25 s.
    fn start_flip(&mut self, side: usize, dir: f32) {
        self.flip_a[side].start(0.25, dir, 0.0);
        self.flip_b[side].start(0.25, 0.0, -dir);
        self.target_prev[side] = self.target[side];
        self.prev_hp[side] = self.hp[side];
        self.prev_hp_max[side] = self.hp_max[side];
        self.flip_done[side] = false;
    }

    /// `HudTeam_SelectSide`, `HudNode_Update` and `HudNode_Draw` of the tree for one side.
    pub(crate) fn run(&mut self, side: usize, layer: &mut GsLayer, textures: &HudTextures) {
        // HudTeam_SelectSide.
        self.side = side;
        let mirror = side != 0;
        self.spr[8].mirror = mirror;
        self.spr[9].mirror = mirror;
        let shown = self.shown[side];
        // HudTeam_UpdateRoot.
        if !self.paused {
            self.slide[side].step();
            if side == 0 {
                self.hide.step();
            }
        }
        let (slide, hide) = (self.slide[side].value, self.hide.value);
        let root = [
            (slide * -256.0) as i32 + (hide * -256.0) as i32 + 12,
            (slide * -224.0) as i32 + (hide * -224.0) as i32 + 66,
        ];
        if !self.paused {
            if self.flip_a[side].step() {
                self.flip_a[side].stop();
            }
            if self.flip_b[side].step() {
                self.flip_b[side].stop();
                self.flip_done[side] = true;
            } else {
                self.flip_done[side] = false;
            }
        }
        let group_a = add(root, flip_pos(self.flip_a[side].value));
        let group_b = add(root, flip_pos(self.flip_b[side].value));
        let group_b_shown = !self.flip_done[side];
        // HudTeam_UpdateFaceA / UpdateFaceB: the face moves over for the plate with two or more in reserve.
        let plate = self.reserve[side] >= 2;
        let face = if plate { [14, 0] } else { [0, 0] };
        // The current group's updates, then the previous group's (the gauge's twice).
        self.update_bar(1, self.hp[side], self.hp_max[side]);
        self.update_gauge();
        self.update_bar(2, self.prev_hp[side], self.prev_hp_max[side]);
        self.update_gauge();
        if !shown {
            return;
        }
        let mut paint = Painter {
            layer,
            textures,
            mirror,
        };
        for (group, shown, target, face_spr, hp, hp_mask) in [
            (group_a, true, self.target[side], 8, 1, 3),
            (group_b, group_b_shown, self.target_prev[side], 9, 2, 4),
        ] {
            if !shown {
                continue;
            }
            let face_at = add(group, face);
            // HudTeam_DrawFaceA / DrawFaceB: the frame, then texture 0 of the member's face sheet.
            paint.draw(&self.spr[0], face_at, 0, None);
            let member = usize::try_from(target)
                .ok()
                .and_then(|m| textures.members[side].get(m))
                .and_then(Option::as_ref);
            if let Some(t) = member {
                let spr = &mut self.spr[face_spr];
                spr.tex = 0;
                spr.sub = 0;
                spr.set_rect(0, t.width as i32, 0, t.height as i32);
                spr.set_uv(0, t.width as i32, 0, t.height as i32);
                spr.rgba = [0x80; 4];
                spr.move_by(-10, -11);
                paint.draw_texture(&self.spr[face_spr], face_at, 0, Some(t));
            }
            // HudTeam_DrawHpA / DrawHpB: the bar through its mask.
            let at = add(face_at, [0x2E, 4]);
            paint.alpha_clear(&self.spr[hp_mask], at);
            paint.begin_mask();
            paint.draw(&self.spr[hp], at, 1, None);
            paint.draw(&self.spr[hp_mask], at, 0, None);
            paint.end_mask();
            // HudTeam_DrawGauge: the switch gauge through its mask, then the "full" marker.
            let at = add(face_at, [0x29, 12]);
            paint.alpha_clear(&self.spr[6], at);
            paint.begin_mask();
            paint.draw(&self.spr[5], at, 1, None);
            paint.draw(&self.spr[6], at, 0, None);
            paint.end_mask();
            paint.draw(&self.spr[7], at, 0, None);
            // The plate.
            if plate {
                paint.draw(&self.spr[10], group, 0, None);
                paint.draw(&self.spr[11], group, 0, None);
            }
        }
    }

    /// `HudTeam_UpdateHpA` / `UpdateHpB`: the 32-wide bar shifted right by 30 * health / maximum.
    fn update_bar(&mut self, i: usize, cur: i32, max: i32) {
        let spr = &mut self.spr[i];
        spr.set_rect(0, 32, 0, 16);
        spr.move_by(bar_shift(cur, max), 0);
    }

    /// `HudTeam_UpdateGauge`: the switch gauge's bar; when full the marker shows, its alpha running 0 -> 128 -> 0
    /// over two seconds.
    fn update_gauge(&mut self) {
        let side = self.side;
        let gauge = self.gauge[side];
        self.update_bar(5, gauge, GAUGE_FULL);
        if gauge == GAUGE_FULL {
            self.spr[7].show(true);
            let done = !self.paused && self.flash[side].step();
            if done {
                let f = &mut self.flash[side];
                if f.value == 0.0 {
                    f.start(1.0, 0.0, 1.0);
                } else {
                    f.start(1.0, 1.0, 0.0);
                }
            }
            let a = (self.flash[side].value * 128.0) as u8;
            self.spr[7].set_color(0x80, 0x80, 0x80, a);
        } else {
            self.spr[7].show(false);
            self.spr[7].set_color(0x80, 0x80, 0x80, 0x80);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flips_slide_the_faces_and_hide_the_old_group_when_done() {
        let mut part = TeamPart::new(&HudSheet::default());
        let side = SideTeam {
            alive: 3,
            gauge: 50_000,
            hp: 20000,
            hp_max: 40000,
            target: 1,
            ..SideTeam::default()
        };
        part.set_side(0, &side);
        assert!(part.shown[0]);
        assert_eq!(part.reserve[0], 2);
        part.set_side(
            0,
            &SideTeam {
                flip_next: true,
                target: 2,
                ..side
            },
        );
        assert_eq!(part.target_prev[0], 1);
        assert_eq!(part.prev_hp[0], 20000);
        let mut layer = GsLayer::new();
        let textures = HudTextures::default();
        layer.begin(false);
        part.run(0, &mut layer, &textures);
        assert!(!part.flip_done[0]);
        assert_eq!(
            flip_pos(part.flip_a[0].value)[1],
            (part.flip_a[0].value * 32.0) as i32
        );
        for _ in 0..8 {
            part.run(0, &mut layer, &textures);
        }
        assert!(part.flip_done[0]);
        assert_eq!(part.flip_a[0].value, 0.0);
        // Half the gauge: the bar moved 15 to the right.
        assert_eq!(part.spr[5].rect[0], 15);
    }
}
