//! The battle HUD (src/battle/hud.c, hud_sprite.c, hud_notice_1.c, hud_notice_2.c): the manager's visibility
//! bits, the team panel (hud_team.rs), the gauges and timer (hud_gauge.rs), the combo part (hud_combo.rs) and the
//! notice part: the READY / FIGHT announcements, the finishes (K.O., the other K.O., the two spinning words of time
//! up and the like), the winner's and the result's lower banners, with the announcer's lines; the caption of the
//! attract demo; and the button prompts (hud_prompt.rs).
//!
//! The notices lean on the GS destination alpha: a band written only into the alpha plane lets the word through
//! where it covers (TEST_2 0x3C000: DATE with DATM 1), and the FIGHT streak is added in proportion to the alpha
//! the word wrote (ALPHA_2 0x58: Cs * Ad + Cd). The band is a convex quad, so the word is clipped to it here; the
//! streak carries the word's texture as its weight.

use crate::battle_seq::Ramp;
use crate::hud_gauge::{GaugePart, HudTextures, SideGauge, TimerPart, draw_gauges};
use crate::hud_gs::GsLayer;
use crate::rng::{GameRandom, LibcRandom};
use crate::title::TitleSound;

/// `Hud_Init`: the file `6 + gProgress->unk0[0]`; sheet k sits at the byte offset in header word `SHEET_WORDS[k]`.
pub(crate) const HUD_FILE: u32 = 6;
pub(crate) const SHEET_WORDS: [usize; 5] = [1, 4, 5, 2, 3];
const NOTICE_SHEET: usize = 1;
const COMBO_SHEET: usize = 2;
const CAPTION_SHEET: usize = 4;
const PROMPT_SHEET: usize = 3;
/// The font's icon sheet, appended after the HUD file's five.
const ICON_SHEET: usize = 5;
/// `HudNotice_GetVoiceBase`: the announcer's lines of the voice set of the options.
pub(crate) fn announcer_base() -> u32 {
    if crate::sound::options().alt_voice {
        0x10B6C
    } else {
        0x10B34
    }
}
const ANNOUNCER_LINES: u32 = 7;
const SHOW_ALL: u32 = 0x1F;
const SHOW_GAUGES: u32 = 1;
const SHOW_TIMER: u32 = 2;
const SHOW_NOTICE: u32 = 4;
const SHOW_COMBO: u32 = 8;
const SHOW_PROMPT: u32 = 0x10;

const SPR_WORD: usize = 0;
const SPR_PART: usize = 1;
const SPR_STREAK: usize = 13;
const SPR_BAND: usize = 14;
const SPR_REPLAY: usize = 15;
/// Where the twelve fragments of the word fly to.
const PART_OFS: [[i16; 2]; 12] = [
    [40, -70],
    [190, -40],
    [190, 40],
    [90, 90],
    [-109, -61],
    [-160, -20],
    [-210, 10],
    [-170, 60],
    [30, -60],
    [160, -30],
    [-150, 30],
    [-100, 60],
];

/// A sprite sheet: where its textures start in the battle's HUD texture list, and each one's TEX0 size.
#[derive(Clone, Debug, Default)]
pub(crate) struct HudSheet {
    pub(crate) base: usize,
    pub(crate) sizes: Vec<[i32; 2]>,
    /// Extra decodings: entry `.0.0` looked up through entry `.0.1`'s palette, at texture `.1`.
    pub(crate) views: Vec<((usize, usize), usize)>,
}

impl HudSheet {
    /// The texture of entry `tex` drawn with entry `clut`'s palette (`BtlMenu_GetTex0(tex, clut)`).
    pub(crate) fn view(&self, tex: usize, clut: usize) -> Option<usize> {
        if tex == clut {
            return Some(self.base + tex);
        }
        self.views
            .iter()
            .find(|(k, _)| *k == (tex, clut))
            .map(|&(_, t)| t)
    }
}

/// One corner of a HUD triangle: GS screen pixels, texture coordinates (0..1) and the mask texture's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub mask_uv: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HudBlend {
    /// ALPHA 0x44: (Cs - Cd) * As + Cd.
    Alpha,
    /// ALPHA 0x58 over the alpha a texture wrote: Cs * Ad + Cd, Ad = mask texel alpha * the prim's alpha / 128.
    AddMasked(usize),
}

/// Triangles of one sprite (three vertices each) with its texture, GS colour and blend.
#[derive(Clone, Debug, PartialEq)]
pub struct HudPrim {
    pub texture: Option<usize>,
    pub vertices: Vec<HudVertex>,
    pub rgba: [u8; 4],
    pub blend: HudBlend,
}

/// `HudCSprite`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sprite {
    pub(crate) hidden: bool,
    /// u0 and u1 change places when drawn (`HudSprite_SetMirror`).
    pub(crate) mirror: bool,
    pub(crate) tex: i16,
    /// `texSub`: the palette comes from entry `tex + sub`.
    pub(crate) sub: u16,
    /// x0, x1, y0, y1 relative to the node.
    pub(crate) rect: [i16; 4],
    /// u0, u1, v0, v1 in texels.
    pub(crate) uv: [i16; 4],
    pub(crate) rgba: [u8; 4],
}

impl Sprite {
    pub(crate) fn show(&mut self, on: bool) {
        self.hidden = !on;
    }

    pub(crate) fn set_rect(&mut self, x0: i32, x1: i32, y0: i32, y1: i32) {
        self.rect = [x0 as i16, x1 as i16, y0 as i16, y1 as i16];
    }

    pub(crate) fn set_uv(&mut self, u0: i32, u1: i32, v0: i32, v1: i32) {
        self.uv = [u0 as i16, u1 as i16, v0 as i16, v1 as i16];
    }

    pub(crate) fn set_tex(&mut self, tex: i16, sub: u16) {
        self.tex = tex;
        self.sub = sub;
    }

    pub(crate) fn set_color(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.rgba = [r, g, b, a];
    }

    pub(crate) fn init_plain(&mut self, rgba: [u8; 4]) {
        self.tex = -1;
        self.uv = [0; 4];
        self.rgba = rgba;
    }

    pub(crate) fn move_by(&mut self, dx: i32, dy: i32) {
        let [x0, x1, y0, y1] = self.rect.map(i32::from);
        self.set_rect(x0 + dx, x1 + dx, y0 + dy, y1 + dy);
    }

    pub(crate) fn scale(&mut self, sx: f32, sy: f32) {
        let [x0, x1, y0, y1] = self.rect.map(f32::from);
        self.rect = [
            (x0 * sx) as i16,
            (x1 * sx) as i16,
            (y0 * sy) as i16,
            (y1 * sy) as i16,
        ];
    }

    pub(crate) fn center(&mut self) {
        let [x0, x1, y0, y1] = self.rect.map(i32::from);
        let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
        self.set_rect(x0 - cx, x1 - cx, y0 - cy, y1 - cy);
    }

    /// `HudSprite_InitTex`: the whole texture at its size, neutral colour.
    fn init_tex(&mut self, sheet: &HudSheet, tex: usize) {
        self.init_tex_sub(sheet, tex, 0);
    }

    pub(crate) fn init_tex_sub(&mut self, sheet: &HudSheet, tex: usize, sub: u16) {
        let [w, h] = sheet.sizes.get(tex).copied().unwrap_or([0, 0]);
        self.tex = tex as i16;
        self.sub = sub;
        self.set_rect(0, w, 0, h);
        self.set_uv(0, w, 0, h);
        self.rgba = [0x80; 4];
    }
}

/// A 2D affine matrix for row vectors: p' = p * m + t.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Xf {
    m: [[f32; 2]; 2],
    t: [f32; 2],
}

impl Xf {
    pub(crate) fn translate(x: f32, y: f32) -> Self {
        Self {
            m: [[1.0, 0.0], [0.0, 1.0]],
            t: [x, y],
        }
    }

    /// `HudNode_Draw`'s mirror (`Vu0Cur_MulMtxRev` of x' = 511 - x): applied after `self`.
    pub(crate) fn mirrored(&self) -> Self {
        let mut out = *self;
        out.m[0][0] = -out.m[0][0];
        out.m[1][0] = -out.m[1][0];
        out.t[0] = 511.0 - out.t[0];
        out
    }

