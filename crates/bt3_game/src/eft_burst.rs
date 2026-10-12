//! The stage-change transition (`EftBurst_*`, src/battle/eft_burst.c): a model around the transition camera's
//! centre (0, 0, -50) whose groups of flag 0x800 get a second pass with the texture pack's map at eight times
//! their UVs and a light that spreads from the centre, and up to 350 particles on a fixed schedule (rings, flashes,
//! 240 streaks, sparks, debris, glows), with their sounds. Every random number is libc `rand()`.
//!
//! Drawn straight to the GS as the original does (no ordering table): the model writes its depth, the particles
//! test it. The model's clearing of the frame's alpha plane is not reproduced (nothing reads it).

use crate::battle::Mtx;
use crate::eft::{EffectBlend, EffectTri, EffectVertex, OFFSET_X, OFFSET_Y, V3};
use crate::rng::LibcRandom;
use crate::title::TitleSound;

const PTCL_MAX: usize = 350;

/// `EFT_BURST_F_*`.
const F_TIMED: u32 = 0x02;
const F_ADD: u32 = 0x08;
const F_SCALE_X: u32 = 0x10;
const F_SCALE_Y: u32 = 0x20;
const F_COLOR: u32 = 0x40;
const F_ROTATE: u32 = 0x80;
const F_SUB: u32 = 0x100;

/// The texture pack's entries the particles use.
const TEX_FLASH: usize = 0;
const TEX_RING: usize = 1;
const TEX_DEBRIS: usize = 3;
const TEX_GLOW: usize = 4;
const TEX_SPARK: usize = 5;
const TEX_ENV: usize = 6;

/// What a particle is: its update and draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    None,
    Spark,
    Streak,
    Flash,
    Ring,
    Glow,
    Debris,
}

/// `EftBurstPtcl`.
#[derive(Clone, Copy, Debug, Default)]
struct Ptcl {
    pos: V3,
    vel: V3,
    rot: V3,
    rot_vel: V3,
    life: i32,
    life_max: i32,
    tex: usize,
    scale: f32,
    flags: u32,
    kind: Kind,
    color: u32,
    alpha: i32,
    visible: bool,
    size: f32,
}

/// A model group as drawn: its texture (an effect texture), positions already scaled, triangles.
#[derive(Clone, Debug, Default)]
pub(crate) struct Group {
    pub env: bool,
    pub texture: Option<usize>,
    pub positions: Vec<V3>,
    pub tris: Vec<bt3_data::burst::BurstTri>,
}

/// `gEftBurstRes`: the texture pack's textures (effect texture indices) and the model.
#[derive(Clone, Debug, Default)]
pub(crate) struct BurstRes {
    pub pack: Vec<Option<usize>>,
    pub groups: Vec<Group>,
}

impl BurstRes {
    /// `EftBurst_Start`'s resources from the transition pack: entry 3's textures and entry 2's model with its
    /// own, appended to the effect textures. A texture that does not decode is drawn untextured.
    pub(crate) fn load(pack: &[u8], textures: &mut Vec<bt3_data::gs::RgbaTexture>) -> Self {
        let mut push = |t: std::io::Result<bt3_data::gs::RgbaTexture>| {
            t.ok().map(|t| {
                textures.push(t);
                textures.len() - 1
            })
        };
        let pack_tex = bt3_data::pack_section(pack, 3)
            .ok()
            .and_then(|s| bt3_data::tex::TextureResource::parse(s).ok())
            .map(|r| {
                r.entries
                    .iter()
                    .map(|e| push(bt3_data::gs::decode_sprite_texture(e)))
                    .collect()
            })
            .unwrap_or_default();
        let model = bt3_data::pack_section(pack, 2)
            .ok()
            .and_then(|s| bt3_data::burst::BurstModel::parse(s).ok());
        let mut groups = Vec::new();
        if let Some(m) = model {
            // The model's textures take their alpha from the texture (TCC 1); the decoder reads the indices alike.
            let tex: Vec<Option<usize>> = m
                .textures
                .iter()
                .map(|e| {
                    let e = bt3_data::tex::TextureEntry {
                        texture_register: e.texture_register & !(1 << 34),
                        ..*e
                    };
                    push(bt3_data::gs::decode_sprite_texture(&e))
                })
                .collect();
            for g in m.groups {
                let k = g.scale / 4096.0;
                groups.push(Group {
                    env: g.flags & 0x800 != 0,
                    texture: tex.get(g.texture).copied().flatten(),
                    positions: g
                        .positions
                        .iter()
                        .map(|p| p.map(|v| f32::from(v) * k))
                        .collect(),
                    tris: g.tris,
                });
            }
        }
        Self {
            pack: pack_tex,
            groups,
        }
    }
}

