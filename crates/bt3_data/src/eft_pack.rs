//! Effect packs: emitter sets (`EftEmit_LoadSet`, src/battle/eft_emit.c: a header, groups of one particle module
//! each, their emitters and the resources that follow) and the "V000" keyframed sprite packs the kind 14 module
//! plays (`EftSprPack_*`, src/battle/eft_sprite_anim.c).

use std::io;

/// `EFT_EMIT_TYPES`: group kinds (one per particle module).
pub const SET_KINDS: usize = 19;

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map_or(0, |w| u32::from_le_bytes(w.try_into().unwrap()))
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_bits(u32_at(b, o))
}

/// `BtlScene_GetPackEntry`: the offset of entry `n` (no emptiness test).
fn entry_offset(pack: &[u8], n: usize) -> usize {
    (u32_at(pack, n * 4) & !3) as usize
}

/// `EftSetHead` / `EftEmitHdr` (0x40 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SetHead {
    /// Bit per group kind present.
    pub mask: u32,
    /// `aimKind`: how a volley spreads its shots (7 straight up, 8 lifted 45 degrees, ...).
    pub aim_kind: u8,
    /// The trail width animation: width0 -> width1 -> width2 over `width_time` seconds, or `width_step` per frame.
    pub width: [f32; 3],
    pub width_time: f32,
    pub width_split: f32,
    pub width_step: f32,
    /// Frames the end animation of a shot runs.
    pub end_frames: u8,
    /// 1: effect type 2 moves its target point (by `move_kind`); 4: the second width animation is used.
    pub flags: u8,
    /// `unk22`: how effect type 2 moves its target point: 0 straight, 1 in front of the opponent, 2 past it.
    pub move_kind: u8,
    /// `unk25`: how effect type 2 aims (`EftShotTech_CalcDir`): 0 at the opponent's node 3, 1 node 0x2E to 3, 2
    /// the aim, 3 node 0x22 to 0x23, 4 node 0x14 to 0x15, 5 node 0x36's -z.
    pub dir_kind: u8,
    /// `unk26`: effect type 2's options: 1 the phase 4 emitters and shots at node 0x36, 2 onto the opponent, 4
    /// started by event 0x10, 8 waits for event 0x10, 0x10 the shots placed on the target point, 0x20 it fires
    /// shots, 0x40 aimed again once the rush connects.
    pub options: u8,
    /// `unk28`: the distance in front of the opponent of move kind 1.
    pub gap: f32,
    /// Bit per phase the set has emitters for.
    pub phase_mask: u8,
    /// The second width animation (flag 4): b0 -> b1 -> b2 over `width_b_time` seconds (`EftEmit_UpdateWidth2`).
    pub width_b: [f32; 3],
    pub width_b_time: f32,
    pub width_b_split: f32,
}

/// `EftEmitDef` (0x40 bytes): one emitter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmitPart(pub [u8; 0x40]);

impl EmitPart {
    /// 0x02: the second point is written without a move (kinds 15 / 17) or never moved; 0x08: the node comes from
    /// the c[] slots; 0x20: an extra module call; 0x40: at the node, not offset along the direction; 0x80: write
    /// the direction.
    pub fn flags(&self) -> u8 {
        self.0[0]
    }
    /// Resource index inside the group.
    pub fn res(&self) -> usize {
        usize::from(self.0[1])
    }
    /// Texture index inside the resource.
    pub fn tex(&self) -> u8 {
        self.0[2]
    }
    pub fn count(&self) -> u8 {
        self.0[3]
    }
    pub fn mode(&self) -> u8 {
        self.0[4]
    }
    pub fn delay(&self) -> u8 {
        self.0[5]
    }
    pub fn hold(&self) -> u8 {
        self.0[6]
    }
    pub fn fade(&self) -> u8 {
        self.0[7]
    }
    /// The phase that starts it: 0, 1, 3, 4, 5 by event; 2 at the end; 6 never stopped.
    pub fn phase(&self) -> u8 {
        self.0[8]
    }
    /// The event that stops it: 1, 2, 4, 5, 6.
    pub fn end_phase(&self) -> u8 {
        self.0[9]
    }
    /// Index into `gEftEmitNodeSlot`; 6 = the opponent's node 3.
    pub fn node(&self) -> u8 {
        self.0[0xA]
    }
    /// 0 dir, 1 -dir, 2 / 3 perpendicular, 4 up, 5 down.
    pub fn dir_mode(&self) -> u8 {
        self.0[0xB]
    }
    pub fn spread(&self) -> f32 {
        f32_at(&self.0, 0xC)
    }
    pub fn offset(&self) -> f32 {
        f32_at(&self.0, 0x10)
    }
    pub fn offset2(&self) -> f32 {
        f32_at(&self.0, 0x14)
    }
    pub fn rate(&self) -> f32 {
        f32_at(&self.0, 0x18)
    }
    /// Scale animation: scale0 -> scale1 -> scale2 over `scale_time` seconds, or `scale_step` per frame.
    pub fn scale(&self, i: usize) -> f32 {
        f32_at(&self.0, 0x1C + i * 4)
    }
    pub fn scale_time(&self) -> f32 {
        f32_at(&self.0, 0x28)
    }
    pub fn scale_split(&self) -> f32 {
        f32_at(&self.0, 0x2C)
    }
    pub fn scale_step(&self) -> f32 {
        f32_at(&self.0, 0x30)
    }
    /// 0x01 depends on the opponent's parameter flag 0x80 (0x10: wanted set), 0x04 has a condition, 0x08 an extra
    /// module call.
    pub fn flags2(&self) -> u8 {
        self.0[0x34]
    }
    pub fn cond(&self) -> u8 {
        self.0[0x35]
    }
    pub fn alt_node(&self) -> u8 {
        self.0[0x36]
    }
    /// 0 the caller's scale, 1 the owner's body scale, 2 the opponent's.
    pub fn scale_mode(&self) -> u8 {
        self.0[0x37]
    }
}

/// `EftSetGroup`: the emitters of one kind and where its resources are.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmitGroup {
    pub kind: u8,
    /// Index of the group's first emitter in `parts` (and in the handle tables).
    pub first: usize,
    pub count: usize,
    /// Index of its first emitter in `pairs` (kind 0 emitters have none).
    pub first_pair: usize,
    /// `EftEmit_TypeToResKind`: which of the four texture-set arrays its resources go to.
    pub cat: usize,
    /// Index of its first resource in that array.
    pub cat_first: usize,
    pub res_count: usize,
}