    /// `Mtx_RotateZ` rows (c, s), (-s, c).
    fn rotate(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            m: [[c, s], [-s, c]],
            t: [0.0, 0.0],
        }
    }

    /// `Vu0Cur_MulMtx(self)` under `parent`: `self` applies first.
    fn then(&self, parent: &Xf) -> Self {
        let mut out = Xf::translate(0.0, 0.0);
        out.m = std::array::from_fn(|i| parent.apply_linear(self.m[i]));
        out.t = parent.apply(self.t);
        out
    }

    fn apply_linear(&self, p: [f32; 2]) -> [f32; 2] {
        [
            p[0] * self.m[0][0] + p[1] * self.m[1][0],
            p[0] * self.m[0][1] + p[1] * self.m[1][1],
        ]
    }

    fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let [x, y] = self.apply_linear(p);
        [x + self.t[0], y + self.t[1]]
    }
}

/// `HudSprite_DrawAt`'s corners: top left, top right, bottom right, bottom left, transformed and cut to whole
/// pixels (`(s32)` of the VU0 result), with their texel centres (UV = texel * 16 + 8).
pub(crate) fn sprite_polygon(spr: &Sprite, xf: &Xf, sheet: &HudSheet) -> Vec<HudVertex> {
    let [x0, x1, y0, y1] = spr.rect.map(f32::from);
    let [u0, u1, v0, v1] = spr.uv.map(f32::from);
    let [w, h] = usize::try_from(spr.tex)
        .ok()
        .and_then(|tex| sheet.sizes.get(tex))
        .map_or([1.0, 1.0], |s| s.map(|v| v.max(1) as f32));
    [
        (x0, y0, u0, v0),
        (x1, y0, u1, v0),
        (x1, y1, u1, v1),
        (x0, y1, u0, v1),
    ]
    .into_iter()
    .map(|(x, y, u, v)| HudVertex {
        pos: xf.apply([x, y]).map(f32::trunc),
        uv: [(u + 0.5) / w, (v + 0.5) / h],
        mask_uv: [0.0; 2],
    })
    .collect()
}

fn lerp_vertex(a: &HudVertex, b: &HudVertex, t: f32) -> HudVertex {
    let mix = |p: [f32; 2], q: [f32; 2]| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
    HudVertex {
        pos: mix(a.pos, b.pos),
        uv: mix(a.uv, b.uv),
        mask_uv: mix(a.mask_uv, b.mask_uv),
    }
}

/// The part of a convex polygon inside a convex quad (Sutherland-Hodgman), for either winding of the quad.
fn clip_to_quad(mut poly: Vec<HudVertex>, quad: &[[f32; 2]]) -> Vec<HudVertex> {
    let area: f32 = (0..quad.len())
        .map(|i| {
            let (a, b) = (quad[i], quad[(i + 1) % quad.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if area == 0.0 {
        return Vec::new();
    }
    let sign = area.signum();
    for i in 0..quad.len() {
        let (a, b) = (quad[i], quad[(i + 1) % quad.len()]);
        let side =
            |p: [f32; 2]| sign * ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]));
        let mut out = Vec::with_capacity(poly.len() + 1);
        for (j, cur) in poly.iter().enumerate() {
            let next = &poly[(j + 1) % poly.len()];
            let (dc, dn) = (side(cur.pos), side(next.pos));
            if dc >= 0.0 {
                out.push(*cur);
            }
            if (dc >= 0.0) != (dn >= 0.0) {
                out.push(lerp_vertex(cur, next, dc / (dc - dn)));
            }
        }
        poly = out;
        if poly.is_empty() {
            break;
        }
    }
    poly
}

pub(crate) fn fan(poly: &[HudVertex]) -> Vec<HudVertex> {
    (1..poly.len().saturating_sub(1))
        .flat_map(|i| [poly[0], poly[i], poly[i + 1]])
        .collect()
}

/// `HudNotice_Show`'s announcements by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    /// 0, 1.
    Ready,
    Fight,
    /// 2: K.O.
    Ko,
    /// 3 and 8: a word on a band (texture 3, rows 64..128 / 0..64).
    BandA,
    BandB,
    /// 4, 5: a spinning word (texture 4, rows 0..64 / 64..128).
    SpinA,
    SpinB,
    /// 6: the word zooming in with its after-images (texture 5).
    Trail,
    /// 7: the lower banner (texture 6).
    Banner,
}

/// `gHudNotice`.
#[derive(Debug)]
struct NoticePart {
    sheet: HudSheet,
    spr: [Sprite; 16],
    kind: Option<Notice>,
    /// Node 1 (the announcement) with its rotation (the spins) and node 2's rotation (the band of READY).
    pos: [i32; 2],
    rot: f32,
    band_rot: f32,
    /// The after-images' ramp values of the last frames (K.O., the trail).
    trail: [f32; 8],
    state: i32,
    streak: i32,
    ramp_a: Ramp,
    ramp_b: Ramp,
}

impl NoticePart {
    /// `HudNotice_Init`.
    fn new(sheet: HudSheet) -> Self {
        let mut spr = [Sprite::default(); 16];
        for (i, tex) in [(SPR_WORD, 0), (SPR_PART, 0), (SPR_STREAK, 7)] {
            spr[i].init_tex(&sheet, tex);
            spr[i].center();
            spr[i].show(false);
        }
        spr[SPR_BAND].show(false);
        spr[SPR_BAND].init_plain([0x80; 4]);
        spr[SPR_BAND].center();
        spr[SPR_REPLAY].show(false);
        spr[SPR_REPLAY].init_tex(&sheet, 8);
        spr[SPR_REPLAY].center();
        Self {
            sheet,
            spr,
            kind: None,
            pos: [256, 224],
            rot: 0.0,
            band_rot: 0.0,
            trail: [0.0; 8],
            state: -1,
            streak: 0,
            ramp_a: Ramp::default(),
            ramp_b: Ramp::default(),
        }
    }

    /// `HudNotice_Show(id)` for the announcements ported so far.
    fn show(&mut self, kind: Notice) {
        self.spr[SPR_STREAK].show(false);
        for spr in &mut self.spr[SPR_PART..SPR_PART + 12] {
            spr.show(false);
        }
        self.spr[SPR_WORD].show(false);
        self.state = 0;
        self.rot = 0.0;
        self.pos = match kind {
            Notice::Trail | Notice::Banner => [256, 352],
            _ => [256, 224],
        };
        self.kind = Some(kind);
    }

    fn update(&mut self, libc: &mut LibcRandom) {
        match self.kind {
            Some(Notice::Ready) => self.update_ready(),
            Some(Notice::Fight) => self.update_fight(libc),
            Some(Notice::Ko) => self.update_ko(libc),
            Some(Notice::BandA) => self.update_band(64),
            Some(Notice::BandB) => self.update_band(0),
            Some(Notice::SpinA) => self.update_spin(0),
            Some(Notice::SpinB) => self.update_spin(64),
            Some(Notice::Trail) => self.update_trail(libc),
            Some(Notice::Banner) => self.update_banner(),
            None => {}
        }
    }

    /// The streak of the finishes (texture 7, three frames of 42 rows): sliding in from the left, shaking by
    /// up to 16 (libc `rand()`), sliding out to the right in the fading colour; hidden otherwise.
    fn streak(&mut self, stage: Option<u8>, value: f32, color: [u8; 3], libc: &mut LibcRandom) {
        let Some(stage) = stage else {
            self.spr[SPR_STREAK].show(false);
            return;
        };
        let frame = self.streak;
        let streak = &mut self.spr[SPR_STREAK];
        streak.show(true);
        streak.init_tex(&self.sheet, 7);
        streak.set_rect(0, 512, 0, 42);
        streak.set_uv(0, 512, frame * 42, frame * 42 + 42);
        streak.center();
        let width = f32::from(streak.uv[1] - streak.uv[0]);
        match stage {
            0 => streak.move_by((-((1.0 - value) * width)) as i32, 0),
            1 => streak.move_by(libc.int_range(-16, 16), 0),
            _ => streak.move_by(((1.0 - value) * width) as i32, 0),
        }
        if stage == 2 {
            let [r, g, b] = color.map(|c| (value * f32::from(c)) as u8);
            streak.set_color(r, g, b, 0x80);
        } else {
            streak.set_color(color[0], color[1], color[2], 0x80);
        }
        self.streak = (self.streak + 1) % 3;
    }