/// `gEftBurst`: the transition task while it runs.
#[derive(Clone, Debug)]
pub(crate) struct Burst {
    pub frame: i32,
    spawn_wait: i32,
    fade_dist: f32,
    unk_c: f32,
    sway: f32,
    phase: i32,
    se_wait: i32,
    ptcl: Vec<Ptcl>,
    /// `EftBurst_End` was asked for.
    ended: bool,
}

impl Burst {
    /// `EftBurst_Init`.
    pub(crate) fn new() -> Self {
        Self {
            frame: 0,
            spawn_wait: 0,
            fade_dist: 8.0,
            unk_c: 20.0,
            sway: 0.0,
            phase: 0,
            se_wait: 60,
            ptcl: vec![Ptcl::default(); PTCL_MAX],
            ended: false,
        }
    }

    /// `EftBurst_IsBusy`: the first 150 frames.
    pub(crate) fn is_busy(&self) -> bool {
        self.frame < 150
    }

    /// `EftBurst_End`: drawn no more.
    #[cfg(test)]
    pub(crate) fn end(&mut self) {
        self.ended = true;
    }

    /// The particles alive.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.ptcl.iter().filter(|p| p.life != 0).count()
    }

    /// `EftBurst_AllocPtcl`: the first slot whose life is 0.
    fn alloc(&mut self) -> Option<usize> {
        self.ptcl.iter().position(|p| p.life == 0)
    }

    /// `EftBurst_Update` (not while paused): the schedule of new particles, the frame, the fade distance, the
    /// sway, every particle, and the sounds (common 0x4A at frames 7 and 30, then every 60..139 frames; 0x47 with
    /// a 20 % chance every 60th frame), pitched down.
    pub(crate) fn update(
        &mut self,
        libc: &mut LibcRandom,
        streak_angle: &mut f32,
        snd: &mut crate::btl_snd::CharSound,
        out: &mut Vec<TitleSound>,
    ) {
        if self.frame == 0 {
            for (size, alpha) in [(70.0, 0x20), (50.0, 0x40)] {
                if let Some(i) = self.alloc() {
                    let p = &mut self.ptcl[i];
                    init_ring(p);
                    p.flags |= F_SCALE_X;
                    p.size = size;
                    p.alpha = alpha;
                }
            }
        }
        if self.frame == 7
            && let Some(i) = self.alloc()
        {
            init_flash(&mut self.ptcl[i]);
            self.ptcl[i].alpha = 0x45;
        }
        if self.frame == 10 {
            for _ in 0..240 {
                if let Some(i) = self.alloc() {
                    let frame = self.frame;
                    init_streak(&mut self.ptcl[i], libc, streak_angle, frame);
                }
            }
        }
        if self.frame == 30
            && let Some(i) = self.alloc()
        {
            init_flash(&mut self.ptcl[i]);
            self.ptcl[i].alpha = 0x20;
        }
        if self.frame >= 9 {
            if self.spawn_wait != 0 {
                self.spawn_wait -= 1;
            } else {
                if let Some(i) = self.alloc() {
                    init_glow(&mut self.ptcl[i], libc);
                }
                self.spawn_wait = (libc.rand() & 3) + 5;
            }
        }
        if self.frame == 10 {
            for idx in 0..32 {
                if let Some(i) = self.alloc() {
                    let sway = self.sway;
                    init_spark(&mut self.ptcl[i], idx, libc, sway);
                }
            }
            for _ in 0..16 {
                if let Some(i) = self.alloc() {
                    init_debris(&mut self.ptcl[i], libc);
                }
            }
        }
        if self.frame >= 11 {
            if let Some(i) = self.alloc() {
                let sway = self.sway;
                init_spark(&mut self.ptcl[i], 0, libc, sway);
            }
            if let Some(i) = self.alloc() {
                init_debris(&mut self.ptcl[i], libc);
            }
        }
        if self.frame < 151 {
            self.frame += 1;
        }
        if self.fade_dist < 50.0 {
            self.fade_dist += 0.4;
        }
        self.unk_c *= 0.995;
        self.phase += (libc.rand() & 7) + 3;
        if self.phase > 360 {
            self.phase -= 360;
        }
        let deg = |a: i32| (a as f32 * std::f32::consts::PI / 180.0).sin();
        let ph = self.phase;
        self.sway = (deg(ph)
            + deg(ph * 2 % 360)
            + deg(ph * 3 % 360)
            + deg(ph * 4 % 360)
            + deg(ph * 5 % 360)
            + deg(ph * 6 % 360))
            / 6.0;
        let frame = self.frame;
        for p in &mut self.ptcl {
            p.visible = p.kind != Kind::None && update_ptcl(p, libc, streak_angle, frame);
        }
        let mut play = |id: i32, pitch: i32, out: &mut Vec<TitleSound>| {
            snd.play_se(0, id, 0x7F, 0x40, pitch, out);
        };
        if self.frame == 7 || self.frame == 30 {
            play(0x4A, -0x352, out);
        }
        if self.frame >= 31 {
            if self.frame % 60 == 0 && libc.rand() % 100 < 20 {
                play(0x47, -500, out);
            }
            self.se_wait -= 1;
            if self.se_wait <= 0 {
                play(0x4A, -0x352, out);
                self.se_wait = libc.rand() % 80 + 60;
            }
        }
    }

    /// `EftBurst_Draw` (until it is ended): the model, then every particle that was visible in the last update.
    pub(crate) fn draw(
        &self,
        res: &BurstRes,
        world2view: &Mtx,
        world2screen: &Mtx,
    ) -> Vec<EffectTri> {
        let mut out = Vec::new();
        if self.ended {
            return out;
        }
        self.draw_model(res, world2screen, &mut out);
        for p in self.ptcl.iter().filter(|p| p.visible) {
            let tex = res.pack.get(p.tex).copied().flatten();
            match p.kind {
                Kind::Glow => draw_quad(p, tex, world2screen, &mut out),
                Kind::Streak | Kind::Flash | Kind::Ring => {
                    draw_quad_rot(p, tex, world2screen, &mut out)
                }
                Kind::Spark | Kind::Debris => {
                    draw_sprite(p, tex, world2view, world2screen, &mut out)
                }
                Kind::None => {}
            }
        }
        out
    }

    /// `EftBurst_DrawModel`: each triangle with its colours and UVs; a group of flag 0x800 also through context 2
    /// with the pack's map at 8 times the UVs, lit from frame 11 by a light that fades with the distance from the
    /// centre (out to `fadeDist`) and grows over the first 60 frames (the first pass's colours are lit too inside
    /// the distance). A triangle is sent when a corner is inside the guard area.
    fn draw_model(&self, res: &BurstRes, w2s: &Mtx, out: &mut Vec<EffectTri>) {
        let env_tex = res.pack.get(TEX_ENV).copied().flatten();
        let fade = (self.frame as f32 * (1.0 / 60.0)).min(1.0);
        let dist = self.fade_dist;
        let origin = [0.0, 0.0, -50.0];
        for g in &res.groups {
            for tri in &g.tris {
                let mut vis = false;
                let mut pass1 = [EffectVertex::default(); 3];
                let mut pass2 = [EffectVertex::default(); 3];
                for k in 0..3 {
                    let pos = g
                        .positions
                        .get(tri.vertices[k])
                        .copied()
                        .unwrap_or_default();
                    let (scr, q) = project(w2s, pos);
                    let uv = tri.uv[k].map(|v| f32::from(v) / 4096.0);
                    let st = [uv[0] * q, uv[1] * q, q];
                    let mut rgba = tri.rgba[k];
                    let mut env_rgba = [0, 0, 0, 0];
                    if g.env && self.frame >= 11 {
                        let d = [origin[0] - pos[0], origin[1] - pos[1], origin[2] - pos[2]];
                        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                        let lit = (255.0 - len / dist * 255.0).min(255.0);
                        let col = rgba.map(f32::from);
                        let a = (fade * 0.6 * lit).clamp(0.0, 255.0);
                        env_rgba = [127, 127, 127, a as i32 as u8];
                        if len < dist {
                            let light = [1.0, -0.2, -0.8, 0.0];
                            rgba = std::array::from_fn(|i| {
                                (light[i] * lit + col[i]).clamp(0.0, 255.0) as i32 as u8
                            });
                        }
                    }
                    vis |= in_guard(scr);
                    pass1[k] = vertex(scr, st, rgba);
                    pass2[k] = vertex(scr, [st[0] * 8.0, st[1] * 8.0, q], env_rgba);
                }
                if !vis {
                    continue;
                }
                out.push(tri_of(g.texture, EffectBlend::Alpha, true, pass1));
                if g.env {
                    out.push(tri_of(env_tex, EffectBlend::Alpha, true, pass2));
                }
            }
        }
    }
}