/// `EftEmit_TypeToResKind`: 3 for kind 2; 1 for 5, 10, 12, 15..18; 2 for 9; else 0 (sets of 32 / 16 / 8 / 4).
pub fn res_kind(kind: u8) -> usize {
    match kind {
        2 => 3,
        5 | 10 | 12 | 15..=18 => 1,
        9 => 2,
        _ => 0,
    }
}

/// A parsed emitter set: entry `head` of `data` is the header, the entries after it the resources (texture packs)
/// and the emitters' definition blocks, in group order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmitSet {
    pub head: SetHead,
    /// By kind.
    pub groups: [Option<EmitGroup>; SET_KINDS],
    pub parts: Vec<EmitPart>,
    /// The texture packs of each resource category, in order (offsets into `data`).
    pub textures: [Vec<usize>; 4],
    /// Per emitter that has them (kind 0 has none): its one or two definition blocks (offsets into `data`); kinds
    /// 2 and 14 have only the first.
    pub pairs: Vec<(usize, Option<usize>)>,
    pub data: Vec<u8>,
}

impl EmitSet {
    /// `EftEmit_LoadSet` of a pack whose header is entry `idx`.
    pub fn parse(pack: &[u8], idx: usize) -> io::Result<Self> {
        let h_ofs = entry_offset(pack, idx);
        let h = pack
            .get(h_ofs..h_ofs + 0x40)
            .ok_or_else(|| invalid_data("emitter set header is outside the pack"))?;
        let head = SetHead {
            mask: u32_at(h, 0),
            aim_kind: h[6],
            width: [f32_at(h, 8), f32_at(h, 0xC), f32_at(h, 0x10)],
            width_time: f32_at(h, 0x14),
            width_split: f32_at(h, 0x18),
            width_step: f32_at(h, 0x1C),
            end_frames: h[0x20],
            flags: h[0x21],
            move_kind: h[0x22],
            dir_kind: h[0x25],
            options: h[0x26],
            gap: f32_at(h, 0x28),
            phase_mask: h[0x27],
            width_b: [f32_at(h, 0x2C), f32_at(h, 0x30), f32_at(h, 0x34)],
            width_b_time: f32_at(h, 0x38),
            width_b_split: f32_at(h, 0x3C),
        };
        let (n_groups, n_parts) = (usize::from(h[4]), usize::from(h[5]));
        if n_groups > SET_KINDS || n_parts > 40 {
            return Err(invalid_data("emitter set has too many groups or emitters"));
        }
        let defs = h_ofs + 0x40;
        let part_base = defs + n_groups * 0x20;
        let parts = (0..n_parts)
            .map(|i| {
                pack.get(part_base + i * 0x40..part_base + i * 0x40 + 0x40)
                    .map(|b| EmitPart(b.try_into().unwrap()))
                    .ok_or_else(|| invalid_data("emitter is outside the pack"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut groups = [None; SET_KINDS];
        let mut count = [0usize; 4];
        let (mut n_part, mut n_pair) = (0, 0);
        let mut order = Vec::new();
        for g in 0..n_groups {
            let d = &pack[defs + g * 0x20..defs + g * 0x20 + 3];
            let (kind, n, res) = (d[0], usize::from(d[1]), usize::from(d[2]));
            if usize::from(kind) >= SET_KINDS {
                return Err(invalid_data("emitter group of an unknown kind"));
            }
            let cat = res_kind(kind);
            groups[usize::from(kind)] = Some(EmitGroup {
                kind,
                first: n_part,
                count: n,
                first_pair: n_pair,
                cat,
                cat_first: count[cat],
                res_count: res,
            });
            n_part += n;
            if kind != 0 {
                n_pair += n;
                count[cat] += res;
            }
            order.push(kind);
        }
        let mut textures: [Vec<usize>; 4] = Default::default();
        let mut pairs = vec![(0, None); n_pair];
        let mut next = idx + 1;
        for kind in order {
            if kind == 0 {
                continue;
            }
            let g = groups[usize::from(kind)].unwrap();
            for _ in 0..g.res_count {
                textures[g.cat].push(entry_offset(pack, next));
                next += 1;
            }
            let two = !matches!(kind, 2 | 0xE);
            for j in 0..g.count {
                let a = entry_offset(pack, next);
                next += 1;
                let b = two.then(|| {
                    next += 1;
                    entry_offset(pack, next - 1)
                });
                pairs[g.first_pair + j] = (a, b);
            }
        }
        Ok(Self {
            head,
            groups,
            parts,
            textures,
            pairs,
            data: pack.to_vec(),
        })
    }

    /// The bytes from an offset `textures` or `pairs` gave.
    pub fn at(&self, offset: usize) -> &[u8] {
        self.data.get(offset..).unwrap_or_default()
    }
}

/// `EftSprKey` (0x40 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SprKey {
    /// Offset from the object, in the object's frame.
    pub pos: [f32; 3],
    /// Degrees; with the spin flag the first key's is a spin per frame.
    pub rot: [f32; 3],
    pub scale: [f32; 3],
    /// 0..255 per channel.
    pub color: [u8; 4],
    pub frame: i32,
}

/// `EftSprLayerDef` with its keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SprLayer {
    pub tex: u8,
    /// Ordering-table layer; >= 2 the second GS context.
    pub ot_layer: u8,
    /// Frames advance by speed * 2 per step.
    pub speed: u8,
    pub loop_end_key: u8,
    pub loop_start_key: u8,
    pub loop_count: i8,
    pub length: i16,
    pub jitter_period: i16,
    /// 0: 2x2, 1: 4x2, 2: 4x4, 3: 3x3.
    pub grid: i8,
    pub blink_on: i8,
    pub blink_off: i8,
    pub grid_step: i8,
    /// `EFT_SPR_LAYER_*`.
    pub flags: u32,
    pub fade: i32,
    pub jitter: [i16; 2],
    pub keys: Vec<SprKey>,
}

/// A "V000" pack (`EftSprPack`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SprPack {
    /// Restart when every layer has finished.
    pub looped: bool,
    pub layers: Vec<SprLayer>,
}

impl SprPack {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.get(0..4) != Some(b"V000") {
            return Err(invalid_data("sprite pack has no V000 magic"));
        }
        let n = usize::from(b[4]);
        let mut key = u32_at(b, 8) as usize;
        let mut layers = Vec::with_capacity(n);
        for i in 0..n {
            let d = b
                .get(0x20 + i * 0x20..0x40 + i * 0x20)
                .ok_or_else(|| invalid_data("sprite layer is outside the pack"))?;
            let s16 = |o: usize| i16::from_le_bytes([d[o], d[o + 1]]);
            // EftSprPack_Bind: the keys of the layers follow each other.
            let count = usize::from(d[4]);
            let keys = (0..count)
                .map(|k| {
                    let o = key + k * 0x40;
                    let r = b
                        .get(o..o + 0x40)
                        .ok_or_else(|| invalid_data("sprite key is outside the pack"))?;
                    let v = |at: usize| [f32_at(r, at), f32_at(r, at + 4), f32_at(r, at + 8)];
                    Ok(SprKey {
                        pos: v(0),
                        rot: v(0x10),
                        scale: v(0x20),
                        color: [r[0x30], r[0x31], r[0x32], r[0x33]],
                        frame: u32_at(r, 0x34) as i32,
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            key += count * 0x40;
            layers.push(SprLayer {
                tex: d[0],
                ot_layer: d[2],
                speed: d[3],
                loop_end_key: d[5],
                loop_start_key: d[6],
                loop_count: d[7] as i8,
                length: s16(8),
                jitter_period: s16(0xA),
                grid: d[0xC] as i8,
                blink_on: d[0xD] as i8,
                blink_off: d[0xE] as i8,
                grid_step: d[0xF] as i8,
                flags: u32_at(d, 0x10),
                fade: u32_at(d, 0x18) as i32,
                jitter: [s16(0x1C), s16(0x1E)],
                keys,
            });
        }
        Ok(Self {
            looped: b[5] != 0,
            layers,
        })
    }
}

/// What the battle reads from a character's effect pack (entry 6 of the character's second animation file,
/// `BtlCharApi_GetPlayerCharPack`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectPack {
    /// The first word of entry 1 of entry 1 (`BtlScene_TestCharPackBit`): bit n - 1 creates the manager of kind n.
    pub kind_bits: u32,
    /// The emitter sets of the ki blast managers (`EftKiBlastMgr_Init`): kind 1 is entry 3, kind 2 entry 4; each
    /// header is entry 1 of its entry. `None` when it cannot be read.
    pub blast_sets: [Option<EmitSet>; 2],
}

/// `BObjFile_GetEntry`: entry `n` of a pack, `None` when it is empty.
fn pack_entry(pack: &[u8], n: usize) -> Option<&[u8]> {
    let start = entry_offset(pack, n);
    let end = entry_offset(pack, n + 1);
    (start != end).then(|| pack.get(start..)).flatten()
}

impl EffectPack {
    /// From the character's second animation file (`chara * 10 + 0x599`); `None` without the pack.
    pub fn parse(anim_file: &[u8]) -> Option<Self> {
        let pack = pack_entry(anim_file, 6)?;
        let kind_bits = pack
            .get(entry_offset(pack, 1)..)
            .and_then(|e| e.get(entry_offset(e, 1)..))
            .map_or(0, |w| u32_at(w, 0));
        let set = |n: usize| {
            let p = pack.get(entry_offset(pack, n)..)?;
            EmitSet::parse(p, 1).ok()
        };
        Some(Self {
            kind_bits,
            blast_sets: [set(3), set(4)],
        })
    }
}

/// `BtlScene_GetCommonEntry`: entry `idx` of the common effect file (common file 3, `gCommonRes->data[1]`).
pub fn common_entry(file: &[u8], idx: usize) -> Option<&[u8]> {
    file.get(idx * 4..idx * 4 + 4)?;
    file.get(entry_offset(file, idx)..)
}

/// `EftRayMgr_Init`: the ray bursts' texture set is common effect entry 10.
pub const COMMON_RAY_TEXTURES: usize = 10;

/// `EftShotTechMgr_Init` / `EftRushShotMgr_Init`: the camera animation a rush technique of slot `slot` (2..4)
/// plays once it connects: sub-entry `slot - 1` of entry 8 of the character pack (`BtlScene_GetCharPackEntry`,
/// entry 6 of the second animation file); `None` when it is empty.
pub fn rush_cam(anim_file: &[u8], slot: usize) -> Option<&[u8]> {
    if !(2..5).contains(&slot) {
        return None;
    }
    let pack = pack_entry(anim_file, 6)?;
    let cams = pack.get(entry_offset(pack, 8)..)?;
    pack_entry(cams, slot - 1)
}

/// `BtlCharApi_GetPlayerEffectPack`: the effect pack of technique slot `slot` (0..4) is entry `slot + 1` of the
/// character's second animation file (`BtlObj_RequestCharaModel`); `None` when it is empty.
pub fn tech_pack(anim_file: &[u8], slot: usize) -> Option<&[u8]> {
    if slot >= 5 {
        return None;
    }
    pack_entry(anim_file, slot + 1)
}

/// `BtlSuper_GetStagePlacement`'s table (`BtlChar_BindObject`: the table at the pack's header word 3): the five
/// placements (x, y, z, yaw in degrees) of a rush technique's steps on stage `stage`, the stages one after another.
pub fn tech_places(pack: &[u8], stage: usize) -> Option<[[f32; 4]; 5]> {
    let table = usize::try_from(u32::from_le_bytes(pack.get(12..16)?.try_into().ok()?)).ok()?;
    let at = table.checked_add(stage.checked_mul(5 * 0x10)?)?;
    let b = pack.get(at..at.checked_add(5 * 0x10)?)?;
    Some(std::array::from_fn(|n| {
        std::array::from_fn(|k| {
            let o = n * 0x10 + k * 4;
            f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
        })
    }))
}

/// `EftBlastMgr_Init`: the emitter set of a type 0 technique effect (the plain blast / beam), header entry 4 of
/// the slot's pack.
pub fn tech_blast_set(pack: &[u8]) -> Option<EmitSet> {
    EmitSet::parse(pack, 4).ok()
}

/// `EftRbnPrm`: a ribbon's parameter block (the emitter's first definition block). "[3]" is the value at the three
/// keys of its animation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RibbonPrm {
    /// 1 animated, 2 random mirror, 4 scroll (second layer), 8 width pulse, 0x10 colour pulse.
    pub flags: i32,
    /// Ordering-table layer of its triangles.
    pub blend: u8,
    /// 0 a strip between two points, 1 and 2 trails.
    pub kind: u8,
    /// Seconds.
    pub fade_in: f32,
    pub fade_out: f32,
    pub anim_time: f32,
    pub anim_split: f32,
    pub width: [f32; 3],
    pub width_lo: [f32; 3],
    pub width_hi: [f32; 3],
    /// Seconds.
    pub width_time: [f32; 3],
    pub alpha: [f32; 3],
    pub scroll: [f32; 3],
}

/// `EftRbnAnim`: a ribbon's colours and colour pulse at the three keys (its second definition block).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RibbonAnim {
    pub color: [[f32; 4]; 3],
    /// Low and high value of the pulse per key, for red, green and blue.
    pub pulse: [[[f32; 2]; 3]; 3],
    /// Seconds.
    pub pulse_time: [f32; 3],
}

impl RibbonPrm {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x60 {
            return Err(invalid_data("ribbon parameters are truncated"));
        }
        let three = |o: usize| [f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8)];
        Ok(Self {
            flags: u32_at(b, 0) as i32,
            blend: b[4],
            kind: b[5],
            fade_in: f32_at(b, 8),
            fade_out: f32_at(b, 0xC),
            anim_time: f32_at(b, 0x10),
            anim_split: f32_at(b, 0x14),
            width: three(0x18),
            width_lo: three(0x24),
            width_hi: three(0x30),
            width_time: three(0x3C),
            alpha: three(0x48),
            scroll: three(0x54),
        })
    }
}