    /// `HudNotice_UpdateKo`: the word pops (scale 1 + 0.4 sin(pi v), v 0 -> 0.5 in 0.15 s and on to 1 in 0.4 s,
    /// alpha min(128, 256 v)) with three after-images of the last frames' scales (alpha at most 48, hidden once
    /// caught up); then 0.2 / 0.55 / 0.4 s while the streak crosses in red, 0.2 s, and the band closes.
    fn update_ko(&mut self, libc: &mut LibcRandom) {
        use crate::btl_tech_volley::mathf_sin;
        if self.state == 0 {
            let word = &mut self.spr[SPR_WORD];
            word.show(true);
            word.init_tex(&self.sheet, 2);
            word.center();
            let band = &mut self.spr[SPR_BAND];
            band.show(true);
            band.init_plain([0x80; 4]);
            band.set_rect(0, 512, 0, 448);
            band.center();
            self.trail[..4].fill(0.0);
            self.ramp_b.start(0.15, 0.0, 0.5);
            self.state = 1;
        }
        match self.state {
            1 | 2 => {
                let s = mathf_sin(self.ramp_b.value * 3.141_592_7);
                self.trail.copy_within(0..3, 1);
                self.trail[0] = s;
                let alpha = |max: f32, v: f32| (v * 256.0).clamp(0.0, max) as u8;
                let v = self.ramp_b.value;
                let word = &mut self.spr[SPR_WORD];
                word.show(true);
                word.init_tex(&self.sheet, 2);
                word.center();
                word.scale(s * 0.4 + 1.0, s * 0.4 + 1.0);
                word.set_color(0x80, 0x80, 0x80, alpha(128.0, v));
                for i in 0..3 {
                    let s = self.trail[i + 1];
                    let part = &mut self.spr[SPR_PART + 1 + i];
                    part.show(s != self.trail[0]);
                    part.init_tex(&self.sheet, 2);
                    part.center();
                    part.scale(s * 0.4 + 1.0, s * 0.4 + 1.0);
                    part.set_color(0x80, 0x80, 0x80, alpha(48.0, v));
                }
                let done = self.ramp_b.step();
                if self.state == 1 {
                    if done {
                        self.ramp_b.start(0.4, 0.5, 1.0);
                        self.state = 2;
                    }
                } else if done && self.spr[SPR_PART + 3].hidden {
                    self.ramp_b.start(0.2, 0.0, 1.0);
                    self.state = 3;
                }
            }
            3 => {
                if self.ramp_b.step() {
                    self.ramp_b.start(0.55, 0.0, 1.0);
                    self.state = 4;
                }
            }
            4 => {
                if self.ramp_b.step() {
                    self.ramp_b.start(0.4, 1.0, 0.0);
                    self.state = 5;
                }
            }
            5 => {
                if self.ramp_b.step() {
                    self.ramp_b.start(0.2, 1.0, 0.0);
                    self.state = 6;
                }
            }
            6 => {
                if self.ramp_b.step() {
                    self.state = 7;
                }
                let band = &mut self.spr[SPR_BAND];
                band.set_rect(0, 512, 0, (self.ramp_b.value * 64.0) as i32);
                band.center();
            }
            7 => {
                self.spr[SPR_WORD].show(false);
                self.spr[SPR_PART].show(false);
                self.spr[SPR_STREAK].show(false);
                return;
            }
            _ => {}
        }
        let stage = match self.state {
            3 => Some(0),
            4 => Some(1),
            5 => Some(2),
            _ => None,
        };
        self.streak(stage, self.ramp_b.value, [0xFF, 0x60, 0x60], libc);
    }

    /// `HudNotice_UpdateBandA` / `UpdateBandB` (texture 3 from row `v0`, 512 x 64): a 4-pixel line slides in from
    /// the left (0.24 s) and opens to 64 (0.24 s); after 0.25 s a copy of the word swells and fades over it (0.64
    /// s, cubic); the band closes to the line (0.24 s) and slides out to the right (0.24 s). The 0.6 s wait between
    /// starts the other ramp, so it takes no time.
    fn update_band(&mut self, v0: i32) {
        let set_word = |spr: &mut Sprite, sheet: &HudSheet| {
            spr.init_tex(sheet, 3);
            spr.set_rect(-256, 256, -32, 32);
            spr.set_uv(0, 512, v0, v0 + 64);
        };
        let open = |spr: &mut Sprite, v: f32| {
            spr.init_plain([0x80; 4]);
            spr.set_rect(0, 512, 0, (v * 60.0 + 4.0) as i32);
            spr.center();
        };
        if self.state == 0 {
            self.spr[SPR_WORD].show(true);
            set_word(&mut self.spr[SPR_WORD], &self.sheet);
            self.spr[SPR_PART].show(false);
            set_word(&mut self.spr[SPR_PART], &self.sheet);
            self.ramp_a.start(0.24, 0.0, 1.0);
            self.state = 1;
        }
        match self.state {
            1 => {
                let done = self.ramp_a.step();
                let v = 1.0 - self.ramp_a.value;
                let band = &mut self.spr[SPR_BAND];
                band.show(true);
                band.init_plain([0x80; 4]);
                band.set_rect(0, 512, 0, 4);
                band.center();
                band.move_by((v * -512.0) as i32, 0);
                if done {
                    self.ramp_a.start(0.24, 0.0, 1.0);
                    self.state = 2;
                }
            }
            2 => {
                let done = self.ramp_a.step();
                open(&mut self.spr[SPR_BAND], self.ramp_a.value);
                if done {
                    self.ramp_a.start(0.25, 0.0, 1.0);
                    self.state = 3;
                }
            }
            3 => {
                if self.ramp_a.step() {
                    self.ramp_a.start(0.64, 1.0, 0.0);
                    self.state = 4;
                }
            }
            4 => {
                let done = self.ramp_a.step();
                let t = self.ramp_a.value.powf(3.0);
                let copy = &mut self.spr[SPR_PART];
                copy.show(true);
                set_word(copy, &self.sheet);
                copy.set_color(0x80, 0x80, 0x80, (t * 128.0) as u8);
                copy.scale(1.0 - t + 1.0, 1.0 - t + 1.0);
                if done {
                    self.ramp_b.start(0.6, 1.0, 0.0);
                    self.state = 5;
                }
            }
            5 => {
                if self.ramp_a.step() {
                    self.ramp_a.start(0.24, 1.0, 0.0);
                    self.state = 6;
                }
            }
            6 => {
                let done = self.ramp_a.step();
                open(&mut self.spr[SPR_BAND], self.ramp_a.value);
                if done {
                    self.ramp_a.start(0.24, 0.0, 1.0);
                    self.state = 7;
                }
            }
            7 => {
                let done = self.ramp_a.step();
                let band = &mut self.spr[SPR_BAND];
                band.set_rect(0, 512, 0, 4);
                band.center();
                band.move_by((self.ramp_a.value * 512.0) as i32, 0);
                if done {
                    self.state = 8;
                }
            }
            8 => {
                for i in [SPR_WORD, SPR_BAND, SPR_PART] {
                    self.spr[i].show(false);
                }
            }
            _ => {}
        }
    }