/// `Mtx_ProjectPoint` with the view's world-to-screen matrix: GS x, y in 12.4 fixed point and z; and Q.
fn project(m: &Mtx, p: V3) -> ([i32; 3], f32) {
    let v = crate::battle::apply(m, [p[0], p[1], p[2], 1.0]);
    let q = 1.0 / v[3];
    (
        [
            (v[0] * q * 16.0) as i32,
            (v[1] * q * 16.0) as i32,
            (v[2] * q) as i32,
        ],
        q,
    )
}

/// `Eft_IsScreenPosVisible`: in front of the camera and inside the guard area.
fn in_guard(s: [i32; 3]) -> bool {
    s[2] > 0 && (0x6801..=0x97FF).contains(&s[0]) && (0x6B01..=0x94FF).contains(&s[1])
}

fn vertex(s: [i32; 3], stq: [f32; 3], rgba: [u8; 4]) -> EffectVertex {
    EffectVertex {
        position: [
            s[0] as f32 / 16.0 - OFFSET_X,
            s[1] as f32 / 16.0 - OFFSET_Y,
            s[2].clamp(0, 0xFF_FFFF) as f32,
        ],
        stq,
        rgba,
    }
}

fn tri_of(
    texture: Option<usize>,
    blend: EffectBlend,
    zwrite: bool,
    v: [EffectVertex; 3],
) -> EffectTri {
    EffectTri {
        texture,
        blend,
        clamp: false,
        zwrite,
        alpha_ref: 0,
        vertices: v,
    }
}

