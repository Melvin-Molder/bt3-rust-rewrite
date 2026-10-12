use std::io;

use bt3_data::fod::{
    self, Action, CXFORM_IDENTITY, Cxform, Fod, Frame, FrameTag, IDENTITY, Matrix,
};

use crate::draw::ScreenQuad;

/// `FlashDraw_PutSprite`: the texture rectangle shrinks by one texel on each side unless flipped. `textures` maps a
/// movie image to its clamped texture; the repeating sampler is the next texture, and texture `n` of the image's
/// file (`uv.tex`) is `2 * n` further on.
pub fn screen_quad(quad: &DrawQuad, textures: &[Option<usize>]) -> ScreenQuad {
    let texture = quad
        .image
        .and_then(|image| textures.get(image).copied().flatten())
        .map(|index| {
            index + 2 * quad.uv.tex.max(0) as usize + usize::from(quad.flags & QUAD_REPEAT != 0)
        });
    let uv = quad.uv;
    let (mut u0, mut u1, mut v0, mut v1) = (uv.u0 + 1, uv.u1 - 1, uv.v0 + 1, uv.v1 - 1);
    if quad.flags & QUAD_FLIPX != 0 {
        (u0, u1) = (uv.u1, uv.u0);
    }
    if quad.flags & QUAD_FLIPY != 0 {
        (v0, v1) = (uv.v1, uv.v0);
    }
    let [u0, u1, v0, v1] = [u0, u1, v0, v1].map(|v| v as f32);
    ScreenQuad {
        texture,
        pt: quad.pt,
        uv: [[u0, v0], [u1, v0], [u0, v1], [u1, v1]],
        rgba: quad.rgba.map(|c| c.clamp(0, 255) as u8),
    }
}

/// `Flash_Draw`'s quads for the renderer. A `FLASH_QUAD_MASK_SET` quad only writes the frame buffer's alpha plane and
/// `FLASH_QUAD_MASK` quads are drawn through it; here a mask is the rectangle of the last mask writer, and masked
/// quads are cut to it (axis-aligned quads only).
pub fn screen_quads(quads: &[DrawQuad], textures: &[Option<usize>]) -> Vec<ScreenQuad> {
    let mut mask: Option<[f32; 4]> = None;
    let mut out = Vec::new();
    for quad in quads {
        if quad.flags & QUAD_MASK_SET != 0 {
            let xs = quad.pt.map(|p| p[0]);
            let ys = quad.pt.map(|p| p[1]);
            let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
            let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
            mask = Some([min(xs), min(ys), max(xs), max(ys)]);
            continue;
        }
        let mut screen = Some(screen_quad(quad, textures));
        if let Some([x0, y0, x1, y1]) = quad.scissor {
            let rect = [x0 as f32, y0 as f32, (x1 + 1) as f32, (y1 + 1) as f32];
            screen = screen.and_then(|s| clip_quad(s, rect));
        }
        let Some(screen) = screen else { continue };
        match (quad.flags & QUAD_MASK != 0, mask) {
            (true, Some(rect)) => out.extend(clip_quad(screen, rect)),
            (true, None) => {}
            (false, _) => out.push(screen),
        }
    }
    out
}

/// The part of an axis-aligned quad inside `[x0, y0, x1, y1]`, with its texture rectangle cut to match.
fn clip_quad(mut q: ScreenQuad, [x0, y0, x1, y1]: [f32; 4]) -> Option<ScreenQuad> {
    let [tl, tr, bl, _] = q.pt;
    if tl[1] != tr[1] || tl[0] != bl[0] {
        return Some(q);
    }
    let (qx0, qy0, qx1, qy1) = (tl[0], tl[1], tr[0], bl[1]);
    let (cx0, cy0, cx1, cy1) = (qx0.max(x0), qy0.max(y0), qx1.min(x1), qy1.min(y1));
    if cx0 >= cx1 || cy0 >= cy1 {
        return None;
    }
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let (tx0, tx1) = ((cx0 - qx0) / (qx1 - qx0), (cx1 - qx0) / (qx1 - qx0));
    let (ty0, ty1) = ((cy0 - qy0) / (qy1 - qy0), (cy1 - qy0) / (qy1 - qy0));
    let [[u0, v0], [u1, _], [_, v1], _] = q.uv;
    let (nu0, nu1) = (lerp(u0, u1, tx0), lerp(u0, u1, tx1));
    let (nv0, nv1) = (lerp(v0, v1, ty0), lerp(v0, v1, ty1));
    q.pt = [[cx0, cy0], [cx1, cy0], [cx0, cy1], [cx1, cy1]];
    q.uv = [[nu0, nv0], [nu1, nv0], [nu0, nv1], [nu1, nv1]];
    Some(q)
}

// FlashTl.state
const TL_PLAY: u8 = 0x01;
const TL_HOLD: u8 = 0x02;
const TL_GOTO: u8 = 0x04;
const TL_SEEK: u8 = 0x08;
const TL_KEEP: u8 = 0x10;

// Flash.flags
const FLASH_PLAY: u32 = 1;
pub const FLASH_PAD: u32 = 2;
const FLASH_HIDE: u32 = 4;
const FLASH_END: u32 = 8;

// FlashClip.flags
const CLIP_USED: u32 = 1;
const CLIP_VISIBLE: u32 = 2;
const CLIP_FREE_RUN: u32 = 8;