    /// `HudNotice_UpdateSpinA` / `UpdateSpinB` (texture 4 from row `v0`): the word shrinks from double size and
    /// fades in while the node turns one and a half turns (0.83 s, cubic); it stands 1 s, then swells to double
    /// size and fades out (0.23 s).
    fn update_spin(&mut self, v0: i32) {
        let set_word = |spr: &mut Sprite, sheet: &HudSheet| {
            spr.init_tex(sheet, 4);
            spr.set_rect(-256, 256, -32, 32);
            spr.set_uv(0, 512, v0, v0 + 64);
        };
        let mut rot = 0.0;
        if self.state == 0 {
            self.spr[SPR_WORD].show(true);
            set_word(&mut self.spr[SPR_WORD], &self.sheet);
            self.ramp_a.start(0.83, 1.0, 0.0);
            self.state = 1;
        }
        match self.state {
            1 => {
                let done = self.ramp_a.step();
                let t = self.ramp_a.value.powf(3.0);
                rot = t * 1.5 * 6.283_185_3;
                while rot > 3.141_592_7 {
                    rot -= 6.283_185_3;
                }
                let word = &mut self.spr[SPR_WORD];
                set_word(word, &self.sheet);
                word.set_color(0x80, 0x80, 0x80, ((1.0 - t) * 128.0) as u8);
                word.scale(t + 1.0, t + 1.0);
                if done {
                    self.ramp_a.start(1.0, 0.0, 1.0);
                    rot = 0.0;
                    self.state = 2;
                }
            }
            2 => {
                if self.ramp_a.step() {
                    self.ramp_a.start(0.23, 1.0, 0.0);
                    self.state = 3;
                }
            }
            3 => {
                let done = self.ramp_a.step();
                let t = self.ramp_a.value.powf(3.0);
                let word = &mut self.spr[SPR_WORD];
                set_word(word, &self.sheet);
                word.set_color(0x80, 0x80, 0x80, (t * 128.0) as u8);
                word.scale(2.0 - t, 2.0 - t);
                if done {
                    word.rgba[3] = 0;
                    self.state = 4;
                }
            }
            4 => {
                self.spr[SPR_WORD].show(false);
                self.spr[SPR_STREAK].show(false);
                return;
            }
            _ => {}
        }
        self.rot = rot;
    }

    /// `HudNotice_UpdateTrail` (texture 5): eight copies of the word zoom in one frame behind each other (v 1 ->
    /// 0 in 0.93 s; scale 2 - (1 - v)^3, alpha 128 (1 - v)^3), each hidden once it has caught up; then the word
    /// itself, 0.35 / 0.8 / 0.35 s while the streak crosses, 0.3 s, and it swells by 0.3 and fades (0.3 s).
    fn update_trail(&mut self, libc: &mut LibcRandom) {
        if self.state == 0 {
            let word = &mut self.spr[SPR_WORD];
            word.show(false);
            word.init_tex(&self.sheet, 5);
            word.center();
            self.spr[SPR_STREAK].show(false);
            self.trail = [1.0; 8];
            for i in 0..8 {
                let part = &mut self.spr[SPR_PART + i];
                part.show(false);
                part.init_tex(&self.sheet, 5);
                part.center();
            }
            self.ramp_a.start(0.93, 1.0, 0.0);
            self.state = 1;
        }
        match self.state {
            1 => {
                self.trail.copy_within(0..7, 1);
                let done = self.ramp_a.step();
                self.trail[0] = self.ramp_a.value;
                for i in 0..8 {
                    let on = i == 0 || self.trail[0] != self.trail[i - 1];
                    let k = 1.0 - (1.0 - self.trail[i]).powf(3.0);
                    let part = &mut self.spr[SPR_PART + i];
                    part.show(on);
                    part.init_tex(&self.sheet, 5);
                    part.center();
                    part.scale(k + 1.0, k + 1.0);
                    part.set_color(0x80, 0x80, 0x80, ((1.0 - k) * 128.0) as u8);
                }
                if done && self.trail[0] == self.trail[7] {
                    self.ramp_a.start(0.35, 0.0, 1.0);
                    for part in &mut self.spr[SPR_PART..SPR_PART + 8] {
                        part.show(false);
                    }
                    self.spr[SPR_WORD].show(true);
                    self.state = 2;
                }
            }
            2 => {
                if self.ramp_a.step() {
                    self.ramp_a.start(0.8, 1.0, 0.0);
                    self.state = 3;
                }
            }
            3 => {
                if self.ramp_a.step() {
                    self.ramp_a.start(0.35, 1.0, 0.0);
                    self.state = 4;
                }
            }
            4 | 5 => {
                if self.ramp_a.step() {
                    self.ramp_a.start(0.3, 1.0, 0.0);
                    self.state += 1;
                }
            }
            6 => {
                let done = self.ramp_a.step();
                let word = &mut self.spr[SPR_WORD];
                word.show(true);
                word.init_tex(&self.sheet, 5);
                word.center();
                if done {
                    word.rgba[3] = 0;
                    self.state = 7;
                } else {
                    let t = self.ramp_a.value.powf(3.0);
                    word.rgba[3] = (t * 128.0) as u8;
                    let k = (1.0 - t) * 0.3 + 1.0;
                    word.scale(k, k);
                }
            }
            7 => {
                self.spr[SPR_WORD].show(false);
                self.spr[SPR_STREAK].show(false);
                for part in &mut self.spr[SPR_PART..SPR_PART + 8] {
                    part.show(false);
                }
                return;
            }
            _ => {}
        }
        let stage = match self.state {
            2 => Some(0),
            3 => Some(1),
            4 => Some(2),
            _ => None,
        };
        self.streak(stage, self.ramp_a.value, [0xC0, 0x70, 0x60], libc);
    }

    /// `HudNotice_UpdateBanner` (texture 6): drops in from 100 above while fading in (0.4 s, sine), bounces 10
    /// (0.36 s), holds 1.73 s, then fades and shrinks to half (0.4 s, cubic).
    fn update_banner(&mut self) {
        use crate::btl_tech_volley::mathf_sin;
        if self.state == 0 {
            let spr = &mut self.spr[SPR_WORD];
            spr.show(true);
            spr.init_tex(&self.sheet, 6);
            spr.center();
            self.spr[SPR_STREAK].show(false);
            self.ramp_a.start(0.4, 1.0, 0.0);
            self.state = 1;
        }
        match self.state {
            1 | 2 => {
                let done = self.ramp_a.step();
                let t = if self.state == 1 {
                    mathf_sin(self.ramp_a.value * 1.570_796_3)
                } else {
                    mathf_sin(self.ramp_a.value * 3.141_592_6) * 0.1
                };
                let spr = &mut self.spr[SPR_WORD];
                spr.init_tex(&self.sheet, 6);
                spr.center();
                spr.move_by(0, (t * -100.0) as i32);
                spr.set_color(0x80, 0x80, 0x80, ((1.0 - t) * 128.0) as u32 as u8);
                if done {
                    if self.state == 1 {
                        self.ramp_a.start(0.36, 0.0, 1.0);
                    } else {
                        self.ramp_a.start(1.73, 1.0, 0.0);
                    }
                    self.state += 1;
                }
            }
            3 => {
                if self.ramp_a.step() {
                    self.ramp_a.start(0.4, 1.0, 0.0);
                    self.state = 4;
                }
            }
            4 => {
                let done = self.ramp_a.step();
                let spr = &mut self.spr[SPR_WORD];
                if done {
                    spr.rgba[3] = 0;
                    self.state = 5;
                } else {
                    let t = self.ramp_a.value.powf(3.0);
                    spr.rgba[3] = (t * 128.0) as u32 as u8;
                    spr.scale(t * 0.5 + 0.5, t * 0.5 + 0.5);
                }
            }
            5 => {
                self.spr[SPR_WORD].show(false);
                self.spr[SPR_STREAK].show(false);
            }
            _ => {}
        }
    }

    /// Sprites 0 (word) and 1..12 (fragments) as `HudNotice_UpdateReady` / `UpdateFight` set them at the end.
    fn fragments(&mut self, tex: usize, value: f32, t: f32) {
        for (i, ofs) in PART_OFS.iter().enumerate() {
            let part = &mut self.spr[SPR_PART + i];
            part.show(true);
            part.init_tex(&self.sheet, tex);
            part.center();
            part.move_by(
                (f32::from(ofs[0]) * (1.0 - t)) as i32,
                (f32::from(ofs[1]) * (1.0 - t)) as i32,
            );
            let delay = if i < 6 { i } else { 11 - i } as f32 / 30.0;
            let a = (value - delay).clamp(0.0, 1.0).powf(3.0) * 64.0;
            part.set_color(0x80, 0x80, 0x80, a as u8);
        }
    }

    fn swell_word(&mut self, tex: usize, t: f32) {
        let k = (1.0 - t) * 0.5 + 1.0;
        let word = &mut self.spr[SPR_WORD];
        word.show(true);
        word.init_tex(&self.sheet, tex);
        word.center();
        word.scale(k, k);
        word.set_color(0x80, 0x80, 0x80, (t * 128.0) as u8);
    }