/// A strip of four corners as two triangles.
fn strip(
    texture: Option<usize>,
    blend: EffectBlend,
    v: [EffectVertex; 4],
    out: &mut Vec<EffectTri>,
) {
    out.push(tri_of(texture, blend, false, [v[0], v[1], v[2]]));
    out.push(tri_of(texture, blend, false, [v[1], v[2], v[3]]));
}

const CORNERS: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
const ST: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];

/// `EftBurst_DrawQuad` (the glow): a quad of the particle's size, additive, its alpha rising to 16 at half life
/// and falling back.
fn draw_quad(p: &Ptcl, tex: Option<usize>, w2s: &Mtx, out: &mut Vec<EffectTri>) {
    let (life, max) = (p.life, p.life_max);
    let a = if max < life * 2 {
        (max - life) * 16 / max
    } else {
        life * 16 / max
    };
    let mut v = [EffectVertex::default(); 4];
    let mut vis = false;
    for i in 0..4 {
        let c = CORNERS[i];
        let (s, _) = project(
            w2s,
            [
                c[0] * p.scale + p.pos[0],
                c[1] * p.scale + p.pos[1],
                p.pos[2],
            ],
        );
        vis |= s[2] > 0;
        v[i] = vertex(s, [ST[i][0], ST[i][1], 1.0], [0x80, 0x80, 0x80, a as u8]);
    }
    if vis {
        strip(tex, EffectBlend::Add, v, out);
    }
}