impl RibbonAnim {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0xB4 {
            return Err(invalid_data("ribbon colours are truncated"));
        }
        let color = std::array::from_fn(|k| std::array::from_fn(|i| f32_at(b, k * 0x10 + i * 4)));
        let pulse = std::array::from_fn(|c| {
            std::array::from_fn(|k| {
                std::array::from_fn(|i| f32_at(b, 0x60 + c * 0x18 + k * 8 + i * 4))
            })
        });
        Ok(Self {
            color,
            pulse,
            pulse_time: [f32_at(b, 0xA8), f32_at(b, 0xAC), f32_at(b, 0xB0)],
        })
    }
}

/// `EftPart10Def`: a kind 10 emitter's first definition block (0x20E bytes); every value has three keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Part10Def {
    /// Particle size at birth / middle / death per key: minimum, and range.
    pub size: [[f32; 3]; 3],
    pub size_range: [[f32; 3]; 3],
    /// Start angle (turns).
    pub angle: [f32; 3],
    pub angle_range: [f32; 3],
    /// Spins (half turns).
    pub spin_x: [f32; 3],
    pub spin_x_range: [f32; 3],
    pub spin_y: [f32; 3],
    pub spin_y_range: [f32; 3],
    pub spin: [f32; 3],
    pub spin_range: [f32; 3],
    /// The burst's direction jitter (half turns).
    pub ang_a: [f32; 3],
    pub ang_a_range: [f32; 3],
    pub ang_b: [f32; 3],
    pub ang_b_range: [f32; 3],
    /// The start distance from the emitter (`dist` + `dist_base`), and the speed.
    pub dist: [f32; 3],
    pub dist_range: [f32; 3],
    pub dist_base: [f32; 3],
    pub dist_vel: [f32; 3],
    pub dist_vel_range: [f32; 3],
    /// The particle's offset (x, y).
    pub scale_x: [f32; 3],
    pub scale_y: [f32; 3],
    /// Seconds.
    pub life: [f32; 3],
    pub life_range: [f32; 3],
    pub wait: [f32; 3],
    pub wait_range: [f32; 3],
    /// The size pulse {from, to} per key, and its time.
    pub pulse: [[f32; 2]; 3],
    pub pulse_time: [f32; 3],
    pub twist_step: f32,
    /// The emitter's spin (flag 0x40): three keys, or one value with flag 0x20.
    pub spin_a: [f32; 3],
    pub spin_a_range: [f32; 3],
    pub spin_b: [f32; 3],
    pub spin_b_range: [f32; 3],
    pub scale: [f32; 3],
    pub scale_range: [f32; 3],
    /// Particles per ring side, per key.
    pub count: [f32; 3],
    pub spin_time: f32,
    pub spin_mid: f32,
    pub key_time: f32,
    pub key_mid: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub size_mid: f32,
    /// 1 aimed along the direction, 2 mirrored when spinning backwards, 4 random spin direction, 8 clipped,
    /// 0x10 random sheet frame, 0x20 / 0x40 emitter spin, 0x80 size pulse, 0x100 colour pulse, 0x200 keyed.
    pub flags: i32,
    pub cols: u8,
    pub rows: u8,
    pub tex_step: u8,
    pub layer: u8,
    pub ring_from: f32,
    pub ring_to: f32,
    pub yaw: f32,
    pub pitch: f32,
    /// 1: facing the camera.
    pub mode: u8,
    /// 1 no colour ramp, 2 a ramp on every other particle at random.
    pub color_mode: u8,
}