// FlashProp.flags (FLASH_OV_*)
const OV_TEX: u32 = 0x0001;
const OV_POS: u32 = 0x0002;
const OV_UV: u32 = 0x0004;
const OV_ALPHA: u32 = 0x0008;
const OV_SCALE: u32 = 0x0010;
const OV_COLOR: u32 = 0x0020;
const OV_FLIPX: u32 = 0x0040;
const OV_FLIPY: u32 = 0x0080;
const OV_REPEAT: u32 = 0x0100;
const OV_NO_OFS: u32 = 0x0200;
const OV_ADD: u32 = 0x0400;
const OV_SUB: u32 = 0x0800;
const OV_MASK: u32 = 0x1000;
const OV_MASK_SET: u32 = 0x2000;

// FlashQuad.flags
pub const QUAD_FLIPX: u32 = 0x01;
pub const QUAD_FLIPY: u32 = 0x02;
pub const QUAD_REPEAT: u32 = 0x04;
pub const QUAD_ADD: u32 = 0x08;
pub const QUAD_SUB: u32 = 0x10;
pub const QUAD_MASK: u32 = 0x20;
pub const QUAD_MASK_SET: u32 = 0x40;

/// Flash_ClipSetFlags `props` bits.
pub const PROP_PLAY: u32 = 0x001;
pub const PROP_VISIBLE: u32 = 0x002;
pub const PROP_MASK: u32 = 0x080;
pub const PROP_MASK_SET: u32 = 0x100;
pub const PROP_REPEAT: u32 = 0x200;

/// `FlashUv`: a texture rectangle in texels and a texture index within the image's entry table.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Uv {
    pub u0: i32,
    pub v0: i32,
    pub u1: i32,
    pub v1: i32,
    pub tex: i32,
}

/// One quad on its way to the GS (`FlashQuad`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawQuad {
    pub flags: u32,
    /// Top left, top right, bottom left, bottom right.
    pub pt: [[f32; 2]; 4],
    pub uv: Uv,
    pub rgba: [i32; 4],
    /// Record of tag 3, or `None` for a flat quad.
    pub image: Option<usize>,
    /// The scissor a clip's callbacks set around it (`Sprite_SetScissor`, inclusive bounds), `None` for the
    /// whole screen.
    pub scissor: Option<[i32; 4]>,
}

#[derive(Clone, Copy, Debug)]
struct Prop {
    flags: u32,
    x: f32,
    y: f32,
    alpha: f32,
    scale_x: f32,
    scale_y: f32,
    color: f32,
    uv: Uv,
    mtx: Matrix,
    cx: Cxform,
}

impl Prop {
    /// `memset` + alpha 1 + identity matrix and colour transform.
    fn reset() -> Self {
        Self {
            flags: 0,
            x: 0.0,
            y: 0.0,
            alpha: 1.0,
            scale_x: 0.0,
            scale_y: 0.0,
            color: 0.0,
            uv: Uv::default(),
            mtx: IDENTITY,
            cx: CXFORM_IDENTITY,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    shape: Option<usize>,
    clip: Option<usize>,
}

#[derive(Clone, Debug)]
struct Timeline {
    /// Index into `Program::timelines`.
    program: usize,
    slots: Vec<Slot>,
    frame: u32,
    prev: u32,
    state: u8,
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    used: bool,
    id: u16,
    mtx: Matrix,
    cx: Cxform,
}

#[derive(Clone, Debug)]
struct Clip {
    name: Vec<u8>,
    id: u16,
    flags: u32,
    prop: Prop,
    tl: Timeline,
    /// `drawOver`: the caller's tag, reported with the clip's drawing state after its children.
    draw_over: Option<u32>,
    /// Callbacks A / B that set a scissor before the clip and the whole screen after it: `[x0, y0, x1, y1]`.
    scissor: Option<[i32; 4]>,
}

/// A `drawOver` call: the clip's combined matrix translation and colour transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawOver {
    pub tag: u32,
    pub x: f32,
    pub y: f32,
    pub mul: [f32; 4],
    pub add: [i16; 4],
}

struct TimelineProgram {
    depths: u16,
    frames: Vec<Frame>,
    /// Label of the size-0 block that ends the list (`Flash_FindName` compares it too).
    end_label: Vec<u8>,
}

struct ShapeQuad {
    image: i16,
    rgba: u32,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
}

/// Everything read from the FOD file, parsed once.
struct Program {
    /// [0] is the root (tag 7); [1 + n] is sprite frame record n (tag 6).
    timelines: Vec<TimelineProgram>,
    /// Sprite list (tag 2): the tag-6 record each instance plays.
    sprites: Vec<u16>,
    /// Images (tag 3): width and height.
    images: Vec<(u16, u16)>,
    /// Shapes (tag 4).
    shapes: Vec<Vec<ShapeQuad>>,
}

/// A clip reference as `Flash_FindLabel` returns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRef {
    index: i32,
    more: i32,
}