/// Rotation matrices of `Mtx_RotateY` / `Mtx_RotateZ` (premultiplying).
fn rot_y(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn rot_z(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// `EftBurst_DrawQuadRot`: a quad scaled per axis by flags 0x10 / 0x20, turned (flag 0x80) by Z(rot.z) Y(rot.y)
/// Y(rot.x), additive (normal with flag 0x08), in its colour (flag 0x40) or blue-grey, fading with its life.
fn draw_quad_rot(p: &Ptcl, tex: Option<usize>, w2s: &Mtx, out: &mut Vec<EffectTri>) {
    let m = (p.flags & F_ROTATE != 0).then(|| {
        let m = crate::eft::mtx_mul(&rot_z(p.rot[2]), &rot_y(p.rot[1]));
        crate::eft::mtx_mul(&m, &rot_y(p.rot[0]))
    });
    let a = (p.alpha * p.life / p.life_max.max(1)) as u8;
    let rgb = if p.flags & F_COLOR != 0 {
        [p.color as u8, (p.color >> 8) as u8, (p.color >> 16) as u8]
    } else {
        [0x50, 0x50, 0x80]
    };
    let mut v = [EffectVertex::default(); 4];
    let mut vis = false;
    for i in 0..4 {
        let mut c = [CORNERS[i][0], CORNERS[i][1], 0.0, 0.0];
        if p.flags & F_SCALE_X != 0 {
            c[0] *= p.scale;
        }
        if p.flags & F_SCALE_Y != 0 {
            c[1] *= p.scale;
        }
        if let Some(m) = &m {
            c = crate::battle::apply(m, c);
        }
        let (s, _) = project(w2s, [c[0] + p.pos[0], c[1] + p.pos[1], c[2] + p.pos[2]]);
        vis |= s[2] > 0;
        v[i] = vertex(s, [ST[i][0], ST[i][1], 1.0], [rgb[0], rgb[1], rgb[2], a]);
    }
    if vis {
        let blend = if p.flags & F_ADD != 0 {
            EffectBlend::Alpha
        } else {
            EffectBlend::Add
        };
        strip(tex, blend, v, out);
    }
}

/// `EftBurst_DrawSprite`: a sprite facing the camera (its corners turned by the view's rotation), additive
/// (normal with flag 0x08, subtractive with 0x100), in its colour and alpha (flag 0x40) or grey fading with its
/// life.
fn draw_sprite(p: &Ptcl, tex: Option<usize>, w2v: &Mtx, w2s: &Mtx, out: &mut Vec<EffectTri>) {
    let mut t: Mtx = [[0.0; 4]; 4];
    for (i, row) in t.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = w2v[j][i];
        }
    }
    let mut s = [[0; 3]; 2];
    let mut vis = false;
    for (i, c) in [[-1.0, -1.0], [1.0, 1.0]].into_iter().enumerate() {
        let d = crate::battle::apply(&t, [c[0], c[1], 0.0, 0.0]);
        let (scr, _) = project(
            w2s,
            [
                d[0] * p.scale + p.pos[0],
                d[1] * p.scale + p.pos[1],
                d[2] * p.scale + p.pos[2],
            ],
        );
        vis |= scr[2] > 0;
        s[i] = scr;
    }
    if !vis {
        return;
    }
    let rgba = if p.flags & F_COLOR != 0 {
        [
            p.color as u8,
            (p.color >> 8) as u8,
            (p.color >> 16) as u8,
            p.alpha as u8,
        ]
    } else {
        [0x50, 0x50, 0x50, (p.life * 80 / p.life_max.max(1)) as u8]
    };
    // A sprite takes the second corner's Z.
    let z = s[1][2];
    let corner = |x: i32, y: i32, st: [f32; 2]| vertex([x, y, z], [st[0], st[1], 1.0], rgba);
    let v = [
        corner(s[0][0], s[0][1], [0.0, 0.0]),
        corner(s[1][0], s[0][1], [1.0, 0.0]),
        corner(s[0][0], s[1][1], [0.0, 1.0]),
        corner(s[1][0], s[1][1], [1.0, 1.0]),
    ];
    let blend = if p.flags & F_ADD != 0 {
        EffectBlend::Alpha
    } else if p.flags & F_SUB != 0 {
        EffectBlend::Subtract
    } else {
        EffectBlend::Add
    };
    strip(tex, blend, v, out);
}