impl Part10Def {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x20E {
            return Err(invalid_data("kind 10 parameters are truncated"));
        }
        let f = |o: usize| f32_at(b, o);
        let three = |o: usize| [f(o), f(o + 4), f(o + 8)];
        let nine = |o: usize| std::array::from_fn(|k| three(o + k * 0xC));
        Ok(Self {
            size: nine(0),
            size_range: nine(0x24),
            angle: three(0x48),
            angle_range: three(0x54),
            spin_x: three(0x60),
            spin_x_range: three(0x6C),
            spin_y: three(0x78),
            spin_y_range: three(0x84),
            spin: three(0x90),
            spin_range: three(0x9C),
            ang_a: three(0xA8),
            ang_a_range: three(0xB4),
            ang_b: three(0xC0),
            ang_b_range: three(0xCC),
            dist: three(0xD8),
            dist_range: three(0xE4),
            dist_base: three(0xF0),
            dist_vel: three(0xFC),
            dist_vel_range: three(0x108),
            scale_x: three(0x114),
            scale_y: three(0x120),
            life: three(0x12C),
            life_range: three(0x138),
            wait: three(0x144),
            wait_range: three(0x150),
            pulse: std::array::from_fn(|k| [f(0x15C + k * 8), f(0x160 + k * 8)]),
            pulse_time: three(0x174),
            twist_step: f(0x180),
            spin_a: three(0x184),
            spin_a_range: three(0x190),
            spin_b: three(0x19C),
            spin_b_range: three(0x1A8),
            scale: three(0x1B4),
            scale_range: three(0x1C0),
            count: three(0x1CC),
            spin_time: f(0x1D8),
            spin_mid: f(0x1DC),
            key_time: f(0x1E0),
            key_mid: f(0x1E4),
            fade_in: f(0x1E8),
            fade_out: f(0x1EC),
            size_mid: f(0x1F0),
            flags: u32_at(b, 0x1F4) as i32,
            cols: b[0x1F8],
            rows: b[0x1F9],
            tex_step: b[0x1FA],
            layer: b[0x1FB],
            ring_from: f(0x1FC),
            ring_to: f(0x200),
            yaw: f(0x204),
            pitch: f(0x208),
            mode: b[0x20C],
            color_mode: b[0x20D],
        })
    }
}