    /// `HudNotice_UpdateReady`: the band grows from 32 x 4 to 512 wide (0.2 s), then to 128 high while it turns
    /// half a turn backwards (0.36 s); after 1.1 s the word and the band swell and fade (0.43 s) while the
    /// fragments fly out.
    fn update_ready(&mut self) {
        if self.state == 0 {
            let word = &mut self.spr[SPR_WORD];
            word.show(true);
            word.init_tex(&self.sheet, 0);
            word.center();
            let band = &mut self.spr[SPR_BAND];
            band.show(true);
            band.set_rect(0, 32, 0, 4);
            band.center();
            self.band_rot = 0.0;
            self.ramp_a.start(0.2, 0.0, 1.0);
            self.state = 1;
        }
        match self.state {
            1 => {
                let done = self.ramp_a.step();
                let v = self.ramp_a.value * 480.0 + 32.0;
                let band = &mut self.spr[SPR_BAND];
                band.show(true);
                band.set_rect(0, v as i32, 0, 4);
                band.center();
                if done {
                    self.ramp_a.start(0.36, 0.0, 1.0);
                    self.state = 2;
                }
            }
            2 => {
                let done = self.ramp_a.step();
                let v = self.ramp_a.value * 124.0 + 4.0;
                let band = &mut self.spr[SPR_BAND];
                band.show(true);
                band.set_rect(0, 512, 0, v as i32);
                band.center();
                self.band_rot = -self.ramp_a.value * 3.141_592_7;
                if done {
                    self.ramp_b.start(1.1, 1.0, 0.0);
                    self.state = 3;
                }
            }
            3 => {
                if self.ramp_b.step() {
                    self.ramp_b.start(0.43, 1.0, 0.0);
                    self.state = 4;
                }
            }
            4 => {
                let done = self.ramp_b.step();
                let value = self.ramp_b.value;
                let t = value.powf(3.0);
                self.swell_word(0, t);
                let k = (1.0 - t) * 0.5 + 1.0;
                let band = &mut self.spr[SPR_BAND];
                band.show(true);
                band.init_plain([0x80; 4]);
                band.set_rect(0, 512, 0, 128);
                band.center();
                band.scale(k, k);
                self.fragments(0, value, t);
                if done {
                    self.state = 5;
                }
            }
            5 => {
                self.spr[SPR_WORD].show(false);
                self.spr[SPR_BAND].show(false);
                for part in &mut self.spr[SPR_PART..SPR_PART + 12] {
                    part.show(false);
                }
            }
            _ => {}
        }
    }

    /// `HudNotice_UpdateFight`: the word shrinks into place and fades in as the fragments fly in (0.43 s), stands
    /// for 0.4 + 0.45 + 0.2 + 0.2 s while a streak crosses it (sliding in, shaking, sliding out), then the band
    /// closes on it (0.2 s).
    fn update_fight(&mut self, libc: &mut LibcRandom) {
        if self.state == 0 {
            let word = &mut self.spr[SPR_WORD];
            word.show(true);
            word.init_tex(&self.sheet, 1);
            word.center();
            let band = &mut self.spr[SPR_BAND];
            band.show(true);
            band.init_plain([0x80; 4]);
            band.set_rect(0, 512, 0, 448);
            band.center();
            self.ramp_b.start(0.43, 0.0, 1.0);
            self.state = 1;
        }
        match self.state {
            1 => {
                let done = self.ramp_b.step();
                let value = self.ramp_b.value;
                let t = value.powf(3.0);
                self.swell_word(1, t);
                self.fragments(1, value, t);
                if done {
                    for part in &mut self.spr[SPR_PART..SPR_PART + 12] {
                        part.show(false);
                    }
                    self.spr[SPR_WORD].set_color(0x80, 0x80, 0x80, 0x80);
                    self.ramp_b.start(0.4, 0.0, 1.0);
                    self.state = 2;
                }
            }
            2 => {
                if self.ramp_b.step() {
                    self.ramp_b.start(0.45, 0.0, 1.0);
                    self.state = 3;
                }
            }
            3 => {
                if self.ramp_b.step() {
                    self.ramp_b.start(0.2, 1.0, 0.0);
                    self.state = 4;
                }
            }
            4 => {
                if self.ramp_b.step() {
                    self.ramp_b.start(0.2, 1.0, 0.0);
                    self.state = 5;
                }
            }
            5 => {
                let done = self.ramp_b.step();
                let band = &mut self.spr[SPR_BAND];
                band.set_rect(0, 512, 0, (self.ramp_b.value * 64.0) as i32);
                band.center();
                if done {
                    self.state = 6;
                }
            }
            6 => {
                self.spr[SPR_WORD].show(false);
                self.spr[SPR_STREAK].show(false);
                return;
            }
            _ => {}
        }
        if !(2..=4).contains(&self.state) {
            self.spr[SPR_STREAK].show(false);
            return;
        }
        // The streak: texture 7, three frames of 42 rows.
        let frame = self.streak;
        let value = self.ramp_b.value;
        let streak = &mut self.spr[SPR_STREAK];
        streak.show(true);
        streak.init_tex(&self.sheet, 7);
        streak.set_rect(0, 512, 0, 42);
        streak.set_uv(0, 512, frame * 42, frame * 42 + 42);
        streak.center();
        let width = f32::from(streak.uv[1] - streak.uv[0]);
        match self.state {
            2 => streak.move_by((-((1.0 - value) * width)) as i32, 0),
            3 => streak.move_by(libc.int_range(-16, 16), 0),
            _ => streak.move_by(((1.0 - value) * width) as i32, 0),
        }
        if self.state == 4 {
            streak.set_color(
                (value * 192.0) as u8,
                (value * 112.0) as u8,
                (value * 96.0) as u8,
                0x80,
            );
        } else {
            streak.set_color(0xC0, 0x70, 0x60, 0x80);
        }
        self.streak = (self.streak + 1) % 3;
    }

    fn prim(&self, spr: &Sprite, vertices: Vec<HudVertex>, blend: HudBlend) -> HudPrim {
        HudPrim {
            texture: usize::try_from(spr.tex)
                .ok()
                .map(|tex| self.sheet.base + tex),
            vertices: fan(&vertices),
            rgba: spr.rgba,
            blend,
        }
    }

    /// The announcements' draw callbacks: READY, FIGHT, K.O. and the bands draw the word through the band's
    /// alpha; the spins, the trail and the banner draw it plainly. Then the fragments, after-images or copy, and for
    /// FIGHT, K.O. and the trail the streak added in proportion to the word's alpha.
    fn draw(&self, out: &mut Vec<HudPrim>) {
        // Node 3 (256, 50): the replay mark, drawn plainly.
        let mark = &self.spr[SPR_REPLAY];
        if !mark.hidden {
            let at = Xf::translate(256.0, 50.0);
            out.push(self.prim(
                mark,
                sprite_polygon(mark, &at, &self.sheet),
                HudBlend::Alpha,
            ));
        }
        let Some(kind) = self.kind else {
            return;
        };
        let node =
            Xf::rotate(self.rot).then(&Xf::translate(self.pos[0] as f32, self.pos[1] as f32));
        let band_xf = match kind {
            Notice::Ready => Xf::rotate(self.band_rot).then(&node),
            _ => node,
        };
        let word = &self.spr[SPR_WORD];
        let band = &self.spr[SPR_BAND];
        let masked = matches!(
            kind,
            Notice::Ready | Notice::Fight | Notice::Ko | Notice::BandA | Notice::BandB
        );
        if !masked {
            if !word.hidden {
                out.push(self.prim(
                    word,
                    sprite_polygon(word, &node, &self.sheet),
                    HudBlend::Alpha,
                ));
            }
        } else if !word.hidden && !band.hidden {
            // The word through the band's alpha.
            let mask: Vec<[f32; 2]> = sprite_polygon(band, &band_xf, &self.sheet)
                .iter()
                .map(|v| v.pos)
                .collect();
            let poly = clip_to_quad(sprite_polygon(word, &node, &self.sheet), &mask);
            if poly.len() >= 3 {
                out.push(self.prim(word, poly, HudBlend::Alpha));
            }
        }
        for part in &self.spr[SPR_PART..SPR_PART + 12] {
            if !part.hidden {
                out.push(self.prim(
                    part,
                    sprite_polygon(part, &node, &self.sheet),
                    HudBlend::Alpha,
                ));
            }
        }
        // The word writes only its alpha, then the streak is added in proportion to it.
        let streak = &self.spr[SPR_STREAK];
        let streaked = matches!(kind, Notice::Fight | Notice::Ko | Notice::Trail);
        if streaked && !streak.hidden && !word.hidden && word.tex >= 0 {
            let w = sprite_polygon(word, &node, &self.sheet);
            let (span_x, span_y) = (w[1].pos[0] - w[0].pos[0], w[2].pos[1] - w[0].pos[1]);
            let mut poly = sprite_polygon(streak, &node, &self.sheet);
            if span_x != 0.0 && span_y != 0.0 {
                for v in &mut poly {
                    let fx = (v.pos[0] - w[0].pos[0]) / span_x;
                    let fy = (v.pos[1] - w[0].pos[1]) / span_y;
                    v.mask_uv = [
                        w[0].uv[0] + fx * (w[1].uv[0] - w[0].uv[0]),
                        w[0].uv[1] + fy * (w[2].uv[1] - w[0].uv[1]),
                    ];
                }
                let mut prim = self.prim(
                    streak,
                    poly,
                    HudBlend::AddMasked(self.sheet.base + word.tex as usize),
                );
                prim.rgba[3] = word.rgba[3];
                out.push(prim);
            }
        }
    }
}