impl ClipRef {
    pub fn found(&self) -> bool {
        self.index >= 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TlId {
    Root,
    Clip(usize),
}

/// A movie instance (`Flash`, src/sys/flash.c).
pub struct Flash {
    program: Program,
    root: Timeline,
    clips: Vec<Clip>,
    shapes: Vec<Shape>,
    flags: u32,
    trig: u32,
    se: u32,
    speed: u32,
    ofs: [i32; 2],
}

impl Flash {
    /// `Flash_Create`: the root timeline, one instance per sprite record, one shape per depth.
    pub fn create(file: &[u8]) -> io::Result<Self> {
        let fod = Fod::parse(file)?;
        let data = fod.bytes();
        let mut timelines = vec![parse_timeline(data, fod.record(7, 0)?)?];
        for index in 0..fod.record_count(6)? {
            timelines.push(parse_timeline(data, fod.record(6, index)?)?);
        }
        let mut sprites = Vec::new();
        if let Some(tag) = fod.find_tag(2)? {
            for index in 0..fod.header(tag)?.count {
                let rec = fod.skip_records(tag, index)?;
                let id = fod::u16_at(data, rec + 8)?;
                if usize::from(id) + 1 >= timelines.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "sprite has no frames",
                    ));
                }
                sprites.push(id);
            }
        }
        let mut images = Vec::new();
        for index in 0..fod.record_count(3)? {
            let rec = fod.record(3, index)?;
            images.push((fod::u16_at(data, rec + 8)?, fod::u16_at(data, rec + 10)?));
        }
        let mut shapes = Vec::new();
        for index in 0..fod.record_count(4)? {
            let rec = fod.record(4, index)?;
            let mut quads = Vec::new();
            for quad in 0..usize::from(fod::u16_at(data, rec + 8)?) {
                let q = rec + 10 + quad * 0x18;
                quads.push(ShapeQuad {
                    image: fod::i16_at(data, q)?,
                    rgba: fod::u32_at(data, q + 4)?,
                    x0: fod::i32_at(data, q + 8)?,
                    x1: fod::i32_at(data, q + 0xC)?,
                    y0: fod::i32_at(data, q + 0x10)?,
                    y1: fod::i32_at(data, q + 0x14)?,
                });
            }
            shapes.push(quads);
        }
        let program = Program {
            timelines,
            sprites,
            images,
            shapes,
        };