/// libc `rand()` as the original's `rand() & mask` and `% n` take it.
fn r(libc: &mut LibcRandom) -> i32 {
    libc.rand()
}

fn normalize(v: V3) -> V3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        v
    }
}

/// `EftBurst_InitSpark`: on the axis, flying at the camera in a narrow cone.
fn init_spark(p: &mut Ptcl, idx: i32, libc: &mut LibcRandom, sway: f32) {
    let fi = idx as f32;
    p.pos = [0.0, 0.0, -40.0 - fi];
    let x = ((r(libc) & 0xFF) - 0x7F) as f32 * 0.1 * (1.0 / 128.0);
    let y = ((r(libc) & 0xFF) - 0x7F) as f32 * 0.1 * (1.0 / 128.0);
    let z = -((r(libc) & 0x7F) as f32) * (1.0 / 128.0) - 1.0;
    let mut v = normalize([x, y, z]);
    v[2] -= 1.0;
    p.vel = v;
    p.tex = TEX_SPARK;
    p.scale = 17.0 - fi * 0.4 + sway;
    p.flags = F_COLOR | F_TIMED;
    p.life = 0x19 - idx;
    p.life_max = p.life;
    p.color = 0x80_8080;
    p.alpha = 0x10;
    if p.life <= 0 {
        p.life = 1;
    }
    p.kind = Kind::Spark;
}

/// `EftBurst_InitStreak`: a debris particle made a turned quad on a circle round the axis, drifting back to it.
fn init_streak(p: &mut Ptcl, libc: &mut LibcRandom, angle: &mut f32, frame: i32) {
    init_debris(p, libc);
    p.flags = 0xF2;
    p.tex = TEX_DEBRIS;
    p.life = (r(libc) & 0xF) + 7;
    p.life_max = p.life;
    p.scale = (r(libc) & 0x1F) as f32 * (1.0 / 32.0) * 1.5 + 2.5;
    p.rot_vel = [0.0; 3];
    let t = (frame as f32 * 0.001_666_666_7).min(1.0);
    let ang = *angle * 3.14 / 180.0;
    let rad = (r(libc) & 0x1F) as f32 * (1.0 / 32.0) * (t + 2.0) + 10.0;
    p.pos = [ang.cos() * rad, ang.sin() * rad, -50.0];
    let mut v = [p.pos[0] * 0.04, p.pos[1] * 0.04, -50.0 * 0.04];
    v[2] = (r(libc) & 0x1F) as f32 / 15.0 * -0.6 + t * 0.2;
    p.vel = v;
    p.rot = [-5.0 * 3.14 / 180.0, 1.57, *angle * 3.14 / 180.0];
    if r(libc) & 1 != 0 {
        p.color = 0x30;
        p.alpha = 0x50;
        p.flags |= F_ADD;
    } else {
        p.color = 0x40;
        p.alpha = 0x50;
    }
    *angle += (r(libc) & 0x3F) as f32;
    if *angle > 180.0 {
        *angle -= 360.0;
    }
    p.kind = Kind::Streak;
}