/// `gHudCaption` (src/battle/hud_caption.c): sheet 4's texture 0 at the top (240, 19) and texture 1 at the bottom
/// left (0, 386), whose alpha pulses 64..128 once a second (not frozen by the pause).
#[derive(Debug)]
struct CaptionPart {
    sheet: HudSheet,
    spr: [Sprite; 3],
    pulse: Ramp,
}

impl CaptionPart {
    /// `HudCaption_Init`.
    fn new(sheet: HudSheet) -> Self {
        let mut spr = [Sprite::default(); 3];
        spr[1].init_tex(&sheet, 0);
        spr[1].move_by(240, 19);
        spr[2].init_tex(&sheet, 1);
        spr[2].move_by(0, 386);
        Self {
            sheet,
            spr,
            pulse: Ramp::default(),
        }
    }

    /// `HudCaption_Update` and `HudCaption_Draw` (the pulsing sprite first; it is left out during a replay).
    fn run(&mut self, out: &mut Vec<HudPrim>) {
        if self.pulse.step() {
            self.pulse.start(1.0, 0.0, 1.0);
        }
        let s = crate::btl_tech_volley::mathf_sin(self.pulse.value * 3.141_592_7 * 2.0);
        self.spr[2].set_color(0xFF, 0xFF, 0xFF, ((s + 1.0) * 0.5 * 64.0 + 64.0) as u8);
        let node = Xf::translate(0.0, 0.0);
        for spr in [&self.spr[2], &self.spr[1]] {
            out.push(HudPrim {
                texture: usize::try_from(spr.tex)
                    .ok()
                    .map(|tex| self.sheet.base + tex),
                vertices: fan(&sprite_polygon(spr, &node, &self.sheet)),
                rgba: spr.rgba,
                blend: HudBlend::Alpha,
            });
        }
    }
}

/// `gHud` with the parts ported so far.
pub(crate) struct Hud {
    flags: u32,
    team: crate::hud_team::TeamPart,
    /// `Battle_GetMode() != 1`: story mode draws no team panel.
    team_shown: bool,
    combo: crate::hud_combo::ComboPart,
    gauge: GaugePart,
    timer: TimerPart,
    notice: NoticePart,
    prompt: crate::hud_prompt::PromptPart,
    caption: CaptionPart,
    /// `Battle_GetMode() == 7` (the attract demo): only the caption is drawn.
    caption_only: bool,
    /// `Battle_IsSplitScreen`: side 1's prompts are drawn too.
    pub(crate) split: bool,
    /// `gBtlGameReplayActive` (latched at the round's reset) and `gHud->replayMode`.
    replay: bool,
    replay_mode: i32,
    textures: HudTextures,
    layer: GsLayer,
    announcer: i32,
}

/// What `Hud_Draw` drew: the gauges and timer as one premultiplied 512 x 448 layer (none when empty), then the
/// notice's sprites over it.
pub struct HudFrame {
    pub layer: Option<Vec<u8>>,
    pub prims: Vec<HudPrim>,
}

impl Hud {
    /// A side's face after its character changed (`BtlObj_BindTables`: object + 0x50 of the new model).
    pub(crate) fn set_face(&mut self, side: usize, face: Option<bt3_data::gs::RgbaTexture>) {
        self.textures.faces[side] = face;
    }

    /// A member's team-panel face after its character changed (`BattleMember.data` reloaded).
    pub(crate) fn set_member_face(
        &mut self,
        side: usize,
        member: usize,
        face: Option<bt3_data::gs::RgbaTexture>,
    ) {
        if let Some(f) = self.textures.members[side].get_mut(member) {
            *f = face;
        }
    }

    /// `Hud_Init` then `Hud_Reset` (everything hidden).
    pub(crate) fn new(
        sheets: &[HudSheet],
        textures: HudTextures,
        announcer: i32,
        mode: i32,
    ) -> Self {
        let sheet0 = sheets.first().cloned().unwrap_or_default();
        Self {
            flags: 0,
            team: crate::hud_team::TeamPart::new(&sheet0),
            team_shown: mode != 1,
            combo: crate::hud_combo::ComboPart::new(
                sheets.get(COMBO_SHEET).cloned().unwrap_or_default(),
            ),
            timer: TimerPart::new(&sheet0),
            gauge: GaugePart::new(sheet0),
            notice: NoticePart::new(sheets.get(NOTICE_SHEET).cloned().unwrap_or_default()),
            prompt: crate::hud_prompt::PromptPart::new(
                sheets.get(PROMPT_SHEET).cloned().unwrap_or_default(),
                sheets.get(ICON_SHEET).cloned().unwrap_or_default(),
                crate::hud_prompt::PromptDefs::default(),
            ),
            caption: CaptionPart::new(sheets.get(CAPTION_SHEET).cloned().unwrap_or_default()),
            caption_only: mode == 7,
            split: false,
            replay: false,
            replay_mode: 0,
            textures,
            layer: GsLayer::new(),
            announcer,
        }
    }

    /// `Hud_PreUpdate` for what is simulated so far: the clock, each side's gauges and, when not `paused`, its
    /// team panel and switch slide.
    pub(crate) fn pre_update(
        &mut self,
        time_left: i32,
        count: Option<i32>,
        sides: &[SideGauge; 2],
        teams: &[crate::hud_team::SideTeam; 2],
        combos: &[crate::hud_combo::SideCombo; 2],
        prompts: &[crate::hud_prompt::SidePrompt; 2],
        paused: bool,
    ) {
        // Mode 3 counts the opponents beaten in place of the time.
        match count {
            Some(n) => self.timer.set_count(n),
            None => self.timer.set_time(time_left),
        }
        self.prompt.paused = paused;
        for (side, p) in prompts.iter().enumerate() {
            self.prompt.set_side(side, p);
        }
        for (side, gauge) in sides.iter().enumerate() {
            self.gauge.set_side(side, gauge);
            let c = &combos[side];
            self.combo.set_side(side, c, paused);
            // The combo damage shakes the other side's health; powering up shakes the side's ki.
            if !paused && c.damage_new {
                let (count, amp) = match c.damage {
                    9999.. => (15, 3),
                    1000.. => (10, 2),
                    _ => (5, 2),
                };
                self.gauge.shake_hp(side ^ 1, count, amp);
            }
            if c.powered_up {
                self.gauge.shake_ki(side, 8, 2);
            }
        }
        if !paused {
            for (side, team) in teams.iter().enumerate() {
                self.team.set_side(side, team);
                self.gauge.set_switching(side, team.switching);
            }
        }
    }