        let mut root = new_timeline(&program, 0);
        root.state |= TL_PLAY;
        let clips: Vec<Clip> = program
            .sprites
            .iter()
            .map(|&id| Clip {
                name: Vec::new(),
                id,
                flags: 0,
                prop: Prop::reset(),
                tl: new_timeline(&program, usize::from(id) + 1),
                draw_over: None,
                scissor: None,
            })
            .collect();
        let depths = usize::from(program.timelines[0].depths)
            + clips.iter().map(|clip| clip.tl.slots.len()).sum::<usize>();
        let shapes = vec![
            Shape {
                used: false,
                id: 0,
                mtx: IDENTITY,
                cx: CXFORM_IDENTITY
            };
            depths
        ];
        Ok(Self {
            program,
            root,
            clips,
            shapes,
            flags: 0,
            trig: 0,
            se: 0,
            speed: 0,
            ofs: [0; 2],
        })
    }

    fn image_size(&self, image: usize) -> Option<(u16, u16)> {
        self.program.images.get(image).copied()
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// Bits set by "se" GetURL actions during the last `advance`.
    pub fn se(&self) -> u32 {
        self.se
    }

    /// `Flash_Play`.
    pub fn play(&mut self, speed: u32) {
        self.speed = speed;
        self.flags |= FLASH_PLAY;
    }

    /// `Flash_Advance`: clears the event bits of the last tick and runs `speed` root frames.
    pub fn advance(&mut self) {
        self.trig = 0;
        self.se = 0;
        self.flags &= !FLASH_END;
        if self.flags & FLASH_PLAY != 0 {
            self.advance_tl(TlId::Root, self.speed, false, false);
        }
    }

    /// `Flash_StepFrames` (`FlashTl_StepFrames` of the root): `step` frames on, playing, if the frame exists (a
    /// step other than 1 goes there as a goto).
    pub fn step_frames(&mut self, step: i32) {
        let count = self.program.timelines[self.tl(TlId::Root).program]
            .frames
            .len() as i64;
        let tl = self.tl_mut(TlId::Root);
        let frame = i64::from(tl.frame) + i64::from(step);
        if (0..=count).contains(&frame) {
            tl.frame = frame as u32;
            tl.state |= TL_PLAY;
            if step != 1 {
                tl.state |= TL_GOTO;
            }
        }
    }

    /// `Flash_GotoLabel`: jumps the root timeline.
    pub fn goto_label(&mut self, label: &str, restart: bool) {
        self.goto_label_tl(TlId::Root, label.as_bytes(), restart);
    }

    /// `Flash_FindLabel` / `FlashClipList_FindRef`.
    pub fn find(&self, parent: Option<&str>, name: &str) -> ClipRef {
        let mut out = ClipRef { index: -1, more: 0 };
        let mut found = false;
        match parent {
            Some(parent) => {
                if let Some(clip) = self
                    .clips
                    .iter()
                    .find(|clip| clip.name == parent.as_bytes())
                {
                    for slot in &clip.tl.slots {
                        if let Some(child) = slot.clip
                            && self.clips[child].name == name.as_bytes()
                        {
                            found = true;
                            out.index = child as i32;
                            break;
                        }
                    }
                }
            }
            None => {
                for (index, clip) in self.clips.iter().enumerate() {
                    if clip.name == name.as_bytes() {
                        if found {
                            out.more += 1;
                        } else {
                            out.index = index as i32;
                            found = true;
                        }
                    }
                }
            }
        }
        if !found {
            out = ClipRef { index: -1, more: 0 };
        }
        out
    }

    /// The instance `r` points at and its next `more` namesakes, as every FlashClipList setter walks them.
    fn refs(&self, r: ClipRef) -> Vec<usize> {
        if r.index < 0 {
            return Vec::new();
        }
        let first = r.index as usize;
        let mut out = vec![first];
        let mut n = r.more;
        if n != 0 {
            for i in first + 1..self.clips.len() {
                if self.clips[i].name == self.clips[first].name {
                    out.push(i);
                    n -= 1;
                    if n == 0 {
                        break;
                    }
                }
            }
        }
        out
    }

    /// `Flash_ClipGotoLabel` / `FlashClipList_GotoLabel`: jumps and, if the jump took, runs the timeline at once.
    pub fn clip_goto_label(&mut self, r: ClipRef, label: &str) {
        for index in self.refs(r) {
            self.goto_label_tl(TlId::Clip(index), label.as_bytes(), true);
            if self.clips[index].tl.state & TL_GOTO != 0 {
                let force = self.clips[index].flags & CLIP_FREE_RUN != 0;
                self.advance_tl(TlId::Clip(index), self.speed, true, force);
            }
        }
    }

    /// `Flash_ClipSetColor`: negative values are refused; clamped to 0..1.
    pub fn clip_set_color(&mut self, r: ClipRef, color: f32) {
        if color < 0.0 {
            return;
        }
        for index in self.refs(r) {
            self.clips[index].prop.color = color.clamp(0.0, 1.0);
            self.clips[index].prop.flags |= OV_COLOR;
        }
    }

    /// `Flash_ClipSetTex`: draws with texture `tex` of the image's texture file.
    pub fn clip_set_tex(&mut self, r: ClipRef, tex: i32) {
        for index in self.refs(r) {
            let prop = &mut self.clips[index].prop;
            prop.uv.tex = tex;
            prop.flags |= OV_TEX;
        }
    }

    /// `Flash_ClipSetCallbackC`; `None` takes the callback off.
    pub fn clip_set_draw_over(&mut self, r: ClipRef, tag: Option<u32>) {
        for index in self.refs(r) {
            self.clips[index].draw_over = tag;
        }
    }

    /// `Flash_ClipSetCallbackA` / `B` with `Sprite_SetScissor(x0, x1, y0, y1)` before the clip and the whole
    /// screen after it (`[x0, y0, x1, y1]`, inclusive); `None` takes them off.
    pub fn clip_set_scissor(&mut self, r: ClipRef, rect: Option<[i32; 4]>) {
        for index in self.refs(r) {
            self.clips[index].scissor = rect;
        }
    }

    /// `Flash_ClipSetScale`: negative factors are refused.
    pub fn clip_set_scale(&mut self, r: ClipRef, x: f32, y: f32) {
        if x < 0.0 || y < 0.0 {
            return;
        }
        for index in self.refs(r) {
            let prop = &mut self.clips[index].prop;
            (prop.scale_x, prop.scale_y) = (x, y);
            prop.flags |= OV_SCALE;
        }
    }

    /// `Flash_ClipSetUv`: replaces the texture rectangle (the texture index is not copied).
    pub fn clip_set_uv(&mut self, r: ClipRef, uv: Uv) {
        for index in self.refs(r) {
            let prop = &mut self.clips[index].prop;
            (prop.uv.u0, prop.uv.v0, prop.uv.u1, prop.uv.v1) = (uv.u0, uv.v0, uv.u1, uv.v1);
            prop.flags |= OV_UV;
        }
    }

    /// `Flash_ClipSetFlags`: 1 playing, 2 visible, 4 / 8 flip, 0x10 no screen offset, 0x20 additive, 0x40
    /// subtractive, 0x80 masked, 0x100 mask writer, 0x200 repeat, 0x400 free running.
    pub fn clip_set_flags(&mut self, r: ClipRef, props: u32, on: bool) {
        for index in self.refs(r) {
            let clip = &mut self.clips[index];
            if props & PROP_PLAY != 0 {
                set_bits(&mut clip.tl.state, TL_PLAY, on);
            }
            if props & PROP_VISIBLE != 0 {
                set_bits(&mut clip.flags, CLIP_VISIBLE, on);
            }
            if props & 0x400 != 0 {
                set_bits(&mut clip.flags, CLIP_FREE_RUN, on);
            }
            for (prop, ov) in [
                (0x004, OV_FLIPX),
                (0x008, OV_FLIPY),
                (0x010, OV_NO_OFS),
                (0x020, OV_ADD),
                (0x040, OV_SUB),
                (PROP_MASK, OV_MASK),
                (PROP_MASK_SET, OV_MASK_SET),
                (PROP_REPEAT, OV_REPEAT),
            ] {
                if props & prop != 0 {
                    set_bits(&mut clip.prop.flags, ov, on);
                }
            }
        }
    }

    /// `Flash_ClipSetAlpha`: clamped to 0..1.
    pub fn clip_set_alpha(&mut self, r: ClipRef, alpha: f32) {
        for index in self.refs(r) {
            self.clips[index].prop.alpha = alpha.clamp(0.0, 1.0);
            self.clips[index].prop.flags |= OV_ALPHA;
        }
    }

    /// `Flash_ClipSetOffset`.
    pub fn clip_set_offset(&mut self, r: ClipRef, x: i32, y: i32) {
        for index in self.refs(r) {
            let prop = &mut self.clips[index].prop;
            (prop.x, prop.y) = (x as f32, y as f32);
            prop.flags |= OV_POS;
        }
    }

    /// Bits set by "trig" GetURL actions during the last `advance`.
    pub fn trig(&self) -> u32 {
        self.trig
    }

    /// `FlashClipList_GetPos`: the clip's own translation plus its position override.
    pub fn clip_get_pos(&self, r: ClipRef) -> Option<(i32, i32)> {
        let prop = &self.clips.get(usize::try_from(r.index).ok()?)?.prop;
        let (mut x, mut y) = (prop.mtx[2] as i32, prop.mtx[5] as i32);
        if prop.flags & OV_POS != 0 {
            x += prop.x as i32;
            y += prop.y as i32;
        }
        Some((x, y))
    }

    /// `FlashClipList_GetAlpha`: colour-transform alpha (0..255) times the override, over 128; 0 without a clip.
    pub fn clip_get_alpha(&self, r: ClipRef) -> f32 {
        let Some(clip) = usize::try_from(r.index)
            .ok()
            .and_then(|i| self.clips.get(i))
        else {
            return 0.0;
        };
        let prop = &clip.prop;
        let a = ((prop.cx.mul[3] * 128.0) as i32 + i32::from(prop.cx.add[3])).clamp(0, 255) as f32;
        if prop.flags & OV_ALPHA != 0 {
            a * prop.alpha * 0.0078125
        } else {
            a * 0.0078125
        }
    }

    fn tl(&self, id: TlId) -> &Timeline {
        match id {
            TlId::Root => &self.root,
            TlId::Clip(index) => &self.clips[index].tl,
        }
    }

    fn tl_mut(&mut self, id: TlId) -> &mut Timeline {
        match id {
            TlId::Root => &mut self.root,
            TlId::Clip(index) => &mut self.clips[index].tl,
        }
    }

    /// `FlashTl_GotoLabel`.
    fn goto_label_tl(&mut self, id: TlId, label: &[u8], restart: bool) {
        let Some(index) = self.find_frame(id, label) else {
            return;
        };
        let tl = self.tl_mut(id);
        tl.prev = tl.frame;
        tl.frame = index;
        tl.state |= TL_GOTO;
        if !restart {
            tl.state |= TL_KEEP;
        }
    }

    /// `Flash_FindName` over a timeline's frame blocks.
    fn find_frame(&self, id: TlId, label: &[u8]) -> Option<u32> {
        let program = &self.program.timelines[self.tl(id).program];
        program
            .frames
            .iter()
            .map(|frame| frame.label.as_slice())
            .chain(std::iter::once(program.end_label.as_slice()))
            .position(|name| name == label)
            .map(|index| index as u32)
    }

    /// `FlashTl_Rewind`.
    fn rewind(&mut self, id: TlId) {
        let tl = self.tl_mut(id);
        tl.prev = 0;
        tl.frame = 0;
        tl.state = 0;
        for depth in 0..self.tl(id).slots.len() {
            self.slot_clear(id, depth);
        }
    }

    /// `FlashTl_Advance`.
    fn advance_tl(&mut self, id: TlId, count: u32, children: bool, force: bool) {
        for _ in 0..count {
            let state = self.tl(id).state;
            if state & (TL_PLAY | TL_GOTO) != 0 {
                if state & TL_GOTO != 0 {
                    let target = self.tl(id).frame;
                    let start = if state & TL_KEEP != 0 {
                        self.tl(id).prev
                    } else {
                        self.rewind(id);
                        0
                    };
                    self.tl_mut(id).state |= TL_PLAY | TL_SEEK;
                    for frame in start..target {
                        self.run_frame(id, frame);
                    }
                    let tl = self.tl_mut(id);
                    tl.frame = target;
                    tl.state &= !(TL_GOTO | TL_SEEK | TL_KEEP);
                }
                let frame = self.tl(id).frame;
                self.run_frame(id, frame);
                let tl = self.tl_mut(id);
                if tl.state & (TL_GOTO | TL_HOLD) == 0 && tl.state & TL_PLAY != 0 {
                    tl.frame += 1;
                }
                if children {
                    self.advance_children(id, count);
                }
            } else if children && force {
                self.advance_children(id, count);
            }
        }
        if !children {
            self.advance_children(id, count);
        }
    }

    /// `FlashSlot_Advance` for every depth.
    fn advance_children(&mut self, id: TlId, count: u32) {
        for depth in 0..self.tl(id).slots.len() {
            if let Some(clip) = self.tl(id).slots[depth].clip {
                let force = self.clips[clip].flags & CLIP_FREE_RUN != 0;
                self.advance_tl(TlId::Clip(clip), count, true, force);
            }
        }
    }

    /// `FlashTl_RunFrame`: runs a frame's tags; past the end it stops on the last frame.
    fn run_frame(&mut self, id: TlId, frame: u32) {
        let program = self.tl(id).program;
        let Some(tags) = self.program.timelines[program]
            .frames
            .get(frame as usize)
            .map(|f| f.tags.clone())
        else {
            let tl = self.tl_mut(id);
            tl.state &= !TL_PLAY;
            tl.frame = tl.frame.saturating_sub(1);
            return;
        };
        for tag in tags {
            match tag {
                FrameTag::Place {
                    depth,
                    kind,
                    id: object,
                    matrix,
                    cxform,
                } => match kind {
                    4 => self.slot_set_shape(id, depth, object, matrix, CXFORM_IDENTITY),
                    6 => self.slot_set_clip(id, depth, object, Vec::new(), matrix, cxform),
                    _ => {}
                },
                FrameTag::PlaceNew {
                    depth,
                    kind,
                    id: object,
                    matrix,
                    cxform,
                    name,
                } => match kind {
                    4 => self.slot_set_shape(id, depth, object, matrix, cxform),
                    6 => self.slot_set_clip(id, depth, object, name, matrix, cxform),
                    _ => {}
                },
                FrameTag::Move {
                    depth,
                    matrix,
                    cxform,
                } => {
                    if let Some(slot) = self.slot(id, depth) {
                        if let Some(shape) = slot.shape {
                            let shape = &mut self.shapes[shape];
                            shape.mtx = matrix.unwrap_or(shape.mtx);
                            shape.cx = cxform.unwrap_or(shape.cx);
                        }
                        if let Some(clip) = slot.clip {
                            let prop = &mut self.clips[clip].prop;
                            prop.mtx = matrix.unwrap_or(prop.mtx);
                            prop.cx = cxform.unwrap_or(prop.cx);
                        }
                    }
                }
                FrameTag::Remove { depth, kind, .. } => {
                    let Some(slot) = self.slot(id, depth) else {
                        continue;
                    };
                    match kind {
                        4 => {
                            if let Some(shape) = slot.shape {
                                self.shapes[shape].used = false;
                                self.tl_mut(id).slots[usize::from(depth) - 1].shape = None;
                            }
                        }
                        6 => {
                            if let Some(clip) = slot.clip {
                                self.clip_reset(clip);
                                self.tl_mut(id).slots[usize::from(depth) - 1].clip = None;
                            }
                        }
                        _ => {}
                    }
                }
                FrameTag::Remove2 { depth } => {
                    if self.slot(id, depth).is_some() {
                        self.slot_clear(id, usize::from(depth) - 1);
                    }
                }
                FrameTag::Actions(actions) => self.run_actions(id, &actions),
                FrameTag::Unknown(_) => {}
            }
        }
    }

    fn slot(&self, id: TlId, depth: u16) -> Option<Slot> {
        usize::from(depth)
            .checked_sub(1)
            .and_then(|index| self.tl(id).slots.get(index).copied())
    }

    /// `FlashTl_TagActions`, or `FlashTl_SkipActions` while a jump replays frames.
    fn run_actions(&mut self, id: TlId, actions: &[Action]) {
        if self.tl(id).state & TL_SEEK != 0 {
            for action in actions {
                if let Action::GetUrl(url, arg) = action
                    && url == b"pad"
                {
                    self.set_pad(arg);
                }
            }
            return;
        }
        let mut cur = id;
        for action in actions {
            match action {
                Action::NextFrame => {
                    let tl = self.tl_mut(cur);
                    tl.frame += 1;
                    tl.state |= TL_HOLD;
                }
                Action::PrevFrame => {
                    let tl = self.tl_mut(cur);
                    tl.frame = tl.frame.saturating_sub(1);
                    tl.state |= TL_HOLD;
                }
                Action::Play => self.tl_mut(cur).state |= TL_PLAY,
                Action::Stop => self.tl_mut(cur).state &= !TL_PLAY,
                Action::GotoFrame(frame) => {
                    let tl = self.tl_mut(cur);
                    tl.frame = u32::from(*frame).wrapping_sub(1);
                    tl.state |= TL_GOTO;
                }
                Action::GetUrl(url, arg) => match url.as_slice() {
                    b"pad" => self.set_pad(arg),
                    b"trig" => self.trig |= 1u32.wrapping_shl(atoi(arg)),
                    b"se" => self.se |= 1u32.wrapping_shl(atoi(arg)),
                    b"trigger" if arg == b"end" => self.flags |= FLASH_END,
                    _ => {}
                },
                Action::SetTarget(name) => {
                    cur = if name.is_empty() {
                        id
                    } else {
                        self.clips
                            .iter()
                            .position(|clip| &clip.name == name)
                            .map_or(id, TlId::Clip)
                    };
                }
                Action::GotoLabel(label) => {
                    if let Some(index) = self.find_frame(cur, label) {
                        let tl = self.tl_mut(cur);
                        tl.frame = index;
                        tl.state |= TL_GOTO;
                    }
                }
                Action::WaitForFrame | Action::Other(_) => {}
            }
        }
    }

    fn set_pad(&mut self, arg: &[u8]) {
        match arg {
            b"true" => self.flags |= FLASH_PAD,
            b"false" => self.flags &= !FLASH_PAD,
            _ => {}
        }
    }

    /// `FlashSlot_SetShape` + `FlashShape_Create`.
    fn slot_set_shape(&mut self, id: TlId, depth: u16, shape_id: u16, mtx: Matrix, cx: Cxform) {
        let Some(slot) = self.slot(id, depth) else {
            return;
        };
        if let Some(shape) = slot.shape {
            self.shapes[shape].used = false;
        }
        let new = self.shapes.iter().position(|shape| !shape.used);
        if let Some(index) = new {
            self.shapes[index] = Shape {
                used: true,
                id: shape_id,
                mtx,
                cx,
            };
        }
        self.tl_mut(id).slots[usize::from(depth) - 1].shape = new;
    }

    /// `FlashSlot_SetClip` + `FlashClip_Start`.
    fn slot_set_clip(
        &mut self,
        id: TlId,
        depth: u16,
        sprite: u16,
        name: Vec<u8>,
        mtx: Matrix,
        cx: Cxform,
    ) {
        let Some(slot) = self.slot(id, depth) else {
            return;
        };
        if let Some(clip) = slot.clip {
            self.clip_reset(clip);
        }
        let new = self
            .clips
            .iter()
            .position(|clip| clip.flags & CLIP_USED == 0 && clip.id == sprite);
        if let Some(index) = new {
            let clip = &mut self.clips[index];
            clip.name = name;
            clip.flags |= CLIP_USED | CLIP_VISIBLE;
            clip.prop = Prop {
                mtx,
                cx,
                ..Prop::reset()
            };
            clip.tl.state |= TL_PLAY;
            if clip.tl.prev != 0 {
                clip.tl.frame = clip.tl.prev;
                clip.tl.state |= TL_GOTO;
                clip.tl.prev = 0;
            }
        }
        self.tl_mut(id).slots[usize::from(depth) - 1].clip = new;
    }

    /// `FlashSlot_Clear`: frees the shape; the sprite only loses its USED flag.
    fn slot_clear(&mut self, id: TlId, index: usize) {
        let slot = self.tl(id).slots[index];
        if let Some(shape) = slot.shape {
            self.shapes[shape].used = false;
        }
        if let Some(clip) = slot.clip {
            self.clips[clip].flags &= !CLIP_USED;
        }
        self.tl_mut(id).slots[index] = Slot::default();
    }

    /// `FlashClip_Reset`.
    fn clip_reset(&mut self, index: usize) {
        let clip = &mut self.clips[index];
        clip.name.clear();
        clip.flags = 0;
        clip.prop = Prop::reset();
        self.rewind(TlId::Clip(index));
    }

    /// `Flash_Draw`: every quad of the movie, back to front. `has_texture(image)` is `flash->tex[image] != NULL`.
    pub fn draw(&self, has_texture: &dyn Fn(usize) -> bool) -> Vec<DrawQuad> {
        self.draw_with_overs(has_texture).0
    }

    /// `Flash_Draw`, with the `drawOver` calls it makes in order.
    pub fn draw_with_overs(
        &self,
        has_texture: &dyn Fn(usize) -> bool,
    ) -> (Vec<DrawQuad>, Vec<DrawOver>) {
        let mut quads = Vec::new();
        let mut overs = Vec::new();
        if self.flags & FLASH_HIDE == 0 {
            self.draw_tl(TlId::Root, None, has_texture, &mut quads, &mut overs);
        }
        (quads, overs)
    }

    /// `FlashTl_Draw` + `FlashSlot_Draw`.
    fn draw_tl(
        &self,
        id: TlId,
        parent: Option<&Prop>,
        has_texture: &dyn Fn(usize) -> bool,
        quads: &mut Vec<DrawQuad>,
        overs: &mut Vec<DrawOver>,
    ) {
        for slot in &self.tl(id).slots {
            if let Some(shape) = slot.shape {
                self.draw_shape(&self.shapes[shape], parent, has_texture, quads);
            }
            let Some(clip_index) = slot.clip else {
                continue;
            };
            let clip = &self.clips[clip_index];
            if clip.flags & CLIP_VISIBLE == 0 {
                continue;
            }
            let mut prop = clip.prop;
            if let Some(parent) = parent {
                prop.flags |= parent.flags & (OV_MASK | OV_MASK_SET);
                let mut m = [0.0f32; 9];
                for i in 0..3 {
                    for j in 0..3 {
                        m[i * 3 + j] = prop.mtx[j] * parent.mtx[i * 3]
                            + prop.mtx[3 + j] * parent.mtx[i * 3 + 1]
                            + prop.mtx[6 + j] * parent.mtx[i * 3 + 2];
                    }
                }
                prop.mtx = m;
                if parent.flags & OV_POS != 0 {
                    prop.mtx[2] += parent.x;
                    prop.mtx[5] += parent.y;
                }
                for i in 0..4 {
                    prop.cx.mul[i] *= parent.cx.mul[i];
                    if i < 3 {
                        if parent.flags & OV_COLOR != 0 {
                            prop.cx.mul[i] *= parent.color;
                        }
                    } else if parent.flags & OV_ALPHA != 0 {
                        prop.cx.mul[i] *= parent.alpha;
                    }
                    prop.cx.mul[i] = prop.cx.mul[i].clamp(0.0, 1.0);
                    let add = i32::from(prop.cx.add[i]) + i32::from(parent.cx.add[i]);
                    prop.cx.add[i] = add.clamp(0, 255) as i16;
                }
            }
            let first = quads.len();
            self.draw_tl(
                TlId::Clip(clip_index),
                Some(&prop),
                has_texture,
                quads,
                overs,
            );
            if let Some(rect) = clip.scissor {
                // Callback B sets the whole screen again, so a scissor inside keeps its own.
                for q in &mut quads[first..] {
                    q.scissor.get_or_insert(rect);
                }
            }
            if let Some(tag) = clip.draw_over {
                overs.push(DrawOver {
                    tag,
                    x: prop.mtx[2],
                    y: prop.mtx[5],
                    mul: prop.cx.mul,
                    add: prop.cx.add,
                });
            }
        }
    }

    /// `FlashShape_Draw`: each quad of the shape record through `FlashDraw_TransformQuad`.
    fn draw_shape(
        &self,
        shape: &Shape,
        prop: Option<&Prop>,
        has_texture: &dyn Fn(usize) -> bool,
        quads: &mut Vec<DrawQuad>,
    ) {
        let Some(records) = self.program.shapes.get(usize::from(shape.id)) else {
            return;
        };
        for rec in records {
            let mut quad = DrawQuad {
                flags: 0,
                pt: [
                    [rec.x0 as f32, rec.y0 as f32],
                    [rec.x1 as f32, rec.y0 as f32],
                    [rec.x0 as f32, rec.y1 as f32],
                    [rec.x1 as f32, rec.y1 as f32],
                ],
                uv: Uv::default(),
                rgba: [
                    (rec.rgba >> 24) as i32,
                    ((rec.rgba >> 16) & 0xFF) as i32,
                    ((rec.rgba >> 8) & 0xFF) as i32,
                    (rec.rgba & 0xFF) as i32,
                ],
                image: None,
                scissor: None,
            };
            if rec.image >= 0 {
                let image = rec.image as usize;
                if !has_texture(image) {
                    continue;
                }
                let Some((w, h)) = self.image_size(image) else {
                    continue;
                };
                quad.image = Some(image);
                quad.uv = Uv {
                    u0: 0,
                    v0: 0,
                    u1: i32::from(w),
                    v1: i32::from(h),
                    tex: 0,
                };
            }
            transform_quad(&mut quad, &shape.mtx, &shape.cx, prop, self.ofs);
            if quad.rgba[3] != 0 {
                quads.push(quad);
            }
        }
    }
}