/// `EftBurst_InitFlash`: a quad on the axis growing by 20 % a frame for ten frames.
fn init_flash(p: &mut Ptcl) {
    p.flags = 0x7A;
    p.tex = TEX_FLASH;
    p.life = 10;
    p.life_max = 10;
    p.scale = 10.0;
    p.pos = [0.0, 0.0, -55.0];
    p.vel = [0.0; 3];
    p.alpha = 0x80;
    p.color = 0x80_8080;
    p.kind = Kind::Flash;
}

/// `EftBurst_InitRing`: a quad that swells and shrinks over eight frames.
fn init_ring(p: &mut Ptcl) {
    p.flags = 0x4A;
    p.tex = TEX_RING;
    p.life = 8;
    p.life_max = 8;
    p.scale = 0.1;
    p.color = 0x70_8080;
    p.alpha = 0x40;
    p.size = 70.0;
    p.pos = [-5.0, 0.0, -51.0];
    p.vel = [0.0; 3];
    p.kind = Kind::Ring;
}

/// `EftBurst_InitGlow`: a quad on the axis growing slowly for 47..62 frames.
fn init_glow(p: &mut Ptcl, libc: &mut LibcRandom) {
    p.pos = [0.0, 0.0, -51.0];
    p.vel = [0.0; 3];
    p.tex = TEX_GLOW;
    p.flags = 0x01 | F_TIMED;
    p.life = (r(libc) & 0xF) + 0x2F;
    p.life_max = p.life;
    p.scale = 10.0;
    p.kind = Kind::Glow;
}

/// `EftBurst_InitDebris`: a sprite scattered round the axis, drifting away from it.
fn init_debris(p: &mut Ptcl, libc: &mut LibcRandom) {
    p.flags = 0x4B;
    p.scale = (r(libc) & 0x7F) as f32 * 2.0 * (1.0 / 128.0) + 4.0;
    p.pos[0] = ((r(libc) & 0xFF) - 0x7F) as f32 * 8.0 * (1.0 / 128.0);
    p.pos[1] = ((r(libc) & 0xFF) - 0x7F) as f32 * 8.0 * (1.0 / 128.0);
    p.pos[2] = -((r(libc) & 0x7F) as f32) * 2.0 * (1.0 / 128.0) - 51.0;
    let vx = ((r(libc) & 0xFF) - 0x7F) as f32 * 0.1 * (1.0 / 128.0);
    let vy = ((r(libc) & 0xFF) - 0x7F) as f32 * 0.1 * (1.0 / 128.0);
    let vz = -((r(libc) & 0x7F) as f32) * 0.2 * (1.0 / 128.0);
    let c = normalize([-p.pos[0], -p.pos[1], 0.0]);
    p.vel = [vx + c[0] * -0.1, vy + c[1] * -0.1, vz];
    p.life = (r(libc) & 3) + 0x1F;
    p.life_max = p.life;
    p.tex = TEX_DEBRIS;
    p.color = 0x60_6060;
    p.alpha = 0;
    p.kind = Kind::Debris;
}

/// The life count of a timed particle: true while it lasts.
fn tick(p: &mut Ptcl) -> bool {
    if p.flags & F_TIMED != 0 {
        if p.life != 0 {
            p.life -= 1;
        } else {
            return false;
        }
    }
    true
}

fn drift(p: &mut Ptcl, damp: f32) {
    for i in 0..3 {
        p.pos[i] += p.vel[i];
        p.vel[i] *= damp;
    }
}