/// `EftPart10Def2`: a kind 10 emitter's colours (its second definition block, 0x12C bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Part10Def2 {
    /// 0..255 per key.
    pub color: [[f32; 4]; 3],
    pub color_range: [[f32; 4]; 3],
    pub end_color: [[f32; 4]; 3],
    pub end_color_range: [[f32; 4]; 3],
    /// The colour multiplier pulse {from, to} per key, for red, green and blue, and its time.
    pub mul: [[[f32; 2]; 3]; 3],
    pub mul_time: [f32; 3],
    /// The part of a particle's life over which its colour ramps.
    pub fade0: [f32; 3],
    pub fade1: [f32; 3],
}

impl Part10Def2 {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x12C {
            return Err(invalid_data("kind 10 colours are truncated"));
        }
        let f = |o: usize| f32_at(b, o);
        let vecs = |o: usize| -> [[f32; 4]; 3] {
            std::array::from_fn(|k| std::array::from_fn(|i| f(o + k * 0x10 + i * 4)))
        };
        Ok(Self {
            color: vecs(0),
            color_range: vecs(0x30),
            end_color: vecs(0x60),
            end_color_range: vecs(0x90),
            mul: std::array::from_fn(|c| {
                std::array::from_fn(|k| [f(0xC0 + c * 0x18 + k * 8), f(0xC4 + c * 0x18 + k * 8)])
            }),
            mul_time: [f(0x108), f(0x10C), f(0x110)],
            fade0: [f(0x114), f(0x118), f(0x11C)],
            fade1: [f(0x120), f(0x124), f(0x128)],
        })
    }
}

/// `EftLinkDef`: a kind 15 sprite chain's first definition block (0x1A4 bytes); every animated value has three
/// keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkDef {
    /// The sprites' turns (turns).
    pub rot_x: [f32; 3],
    pub rot_x_range: [f32; 3],
    pub rot_z: [f32; 3],
    pub rot_z_range: [f32; 3],
    /// The direction of the drift offset (turns), and its distance.
    pub dir_ang: [f32; 3],
    pub dir_ang_range: [f32; 3],
    /// Half turns per frame.
    pub spin: [f32; 3],
    pub spin_range: [f32; 3],
    /// Twist per sprite along the chain (turns).
    pub twist: [f32; 3],
    pub ofs_x: [f32; 3],
    pub ofs_y: [f32; 3],
    /// The random push along the chain.
    pub along_min: [f32; 3],
    pub along_max: [f32; 3],
    pub dist: [f32; 3],
    pub dist_range: [f32; 3],
    pub dist_base: [f32; 3],
    /// Size at birth / middle / death per key: minimum, and range.
    pub size: [[f32; 3]; 3],
    pub size_range: [[f32; 3]; 3],
    /// Seconds.
    pub life: [f32; 3],
    pub life_range: [f32; 3],
    pub wait: [f32; 3],
    pub wait_range: [f32; 3],
    pub pulse: [[f32; 2]; 3],
    pub pulse_time: [f32; 3],
    pub key_time: f32,
    pub key_split: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub size_split: f32,
    pub grow_split: f32,
    /// Seconds.
    pub grow_time: f32,
    /// Distance between two sprites, per key of the growth.
    pub spacing: [f32; 3],
    pub scale: [f32; 3],
    pub scale_range: [f32; 3],
    pub cols: u8,
    pub rows: u8,
    pub tex_step: u8,
    pub layer: u8,
    /// 2 no depth, 4 clipped, 8 mirrored when spinning backwards, 0x10 facing the camera, 0x20 / 0x40 colour ramp,
    /// 0x80 random sheet frame, 0x100 random spin sign, 0x200 grows, 0x400 keyed, 0x800 size pulse, 0x1000 colour
    /// pulse, 0x2000 the drift is not scaled.
    pub flags: i32,
}

impl LinkDef {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x1A4 {
            return Err(invalid_data("kind 15 parameters are truncated"));
        }
        let f = |o: usize| f32_at(b, o);
        let three = |o: usize| [f(o), f(o + 4), f(o + 8)];
        let nine = |o: usize| std::array::from_fn(|k| three(o + k * 0xC));
        Ok(Self {
            rot_x: three(0),
            rot_x_range: three(0xC),
            rot_z: three(0x18),
            rot_z_range: three(0x24),
            dir_ang: three(0x30),
            dir_ang_range: three(0x3C),
            spin: three(0x48),
            spin_range: three(0x54),
            twist: three(0x60),
            ofs_x: three(0x6C),
            ofs_y: three(0x78),
            along_min: three(0x84),
            along_max: three(0x90),
            dist: three(0x9C),
            dist_range: three(0xA8),
            dist_base: three(0xB4),
            size: nine(0xC0),
            size_range: nine(0xE4),
            life: three(0x108),
            life_range: three(0x114),
            wait: three(0x120),
            wait_range: three(0x12C),
            pulse: std::array::from_fn(|k| [f(0x138 + k * 8), f(0x13C + k * 8)]),
            pulse_time: three(0x150),
            key_time: f(0x15C),
            key_split: f(0x160),
            fade_in: f(0x164),
            fade_out: f(0x168),
            size_split: f(0x16C),
            grow_split: f(0x170),
            grow_time: f(0x174),
            spacing: three(0x178),
            scale: three(0x184),
            scale_range: three(0x190),
            cols: b[0x19C],
            rows: b[0x19D],
            tex_step: b[0x19E],
            layer: b[0x19F],
            flags: u32_at(b, 0x1A0) as i32,
        })
    }
}