/// `FlashDraw_TransformQuad`: shape matrix and colour transform, then the sprite's drawing state and overrides.
fn transform_quad(q: &mut DrawQuad, mtx: &Matrix, cx: &Cxform, prop: Option<&Prop>, ofs: [i32; 2]) {
    for point in q.pt.iter_mut().take(3) {
        let [x, y] = *point;
        point[0] = x * mtx[0] + y * mtx[3] + mtx[2];
        point[1] = x * mtx[1] + y * mtx[4] + mtx[5];
        if prop.is_none() {
            point[0] += ofs[0] as f32;
            point[1] += ofs[1] as f32;
        }
    }
    close_parallelogram(q);
    apply_cxform(&mut q.rgba, cx);
    let Some(prop) = prop else { return };
    let (mut sx, mut sy) = (prop.mtx[0], prop.mtx[4]);
    if prop.flags & OV_SCALE != 0 {
        sx *= prop.scale_x;
        sy *= prop.scale_y;
    }
    for point in q.pt.iter_mut().take(3) {
        let [x, y] = *point;
        point[0] = x * sx + y * prop.mtx[3] + prop.mtx[2];
        point[1] = x * prop.mtx[1] + y * sy + prop.mtx[5];
        if prop.flags & OV_POS != 0 {
            point[0] += prop.x;
            point[1] += prop.y;
        }
        if prop.flags & OV_NO_OFS == 0 {
            point[0] += ofs[0] as f32;
            point[1] += ofs[1] as f32;
        }
    }
    close_parallelogram(q);
    apply_cxform(&mut q.rgba, &prop.cx);
    if prop.flags & OV_COLOR != 0 {
        for channel in &mut q.rgba[..3] {
            *channel = ((*channel as f32 * prop.color) as i32).clamp(0, 255);
        }
    }
    if prop.flags & OV_ALPHA != 0 {
        q.rgba[3] = ((q.rgba[3] as f32 * prop.alpha) as i32).clamp(0, 255);
    }
    if prop.flags & OV_UV != 0 {
        q.uv = Uv { tex: 0, ..prop.uv };
    }
    if prop.flags & OV_TEX != 0 {
        q.uv.tex = prop.uv.tex;
    }
    for (ov, quad) in [
        (OV_FLIPX, QUAD_FLIPX),
        (OV_FLIPY, QUAD_FLIPY),
        (OV_REPEAT, QUAD_REPEAT),
        (OV_ADD, QUAD_ADD),
        (OV_SUB, QUAD_SUB),
        (OV_MASK, QUAD_MASK),
        (OV_MASK_SET, QUAD_MASK_SET),
    ] {
        if prop.flags & ov != 0 {
            q.flags |= quad;
        }
    }
}