/// The particle's update; false once it has ended.
fn update_ptcl(p: &mut Ptcl, libc: &mut LibcRandom, angle: &mut f32, frame: i32) -> bool {
    match p.kind {
        Kind::Flash => {
            drift(p, 30.0 / 31.0);
            p.scale = (p.scale + 0.1) * 1.2;
            tick(p)
        }
        Kind::Streak => {
            drift(p, 30.0 / 31.0);
            let d = normalize([-p.pos[0], -p.pos[1], -p.pos[2]]);
            for i in 0..3 {
                p.vel[i] += d[i] * (0.98 / 15.0);
            }
            for i in 0..3 {
                p.rot[i] += p.rot_vel[i];
            }
            p.rot_vel[0] += -0.5 * 3.14 / 180.0;
            for v in &mut p.rot {
                if *v > 3.14 {
                    *v = -3.14;
                }
                if *v < -3.14 {
                    *v = 3.14;
                }
            }
            if p.flags & F_TIMED != 0 {
                if p.life != 0 {
                    p.life -= 1;
                    if p.life == 0 {
                        init_streak(p, libc, angle, frame);
                    }
                } else {
                    return false;
                }
            }
            true
        }
        Kind::Spark => {
            drift(p, 0.98);
            p.scale -= 0.3;
            if p.life < 10 {
                p.alpha = p.life;
            }
            tick(p)
        }
        Kind::Ring => {
            drift(p, 30.0 / 31.0);
            let mut s = (p.life_max - p.life) as f32 / p.life_max as f32;
            if s > 0.5 {
                s = 1.0 - s;
            }
            p.scale = s * p.size + 10.0;
            tick(p)
        }
        Kind::Glow => {
            drift(p, 30.0 / 31.0);
            p.scale += 0.35;
            tick(p)
        }
        Kind::Debris => {
            drift(p, 0.98);
            p.alpha = p.life * 48 / p.life_max;
            if p.alpha >= 25 {
                p.alpha = 48 - p.alpha;
            }
            tick(p)
        }
        Kind::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schedule_spawns_rings_flashes_streaks_and_plays_the_sounds() {
        let mut b = Burst::new();
        let mut libc = LibcRandom::new(1);
        let mut angle = 0.0;
        let mut snd = crate::btl_snd::CharSound::default();
        let mut out = Vec::new();
        b.update(&mut libc, &mut angle, &mut snd, &mut out);
        // Frame 0: the two rings.
        assert_eq!(b.count(), 2);
        assert_eq!(b.frame, 1);
        for _ in 1..11 {
            b.update(&mut libc, &mut angle, &mut snd, &mut out);
        }
        // By frame 10: 240 streaks, 32 sparks (the last eight, of life 1 or less, already gone), 16 debris and a
        // glow from frame 9, the flash of frame 7; the rings of frame 0 are over.
        assert_eq!(b.count(), 240 + 24 + 16 + 1 + 1);
        while b.is_busy() {
            b.update(&mut libc, &mut angle, &mut snd, &mut out);
        }
        assert_eq!(b.frame, 150);
        // Streaks start over when their life ends, so the burst stays full.
        assert!(b.count() > 240, "{}", b.count());
        assert!(angle.abs() <= 180.0);
    }

    #[test]
    fn particles_draw_in_front_of_the_transition_camera() {
        let mut b = Burst::new();
        let mut libc = LibcRandom::new(1);
        let mut angle = 0.0;
        let mut snd = crate::btl_snd::CharSound::default();
        let mut out = Vec::new();
        for _ in 0..12 {
            b.update(&mut libc, &mut angle, &mut snd, &mut out);
        }
        // A camera at the origin looking down -z (yawed half a turn).
        let view = crate::battle::View::new([0.0; 3], [0.0, std::f32::consts::PI, 0.0]);
        let res = BurstRes {
            pack: (0..7).map(Some).collect(),
            groups: Vec::new(),
        };
        let tris = b.draw(&res, &view.world2view, &view.world2screen);
        assert!(!tris.is_empty());
        assert!(tris.iter().all(|t| t.texture.is_some() && !t.zwrite));
        b.end();
        assert!(
            b.draw(&res, &view.world2view, &view.world2screen)
                .is_empty()
        );
    }
}