/// `EftLinkDef2`: a kind 15 chain's colours, in the layout of kind 10's.
pub type LinkDef2 = Part10Def2;

/// `EftRaysKey`: one key of a ray.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RaysKey {
    pub height: f32,
    pub width: f32,
    pub color: [u8; 4],
}

/// `EftRaysRayDef` (0x78 bytes): one ray of a kind 2 part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RayDef {
    /// Key 2's height is also the reference height of the texture mapping.
    pub key: [RaysKey; 3],
    /// The width flickers between x + y * rand and z + w * rand.
    pub len: [f32; 4],
    /// Seconds.
    pub flick_time: f32,
    /// The colour factor pulses between `col_a` and `col_b` (its w is the pulse time in seconds).
    pub col_a: [f32; 3],
    pub col_b: [f32; 4],
    /// Degrees.
    pub angle: f32,
    /// Radians per frame.
    pub spin: f32,
    /// Seconds for key 0 -> 1 -> 2, and the fraction spent on the first leg.
    pub key_time: f32,
    pub key_split: f32,
    /// Frames before the ray appears.
    pub delay: u16,
    /// Ordering-table layer.
    pub blend: i8,
}

/// `EftRaysDef`: a kind 2 part's rays (its first definition block, 0x4B8 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RaysDef {
    pub rays: [RayDef; 10],
    pub count: i32,
    /// 1 key animation, 2 width flicker, 4 colour pulse.
    pub flags: i32,
}

impl RaysDef {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x4B8 {
            return Err(invalid_data("ray parameters are truncated"));
        }
        let rays = std::array::from_fn(|r| {
            let o = r * 0x78;
            let f = |x: usize| f32_at(b, o + x);
            RayDef {
                key: std::array::from_fn(|k| RaysKey {
                    height: f(k * 0x10),
                    width: f(k * 0x10 + 4),
                    color: [
                        b[o + k * 0x10 + 0xC],
                        b[o + k * 0x10 + 0xD],
                        b[o + k * 0x10 + 0xE],
                        b[o + k * 0x10 + 0xF],
                    ],
                }),
                len: [f(0x30), f(0x34), f(0x38), f(0x3C)],
                flick_time: f(0x40),
                col_a: [f(0x44), f(0x48), f(0x4C)],
                col_b: [f(0x50), f(0x54), f(0x58), f(0x5C)],
                angle: f(0x60),
                spin: f(0x64),
                key_time: f(0x68),
                key_split: f(0x6C),
                delay: u16::from_le_bytes([b[o + 0x70], b[o + 0x71]]),
                blend: b[o + 0x72] as i8,
            }
        });
        Ok(Self {
            rays,
            count: u32_at(b, 0x4B0) as i32,
            flags: u32_at(b, 0x4B4) as i32,
        })
    }
}

/// `EftArcParam`: a chain's parameter block (kind 18's first definition block, 0x1E8 bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainPrm {
    /// 1 keyed, 2 clipped polygons, 4 width pulse, 8 colour pulse, 0x10 turned onto the emitter's direction.
    pub flags: u32,
    /// 0 / 2 the head fades, 1 the strand crawls, 2 a random yaw per strand.
    pub kind: u8,
    /// 0 every strand drifts in colour, 1 every other one at random.
    pub offset_mode: u8,
    /// Ordering-table layer (>= 2: the second GS context).
    pub layer: u8,
    /// Strands per burst, and a random part.
    pub count_base: u8,
    pub count_range: u8,
    /// Seconds of the key animation, and the fraction where key 1 is reached.
    pub key_time: f32,
    pub key_mid: f32,
    /// The 39 rows of three keys (`r0..r38`).
    pub rows: [[f32; 3]; 39],
}

impl ChainPrm {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x1E8 {
            return Err(invalid_data("chain parameters are truncated"));
        }
        Ok(Self {
            flags: u32_at(b, 0),
            kind: b[4],
            offset_mode: b[5],
            layer: b[6],
            count_base: b[8],
            count_range: b[9],
            key_time: f32_at(b, 0xC),
            key_mid: f32_at(b, 0x10),
            rows: std::array::from_fn(|r| {
                std::array::from_fn(|k| f32_at(b, 0x14 + r * 0xC + k * 4))
            }),
        })
    }
}

/// `EftArcKeys`: a chain's colours and colour pulse at the three keys (its second definition block).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChainKeys {
    pub col_base: [[f32; 4]; 3],
    pub col_range: [[f32; 4]; 3],
    pub col2_base: [[f32; 4]; 3],
    pub col2_range: [[f32; 4]; 3],
    /// The colour pulse's factors {from, to} per key, for red, green and blue.
    pub mul: [[[f32; 2]; 3]; 3],
    /// Seconds.
    pub mul_time: [f32; 3],
}

impl ChainKeys {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x114 {
            return Err(invalid_data("chain keys are truncated"));
        }
        let vecs = |o: usize| -> [[f32; 4]; 3] {
            std::array::from_fn(|k| std::array::from_fn(|i| f32_at(b, o + k * 0x10 + i * 4)))
        };
        Ok(Self {
            col_base: vecs(0),
            col_range: vecs(0x30),
            col2_base: vecs(0x60),
            col2_range: vecs(0x90),
            mul: std::array::from_fn(|c| {
                std::array::from_fn(|k| {
                    std::array::from_fn(|i| f32_at(b, 0xC0 + c * 0x18 + k * 8 + i * 4))
                })
            }),
            mul_time: [f32_at(b, 0x108), f32_at(b, 0x10C), f32_at(b, 0x110)],
        })
    }
}

/// `EftPtclDef`: a particle emitter's first definition block (0x2B0 bytes); every animated value has three keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PtclDef {
    /// 1 keyed, 2 no x16 / clipped draws, 4 sheet, 8 width / height pulse, 0x10 random pulse phase, 0x20 colour
    /// pulse, 0x40 random spin direction, 0x80 quads start at the particle, 0x100 fixed axis.
    pub flags: i32,
    pub reverse: u8,
    /// 0 quads along the axis, 1 squares, 2 sprites.
    pub shape: u8,
    pub flip: u8,
    pub centred: u8,
    pub origin_kind: u8,
    pub accel_kind: u8,
    pub layer: u8,
    pub cols: u8,
    pub rows: u8,
    pub anim_time: f32,
    pub key_time: f32,
    pub key_split: f32,
    pub count: [i16; 3],
    pub count_range: [i16; 3],
    /// Emitter angles and spins (half turns): x, x range, spin x, spin x range, then y, then z.
    pub ang: [f32; 12],
    raw: Vec<u8>,
}