fn close_parallelogram(q: &mut DrawQuad) {
    q.pt[3][0] = q.pt[1][0] + q.pt[2][0] - q.pt[0][0];
    q.pt[3][1] = q.pt[1][1] + q.pt[2][1] - q.pt[0][1];
}

fn apply_cxform(rgba: &mut [i32; 4], cx: &Cxform) {
    for i in 0..4 {
        rgba[i] = ((rgba[i] as f32 * cx.mul[i]) as i32 + i32::from(cx.add[i])).clamp(0, 255);
    }
}

fn set_bits<T>(value: &mut T, mask: T, on: bool)
where
    T: Copy
        + std::ops::BitOr<Output = T>
        + std::ops::BitAnd<Output = T>
        + std::ops::Not<Output = T>,
{
    *value = if on { *value | mask } else { *value & !mask };
}

fn atoi(arg: &[u8]) -> u32 {
    let digits: Vec<u8> = arg.iter().copied().take_while(u8::is_ascii_digit).collect();
    std::str::from_utf8(&digits)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn new_timeline(program: &Program, index: usize) -> Timeline {
    Timeline {
        program: index,
        slots: vec![Slot::default(); usize::from(program.timelines[index].depths)],
        frame: 0,
        prev: 0,
        state: 0,
    }
}

/// `FlashTl_Init`: a record whose header count is the depth count, then the frame blocks.
fn parse_timeline(data: &[u8], record: usize) -> io::Result<TimelineProgram> {
    let depths = fod::u16_at(data, record + 2)?;
    let mut frames = Vec::new();
    let mut ofs = record + 8;
    let end_label = loop {
        match fod::parse_frame(data, ofs)? {
            Some(frame) => {
                frames.push(frame);
                ofs = fod::skip_named(data, ofs, 1)?.expect("a parsed frame has a size");
            }
            None => break fod::read_string(data, ofs)?.0.to_vec(),
        }
    };
    Ok(TimelineProgram {
        depths,
        frames,
        end_label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad drawn under a clip's scissor (inclusive bounds) is cut to it, its texels following; one outside
    /// it is dropped.
    #[test]
    fn scissored_quads_are_cut_to_the_window() {
        let quad = |x0: f32, x1: f32| DrawQuad {
            flags: 0,
            pt: [[x0, 0.0], [x1, 0.0], [x0, 100.0], [x1, 100.0]],
            uv: Uv {
                u0: 0,
                v0: 0,
                u1: 102,
                v1: 102,
                tex: 0,
            },
            rgba: [0x80; 4],
            image: None,
            scissor: Some([0, 20, 49, 79]),
        };
        let out = screen_quads(&[quad(0.0, 100.0), quad(60.0, 100.0)], &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pt[0], [0.0, 20.0]);
        assert_eq!(out[0].pt[3], [50.0, 80.0]);
        assert_eq!(out[0].uv[0], [1.0, 21.0]);
        assert_eq!(out[0].uv[3], [51.0, 81.0]);
    }
}