    /// `BATTLE_FLAG_PAUSE` as the parts read it while they update.
    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.gauge.paused = paused;
        self.team.paused = paused;
        self.combo.paused = paused;
        self.timer.paused = paused;
        self.prompt.paused = paused;
    }

    /// `BtlGame_Reset`'s latch of a playing replay.
    pub(crate) fn set_replay(&mut self, on: bool) {
        self.replay = on;
    }

    pub(crate) fn is_replay(&self) -> bool {
        self.replay
    }

    /// `Hud_SetReplayMode`, and `Hud_PreUpdate`'s replay mark (shown in mode 1 while a replay plays).
    pub(crate) fn set_replay_mode(&mut self, mode: i32) {
        self.replay_mode = mode;
        self.notice.spr[SPR_REPLAY].show(self.replay && mode == 1);
    }

    /// The prompt part's icon definitions, read from the executable.
    pub(crate) fn set_prompt_defs(&mut self, defs: crate::hud_prompt::PromptDefs) {
        self.prompt.set_defs(defs);
    }

    /// `Hud_ShowAll`.
    pub(crate) fn show_all(&mut self, on: bool) {
        self.flags = if on { SHOW_ALL } else { 0 };
    }

    /// `HudNotice_Show`: announcement `id` and, for 0..5, the announcer's line (on streamed-SE player 0). Id 8
    /// (the tutorial goal's) plays line 2 or 3, by `Rand_Range(2)` (`goal_line`).
    pub(crate) fn show_notice(&mut self, id: u32, sounds: &mut Vec<TitleSound>) {
        self.show_notice_with(id, None, sounds);
    }

    /// `HudNotice_Show(8)` with its `Rand_Range(2)` draw.
    pub(crate) fn show_goal_notice(
        &mut self,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        let line = if random.range(2) != 0 { 2 } else { 3 };
        self.show_notice_with(8, Some(line), sounds);
    }

    fn show_notice_with(&mut self, id: u32, goal_line: Option<u32>, sounds: &mut Vec<TitleSound>) {
        let kind = match id {
            0 => Notice::Ready,
            1 => Notice::Fight,
            2 => Notice::Ko,
            3 => Notice::BandA,
            4 => Notice::SpinA,
            5 => Notice::SpinB,
            6 => Notice::Trail,
            7 => Notice::Banner,
            8 => Notice::BandB,
            _ => return,
        };
        self.notice.show(kind);
        let line = if id <= 5 { Some(id) } else { goal_line };
        if let Some(line) = line {
            let line = announcer_base() + self.announcer.max(0) as u32 * ANNOUNCER_LINES + line;
            sounds.push(TitleSound::StreamSe(line));
        }
    }

    /// `Hud_ShowGauges`.
    /// `Hud_SlideOut` (`out`) / `Hud_SlideIn`: the gauges, team panel, timer, prompts and combo counter off the
    /// screen or back over `seconds`.
    pub(crate) fn slide(&mut self, out: bool, seconds: f32) {
        self.gauge.slide_all(out, seconds);
        self.team.slide(out, seconds);
        self.timer.slide(out, seconds);
        self.prompt.slide(out, seconds);
        self.combo.slide(out, seconds);
    }

    /// `HudPrompt_ShowCue` / `HudPrompt_HideCue` with `HudPrompt_SetCueDim`.
    pub(crate) fn set_cue(&mut self, shown: Option<bool>) {
        match shown {
            Some(can_act) => {
                self.prompt.show_cue();
                self.prompt.set_cue_dim(can_act);
            }
            None => self.prompt.hide_cue(),
        }
    }

    /// `HudTimer_ShowMark`.
    pub(crate) fn show_timer_mark(&mut self) {
        self.timer.mark = true;
    }

    pub(crate) fn show_gauges(&mut self, on: bool) {
        if on {
            self.flags |= SHOW_GAUGES;
        } else {
            self.flags &= !SHOW_GAUGES;
        }
    }

    #[cfg(test)]
    pub(crate) fn combo(&self) -> &crate::hud_combo::ComboPart {
        &self.combo
    }

    /// `Hud_ShowPrompt`.
    pub(crate) fn show_prompt(&mut self, on: bool) {
        if on {
            self.flags |= SHOW_PROMPT;
        } else {
            self.flags &= !SHOW_PROMPT;
        }
    }

    /// `Hud_ShowNotice`: the notice part (not `HudNotice_Show`, which announces).
    pub(crate) fn show_notice_part(&mut self, on: bool) {
        if on {
            self.flags |= SHOW_NOTICE;
        } else {
            self.flags &= !SHOW_NOTICE;
        }
    }

    /// `Hud_ShowCombo`.
    pub(crate) fn show_combo(&mut self, on: bool) {
        if on {
            self.flags |= SHOW_COMBO;
        } else {
            self.flags &= !SHOW_COMBO;
        }
    }

    /// `HudCombo_DrawText` with the sides' skill scripts (after `draw`, while the combo part is shown).
    pub(crate) fn draw_combo_text(
        &self,
        font: &mut crate::font::Font,
        scripts: [Option<&[u16]>; 2],
    ) {
        if self.flags & SHOW_COMBO != 0 {
            self.combo.draw_text(font, scripts);
        }
    }

    /// `Hud_ShowTimer`.
    pub(crate) fn show_timer(&mut self, on: bool) {
        if on {
            self.flags |= SHOW_TIMER;
        } else {
            self.flags &= !SHOW_TIMER;
        }
    }

    /// `Hud_Draw`: each part that is switched on updates, then draws: the team panels (outside story mode) and the
    /// gauges of both sides, the clock, the combo part, the notice. A frame that is not `shown` runs the
    /// updates and draws nothing.
    pub(crate) fn draw(&mut self, libc: &mut LibcRandom, shown: bool) -> HudFrame {
        if self.caption_only {
            let mut prims = Vec::new();
            self.caption.run(&mut prims);
            return HudFrame { layer: None, prims };
        }
        let mut layer = None;
        if self.flags & (SHOW_GAUGES | SHOW_TIMER) != 0 {
            draw_gauges(
                &mut self.layer,
                shown,
                (self.flags & SHOW_GAUGES != 0 && self.team_shown).then_some(&mut self.team),
                (self.flags & SHOW_GAUGES != 0).then_some(&mut self.gauge),
                (self.flags & SHOW_TIMER != 0).then_some(&mut self.timer),
                &self.textures,
                libc,
            );
            layer = (!self.layer.is_empty()).then(|| self.layer.to_rgba());
        }
        let mut prims = Vec::new();
        // The prompts of side 0, and side 1's in split screen.
        if self.flags & SHOW_PROMPT != 0 {
            self.prompt.run(0, &mut prims);
            if self.split {
                self.prompt.run(1, &mut prims);
            }
        }
        if self.flags & SHOW_COMBO != 0 {
            for side in 0..2 {
                self.combo.run(side, libc, &mut prims);
            }
        }
        if self.flags & SHOW_NOTICE != 0 {
            self.notice.update(libc);
            self.notice.draw(&mut prims);
        }
        // While a replay plays, mode 0 draws only the caption, 1 everything but it, 2 nothing (all parts update).
        if self.replay {
            let mut caption = Vec::new();
            self.caption.run(&mut caption);
            return match self.replay_mode {
                0 => HudFrame {
                    layer: None,
                    prims: caption,
                },
                1 => HudFrame { layer, prims },
                _ => HudFrame {
                    layer: None,
                    prims: Vec::new(),
                },
            };
        }
        HudFrame { layer, prims }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> HudSheet {
        HudSheet {
            base: 10,
            sizes: vec![
                [512, 64],
                [512, 64],
                [512, 64],
                [512, 128],
                [512, 128],
                [512, 64],
                [512, 64],
                [512, 128],
                [32, 32],
            ],
            views: Vec::new(),
        }
    }

    #[test]
    fn ready_reveals_the_word_through_the_turning_band() {
        let mut hud = Hud::new(
            &[HudSheet::default(), sheet()],
            HudTextures::default(),
            2,
            0,
        );
        let mut libc = LibcRandom::new(1);
        let mut sounds = Vec::new();
        assert!(hud.draw(&mut libc, true).prims.is_empty());
        hud.show_all(true);
        hud.show_notice(0, &mut sounds);
        assert_eq!(sounds, [TitleSound::StreamSe(0x10B6C + 14)]);
        // First frame: a 4-pixel line of the band, 512 x 64 word behind it.
        let prims = hud.draw(&mut libc, true).prims;
        assert_eq!(prims.len(), 1);
        let ys: Vec<f32> = prims[0].vertices.iter().map(|v| v.pos[1]).collect();
        assert!(ys.iter().all(|y| (222.0..=226.0).contains(y)), "{ys:?}");
        // 0.2 s + 0.36 s later the band is 128 high and has turned half a turn: the whole word shows.
        for _ in 0..17 {
            hud.draw(&mut libc, true);
        }
        let prims = hud.draw(&mut libc, true).prims;
        let xs: Vec<f32> = prims[0].vertices.iter().map(|v| v.pos[0]).collect();
        let ys: Vec<f32> = prims[0].vertices.iter().map(|v| v.pos[1]).collect();
        let span = |v: &[f32]| {
            v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!(span(&xs) >= 510.0 && span(&ys) >= 63.0, "{xs:?} {ys:?}");
        assert_eq!(prims[0].texture, Some(10));
        // Then 1.1 s of hold and 0.43 s of fading: the announcement is gone after about 2.1 s.
        for _ in 0..50 {
            hud.draw(&mut libc, true);
        }
        assert!(hud.draw(&mut libc, true).prims.is_empty());
    }

    #[test]
    fn fight_streak_is_weighted_by_the_word() {
        let mut hud = Hud::new(
            &[HudSheet::default(), sheet()],
            HudTextures::default(),
            0,
            0,
        );
        let mut libc = LibcRandom::new(1);
        let mut sounds = Vec::new();
        hud.show_all(true);
        hud.show_notice(1, &mut sounds);
        assert_eq!(sounds, [TitleSound::StreamSe(0x10B6C + 1)]);
        let mut streaks = 0;
        let mut frames = 0;
        while frames < 120 {
            let prims = hud.draw(&mut libc, true).prims;
            frames += 1;
            if prims.is_empty() {
                break;
            }
            if let Some(streak) = prims
                .iter()
                .find(|p| matches!(p.blend, HudBlend::AddMasked(11)))
            {
                streaks += 1;
                assert_eq!(streak.texture, Some(17));
                assert!(
                    streak
                        .vertices
                        .iter()
                        .all(|v| (0.0..=1.0).contains(&v.mask_uv[1]))
                );
            }
        }
        // 0.43 s in, then 0.4 + 0.45 + 0.2 s of streak (from the frame the fade-in ends), then the band closes:
        // at its last step it is 0 high and nothing shows.
        assert_eq!(streaks, 32);
        assert_eq!(frames, 51);
    }

    /// Runs announcement `id` until it draws nothing (at most 200 frames): every frame's sprites.
    fn run_notice(id: u32) -> (Vec<Vec<HudPrim>>, Vec<TitleSound>) {
        let mut hud = Hud::new(
            &[HudSheet::default(), sheet()],
            HudTextures::default(),
            0,
            0,
        );
        let mut libc = LibcRandom::new(1);
        let mut sounds = Vec::new();
        hud.show_all(true);
        hud.show_notice(id, &mut sounds);
        let mut frames = Vec::new();
        for _ in 0..200 {
            let prims = hud.draw(&mut libc, true).prims;
            if prims.is_empty() {
                break;
            }
            frames.push(prims);
        }
        (frames, sounds)
    }

    fn of_tex(prims: &[HudPrim], tex: usize) -> usize {
        prims.iter().filter(|p| p.texture == Some(10 + tex)).count()
    }

    #[test]
    fn ko_pops_with_after_images_and_a_red_streak() {
        let (frames, sounds) = run_notice(2);
        assert_eq!(sounds, [TitleSound::StreamSe(0x10B6C + 2)]);
        // The after-images trail the pop, then catch up.
        assert!(frames.iter().any(|f| of_tex(f, 2) == 4));
        let streaks: Vec<&HudPrim> = frames
            .iter()
            .filter_map(|f| {
                f.iter()
                    .find(|p| matches!(p.blend, HudBlend::AddMasked(12)))
            })
            .collect();
        // 0.2 + 0.55 + 0.4 s of streak, red while it comes in and shakes.
        assert_eq!(streaks.len(), 35);
        assert_eq!(streaks[0].rgba, [0xFF, 0x60, 0x60, 0x80]);
        assert!(frames.len() < 70, "{}", frames.len());
    }

    #[test]
    fn spins_turn_the_word_into_place() {
        for (id, tex) in [(4, 4), (5, 4)] {
            let (frames, sounds) = run_notice(id);
            assert_eq!(sounds, [TitleSound::StreamSe(0x10B6C + id)]);
            // Turning at first (the word's top edge is not level), level once in place.
            let tilt = |f: &Vec<HudPrim>| {
                let v = &f[0].vertices;
                (v[0].pos[1] - v[1].pos[1]).abs()
            };
            assert!(tilt(&frames[2]) > 1.0);
            assert_eq!(tilt(&frames[30]), 0.0);
            assert!(frames.iter().all(|f| of_tex(f, tex) == 1));
            // 25 + 30 + 7 frames (0.83, 1 and 0.23 s); its last frame has alpha 0.
            assert_eq!(frames.len(), 62);
        }
    }

    #[test]
    fn bands_and_banners_come_and_go() {
        let (frames, sounds) = run_notice(3);
        assert_eq!(sounds, [TitleSound::StreamSe(0x10B6C + 3)]);
        // The copy swells over the word at some point.
        assert!(frames.iter().any(|f| of_tex(f, 3) == 2));
        assert!((60..90).contains(&frames.len()), "{}", frames.len());
        let (frames, sounds) = run_notice(7);
        assert!(sounds.is_empty());
        // The banner drops in from above its place at y 352.
        let top = |f: &Vec<HudPrim>| {
            f[0].vertices
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::MAX, f32::min)
        };
        assert!(top(&frames[0]) < top(&frames[20]));
        assert!(frames.iter().all(|f| of_tex(f, 6) == 1));
        assert_eq!(frames.len(), 87);
    }

    #[test]
    fn trail_zooms_its_copies_in() {
        let (frames, sounds) = run_notice(6);
        assert!(sounds.is_empty());
        assert!(frames.iter().any(|f| of_tex(f, 5) >= 4));
        assert!(
            frames
                .iter()
                .any(|f| f.iter().any(|p| matches!(p.blend, HudBlend::AddMasked(15))))
        );
    }

    #[test]
    fn attract_demo_shows_only_the_pulsing_caption() {
        let caption = HudSheet {
            base: 40,
            sizes: vec![[32, 16], [128, 32]],
            views: Vec::new(),
        };
        let sheets = [
            HudSheet::default(),
            sheet(),
            HudSheet::default(),
            HudSheet::default(),
            caption,
        ];
        let mut hud = Hud::new(&sheets, HudTextures::default(), 0, 7);
        let mut libc = LibcRandom::new(1);
        hud.show_all(true);
        hud.show_notice(0, &mut Vec::new());
        let mut alphas = Vec::new();
        for _ in 0..31 {
            let prims = hud.draw(&mut libc, true).prims;
            assert_eq!(
                prims.iter().map(|p| p.texture).collect::<Vec<_>>(),
                [Some(41), Some(40)]
            );
            assert_eq!(prims[1].vertices[0].pos, [240.0, 19.0]);
            alphas.push(prims[0].rgba[3]);
        }
        // Once a second from the middle up towards 128, down towards 64 and back (sampled 30 times).
        assert_eq!(alphas[0], 96);
        assert!((126..=128).contains(alphas.iter().max().unwrap()));
        assert!((64..=66).contains(alphas.iter().min().unwrap()));
        assert_eq!(alphas[30], alphas[0]);
    }

    #[test]
    fn clipping_keeps_the_inside_of_either_winding() {
        let square =
            |x0: f32, y0: f32, x1: f32, y1: f32| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        let poly: Vec<HudVertex> = square(0.0, 0.0, 10.0, 10.0)
            .into_iter()
            .map(|pos| HudVertex {
                pos,
                uv: [pos[0] / 10.0, pos[1] / 10.0],
                mask_uv: [0.0; 2],
            })
            .collect();
        let mut band = square(-5.0, 4.0, 15.0, 6.0);
        let clipped = clip_to_quad(poly.clone(), &band);
        assert!(clipped.iter().all(|v| (4.0..=6.0).contains(&v.pos[1])));
        assert!(
            clipped
                .iter()
                .all(|v| (v.uv[1] * 10.0 - v.pos[1]).abs() < 1e-5)
        );
        band.reverse();
        assert_eq!(clip_to_quad(poly.clone(), &band).len(), clipped.len());
        assert!(clip_to_quad(poly, &square(0.0, 5.0, 10.0, 5.0)).is_empty());
    }
}