/// `EftPtclDef2`: colours and colour pulse at the three keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PtclDef2 {
    pub color: [[f32; 4]; 3],
    pub color_range: [[f32; 4]; 3],
    pub color_end: [[f32; 4]; 3],
    pub color_end_range: [[f32; 4]; 3],
    /// Low and high factor per key, for red, green and blue.
    pub pulse: [[[f32; 2]; 3]; 3],
    pub pulse_time: [f32; 3],
}

impl PtclDef {
    /// The float at `o` for key `k` of a three-key value.
    pub fn key(&self, o: usize, k: usize) -> f32 {
        f32_at(&self.raw, o + k * 4)
    }

    /// The float at `o` for key `k` and stage `s` of a [key][stage] table.
    pub fn stage(&self, o: usize, k: usize, s: usize) -> f32 {
        f32_at(&self.raw, o + k * 12 + s * 4)
    }

    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x2B0 {
            return Err(invalid_data("particle definition is truncated"));
        }
        let s16 = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
        Ok(Self {
            flags: u32_at(b, 0) as i32,
            reverse: b[4],
            shape: b[5],
            flip: b[6],
            centred: b[7],
            origin_kind: b[8],
            accel_kind: b[9],
            layer: b[0xA],
            cols: b[0xE],
            rows: b[0xF],
            anim_time: f32_at(b, 0x10),
            key_time: f32_at(b, 0x14),
            key_split: f32_at(b, 0x18),
            count: [s16(0x1C), s16(0x1E), s16(0x20)],
            count_range: [s16(0x22), s16(0x24), s16(0x26)],
            ang: std::array::from_fn(|i| f32_at(b, 0x280 + i * 4)),
            raw: b[..0x2B0].to_vec(),
        })
    }
}

/// `EftBillDef` (`EftLineDef`): a sprite's definition block (0x90 bytes); the animated values have three keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BillDef {
    /// 1 key animation, 2 depth-sorted / clipped quads, 4 size pulse, 8 sheet animation, 0x10 colour pulse.
    pub flags: i32,
    /// The ordering-table layer (2 and 3: layers 0 and 1 in GS context 2).
    pub layer: u8,
    /// 0 a camera-facing sprite, 1 a quad along the direction.
    pub shape: u8,
    pub cols: u8,
    pub rows: u8,
    /// Seconds per sheet frame.
    pub frame_time: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub key_time: f32,
    pub key_split: f32,
    /// Half turns per frame.
    pub spin: [f32; 3],
    /// Width (the quad's half width) and height (its length).
    pub w: [f32; 3],
    pub h: [f32; 3],
    /// Size pulses: low and high factor, seconds per half period.
    pub pulse_w: [[f32; 3]; 3],
    pub pulse_h: [[f32; 3]; 3],
    pub spin_range: f32,
    pub angle_range: f32,
}

/// `EftLineDef2`: colours and colour pulse at the three keys (0x84 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BillDef2 {
    pub color: [[f32; 4]; 3],
    /// Low and high factor per key, for red, green and blue.
    pub pulse: [[[f32; 2]; 3]; 3],
    pub pulse_time: [f32; 3],
}

impl BillDef {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x90 {
            return Err(invalid_data("sprite definition is truncated"));
        }
        let k3 = |o: usize| -> [f32; 3] { std::array::from_fn(|k| f32_at(b, o + k * 4)) };
        Ok(Self {
            flags: u32_at(b, 0) as i32,
            layer: b[4],
            shape: b[5],
            cols: b[6],
            rows: b[7],
            frame_time: f32_at(b, 8),
            fade_in: f32_at(b, 0xC),
            fade_out: f32_at(b, 0x10),
            key_time: f32_at(b, 0x14),
            key_split: f32_at(b, 0x18),
            spin: k3(0x1C),
            w: k3(0x28),
            h: k3(0x34),
            pulse_w: [k3(0x40), k3(0x4C), k3(0x58)],
            pulse_h: [k3(0x64), k3(0x70), k3(0x7C)],
            spin_range: f32_at(b, 0x88),
            angle_range: f32_at(b, 0x8C),
        })
    }
}

impl BillDef2 {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x84 {
            return Err(invalid_data("sprite colours are truncated"));
        }
        Ok(Self {
            color: std::array::from_fn(|k| std::array::from_fn(|i| f32_at(b, k * 0x10 + i * 4))),
            pulse: std::array::from_fn(|c| {
                std::array::from_fn(|k| {
                    std::array::from_fn(|i| f32_at(b, 0x30 + c * 0x18 + k * 8 + i * 4))
                })
            }),
            pulse_time: [f32_at(b, 0x78), f32_at(b, 0x7C), f32_at(b, 0x80)],
        })
    }
}

/// `EftQuadDef` (`EftPart9Def`): a quad emitter's definition block (0x244 bytes); the animated values have three
/// keys, each `[key][component]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuadDef {
    /// Size factor per quad: start, middle, end (`a`) and their ranges (`b`).
    pub size_mul: [[f32; 3]; 3],
    pub size_mul_range: [[f32; 3]; 3],
    /// Speed factor per quad: start, middle, end (`c`) and their ranges (`d`).
    pub speed_mul: [[f32; 3]; 3],
    pub speed_mul_range: [[f32; 3]; 3],
    /// Start angles (turns) and ranges (`e`, `f`).
    pub rot: [[f32; 3]; 3],
    pub rot_range: [[f32; 3]; 3],
    /// Emitter angle step per emitted quad, half turns (`g`).
    pub spin: [[f32; 3]; 3],
    /// Spin per frame, half turns, and ranges (`i`, `j`).
    pub rot_vel: [[f32; 3]; 3],
    pub rot_vel_range: [[f32; 3]; 3],
    /// Size oscillation low / high (`k`) and seconds per half period (`l`).
    pub size_osc: [[f32; 2]; 3],
    pub size_osc_time: [f32; 3],
    /// Direction around the emitter's axis, half turns, and range (`m`, `n`).
    pub arc: [f32; 3],
    pub arc_range: [f32; 3],
    /// Start offset: x, x range, y, y range, z, z range (`o` .. `t`).
    pub offset: [[f32; 6]; 3],
    /// Life in seconds and range (`u`, `v`), speed (`w`).
    pub life: [f32; 3],
    pub life_range: [f32; 3],
    pub speed: [f32; 3],
    pub key_time: f32,
    pub key_split: f32,
    pub fade_in_end: f32,
    pub fade_out_start: f32,
    /// Fraction of a quad's life after which the second speed / size step applies.
    pub speed_split: f32,
    pub base_size: f32,
    pub base_size_range: f32,
    pub base_speed: f32,
    pub base_speed_range: f32,
    /// 1 reverse spin, 2 random spin direction, 4 mirror U while spinning back, 8 size oscillation, 0x10 keyed,
    /// 0x20 clipped draws, 0x40 four quarter quads, 0x80 colour oscillation, 0x100 / 0x200 / 0x400 random sign of
    /// the start angle x / y / z.
    pub flags: i32,
    pub interval: [u8; 3],
    pub count: [u8; 3],
    pub layer: u8,
    pub cols: u8,
    pub rows: u8,
    pub frame_step: u8,
    /// 1 static colour, 2 static colour on a coin flip.
    pub color_mode: u8,
}

/// `EftQuadDef2`: colours, the colour ramp and the colour oscillation at the three keys (0x12C bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuadDef2 {
    pub color: [[f32; 4]; 3],
    pub color_range: [[f32; 4]; 3],
    pub end_color: [[f32; 4]; 3],
    pub end_color_range: [[f32; 4]; 3],
    /// The part of the life the colour ramps over.
    pub ramp_start: [f32; 3],
    pub ramp_end: [f32; 3],
    /// Colour multiplier oscillation per channel: [channel][key] = {low, high}.
    pub osc: [[[f32; 2]; 3]; 3],
    /// Seconds per half period.
    pub osc_time: [f32; 3],
}

impl QuadDef {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x243 {
            return Err(invalid_data("quad definition is truncated"));
        }
        let v3 = |o: usize| -> [[f32; 3]; 3] {
            std::array::from_fn(|k| std::array::from_fn(|i| f32_at(b, o + k * 12 + i * 4)))
        };
        let k3 = |o: usize| -> [f32; 3] { std::array::from_fn(|k| f32_at(b, o + k * 4)) };
        Ok(Self {
            size_mul: v3(0),
            size_mul_range: v3(0x24),
            speed_mul: v3(0x48),
            speed_mul_range: v3(0x6C),
            rot: v3(0x90),
            rot_range: v3(0xB4),
            spin: v3(0xD8),
            rot_vel: v3(0x120),
            rot_vel_range: v3(0x144),
            size_osc: std::array::from_fn(|k| [f32_at(b, 0x168 + k * 8), f32_at(b, 0x16C + k * 8)]),
            size_osc_time: k3(0x180),
            arc: k3(0x18C),
            arc_range: k3(0x198),
            offset: std::array::from_fn(|k| {
                std::array::from_fn(|i| f32_at(b, 0x1A4 + i * 12 + k * 4))
            }),
            life: k3(0x1EC),
            life_range: k3(0x1F8),
            speed: k3(0x204),
            key_time: f32_at(b, 0x210),
            key_split: f32_at(b, 0x214),
            fade_in_end: f32_at(b, 0x218),
            fade_out_start: f32_at(b, 0x21C),
            speed_split: f32_at(b, 0x220),
            base_size: f32_at(b, 0x224),
            base_size_range: f32_at(b, 0x228),
            base_speed: f32_at(b, 0x22C),
            base_speed_range: f32_at(b, 0x230),
            flags: u32_at(b, 0x234) as i32,
            interval: [b[0x238], b[0x239], b[0x23A]],
            count: [b[0x23B], b[0x23C], b[0x23D]],
            layer: b[0x23E],
            cols: b[0x23F],
            rows: b[0x240],
            frame_step: b[0x241],
            color_mode: b[0x242],
        })
    }
}

impl QuadDef2 {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x12C {
            return Err(invalid_data("quad colours are truncated"));
        }
        let v4 = |o: usize| -> [[f32; 4]; 3] {
            std::array::from_fn(|k| std::array::from_fn(|i| f32_at(b, o + k * 0x10 + i * 4)))
        };
        let k3 = |o: usize| -> [f32; 3] { std::array::from_fn(|k| f32_at(b, o + k * 4)) };
        Ok(Self {
            color: v4(0),
            color_range: v4(0x30),
            end_color: v4(0x60),
            end_color_range: v4(0x90),
            ramp_start: k3(0xC0),
            ramp_end: k3(0xCC),
            osc: std::array::from_fn(|c| {
                std::array::from_fn(|k| {
                    std::array::from_fn(|i| f32_at(b, 0xD8 + c * 0x18 + k * 8 + i * 4))
                })
            }),
            osc_time: k3(0x120),
        })
    }
}

impl PtclDef2 {
    pub fn parse(b: &[u8]) -> io::Result<Self> {
        if b.len() < 0x114 {
            return Err(invalid_data("particle colours are truncated"));
        }
        let v4 = |o: usize| -> [[f32; 4]; 3] {
            std::array::from_fn(|k| std::array::from_fn(|i| f32_at(b, o + k * 0x10 + i * 4)))
        };
        Ok(Self {
            color: v4(0),
            color_range: v4(0x30),
            color_end: v4(0x60),
            color_end_range: v4(0x90),
            pulse: std::array::from_fn(|c| {
                std::array::from_fn(|k| {
                    std::array::from_fn(|i| f32_at(b, 0xC0 + c * 0x18 + k * 8 + i * 4))
                })
            }),
            pulse_time: [f32_at(b, 0x108), f32_at(b, 0x10C), f32_at(b, 0x110)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tech_places_reads_the_stage_rows_of_header_word_3() {
        // Header word 3 points at the table; stage 1's five records follow stage 0's.
        let mut pack = vec![0u8; 0x10 + 10 * 0x10];
        pack[12..16].copy_from_slice(&0x10u32.to_le_bytes());
        let rec = 0x10 + 5 * 0x10 + 2 * 0x10;
        for (k, v) in [1.0f32, -160.0, 3.0, 90.0].iter().enumerate() {
            pack[rec + k * 4..rec + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let places = tech_places(&pack, 1).unwrap();
        assert_eq!(places[2], [1.0, -160.0, 3.0, 90.0]);
        assert_eq!(places[0], [0.0; 4]);
        assert!(tech_places(&pack, 2).is_none());
    }
}
