//! Clip animation helpers of the menus (`FlashAnim_*`, src/ui/menu_util_1.c) and the guide's mouth movement
//! (`LipSync_*`, src/ui/menu_util_2.c).

use crate::flash::{ClipRef, Flash, PROP_REPEAT, PROP_VISIBLE, Uv};
use crate::rng::GameRandom;
use crate::title::TitleSound;

/// `VOICE_LANG2_OFFSET`: with save flag bit 0 (a fresh save has it) menu voices take the second set's files.
pub(crate) const VOICE_LANG2_OFFSET: u32 = 0x55C;

/// `Voice_PlayWithSubtitle(pack, base, line)`: the voice file and the line's mouth keys (pack word `line * 2 + 2`)
/// of the second voice set, or the first's (`line`, word `line * 2 + 1`).
pub fn play_with_subtitle(
    lips: &[u8],
    base: u32,
    line: i32,
    lip: &mut LipSync,
    sounds: &mut Vec<TitleSound>,
) {
    let alt = crate::sound::options().alt_voice;
    let offset = if alt { VOICE_LANG2_OFFSET } else { 0 };
    sounds.push(TitleSound::Voice(base + line as u32 + offset));
    let i = line as usize * 2 + if alt { 2 } else { 1 };
    let ofs = lips
        .get(i * 4..i * 4 + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize & !3);
    if let Some(data) = ofs.and_then(|ofs| lips.get(ofs..)) {
        lip.start(data);
    }
}

/// `ADXT_STAT_PLAYING` and `ADXT_STAT_PLAYEND` of `Voice_GetStat`.
pub const VOICE_PLAYING: i32 = 3;
pub const VOICE_PLAYEND: i32 = 5;

/// `FlashAnim_Blink`: hidden except for frames `frame`, `frame + 1`, `frame`; the next blink comes after 136..300 calls.
pub fn blink(flash: &mut Flash, r: ClipRef, timer: &mut i32, frame: i32, random: &mut GameRandom) {
    if !r.found() {
        return;
    }
    match *timer {
        0..36 => flash.clip_set_flags(r, PROP_VISIBLE, false),
        36..40 | 44..48 => {
            flash.clip_set_flags(r, PROP_VISIBLE, true);
            flash.clip_set_tex(r, frame);
        }
        40..44 => {
            flash.clip_set_flags(r, PROP_VISIBLE, true);
            flash.clip_set_tex(r, frame + 1);
        }
        _ => flash.clip_set_flags(r, PROP_VISIBLE, false),
    }
    *timer += 1;
    if *timer >= 136 && random.range(12) >= 8 {
        *timer = 0;
    }
    if *timer > 300 {
        *timer = 0;
    }
}

/// `FlashAnim_Talk`: while the voice's mouth is open, every sixth call picks one of three mouth shapes.
pub fn talk(
    flash: &mut Flash,
    r: ClipRef,
    timer: &mut i32,
    frame: i32,
    lip: &mut LipSync,
    voice_stat: i32,
    random: &mut GameRandom,
) {
    if !r.found() {
        return;
    }
    lip.update(voice_stat);
    if lip.is_open() {
        *timer += 1;
        if *timer % 6 == 0 {
            match random.range(3) {
                0 => {
                    flash.clip_set_flags(r, PROP_VISIBLE, true);
                    flash.clip_set_tex(r, frame);
                }
                1 => {
                    flash.clip_set_flags(r, PROP_VISIBLE, true);
                    flash.clip_set_tex(r, frame + 1);
                }
                _ => {
                    flash.clip_set_flags(r, PROP_VISIBLE, false);
                    flash.clip_set_tex(r, frame);
                }
            }
        }
    } else {
        show_next(flash, r, frame);
    }
}

/// `FlashAnim_ShowNext2`.
pub fn show_next(flash: &mut Flash, r: ClipRef, frame: i32) {
    if r.found() {
        flash.clip_set_flags(r, PROP_VISIBLE, true);
        flash.clip_set_tex(r, frame + 1);
    }
}

/// `FlashAnim_Sheet`: every `period` calls the next of `cols * rows` cells; `uv.u1` / `uv.v1` are the cell size.
#[allow(clippy::too_many_arguments)]
pub fn sheet(
    flash: &mut Flash,
    r: ClipRef,
    timer: &mut i32,
    frame: &mut i32,
    uv: Uv,
    cols: i32,
    rows: i32,
    period: i32,
) {
    if !r.found() {
        return;
    }
    *timer += 1;
    if *timer % period == 0 {
        *timer = 0;
        *frame += 1;
        if *frame >= cols * rows {
            *frame = 0;
        }
    }
    let (col, row) = (*frame % cols, *frame / cols);
    flash.clip_set_uv(
        r,
        Uv {
            u0: uv.u0 + uv.u1 * col,
            v0: uv.v0 + uv.v1 * row,
            u1: uv.u1 + uv.u1 * col,
            v1: uv.v1 + uv.v1 * row,
            tex: 0,
        },
    );
}

/// `FlashAnim_Scroll` along x: the rectangle moves by `*x`, which advances by `dx` and wraps at `uv.u1`.
pub fn scroll(flash: &mut Flash, r: ClipRef, uv: Uv, x: &mut f32, dx: f32) {
    if !r.found() {
        return;
    }
    let cell = Uv {
        u0: uv.u0 + *x as i32,
        u1: uv.u1 + *x as i32,
        ..uv
    };
    *x += dx;
    if dx < 0.0 {
        if *x < 0.0 {
            *x += uv.u1 as f32;
        }
    } else if uv.u1 as f32 <= *x {
        *x -= uv.u1 as f32;
    }
    flash.clip_set_flags(r, PROP_REPEAT, true);
    flash.clip_set_uv(r, cell);
}

/// `FlashAnim_Scroll` along y: the rectangle moves by `*y`, which advances by `dy` and wraps at `uv.v1`.
pub fn scroll_y(flash: &mut Flash, r: ClipRef, uv: Uv, y: &mut f32, dy: f32) {
    if !r.found() {
        return;
    }
    let cell = Uv {
        v0: uv.v0 + *y as i32,
        v1: uv.v1 + *y as i32,
        ..uv
    };
    *y += dy;
    if dy < 0.0 {
        if *y < 0.0 {
            *y += uv.v1 as f32;
        }
    } else if uv.v1 as f32 <= *y {
        *y -= uv.v1 as f32;
    }
    flash.clip_set_flags(r, PROP_REPEAT, true);
    flash.clip_set_uv(r, cell);
}

/// `Num_Draw(flash, fmt, first 0, count, value, w, h, mode)`: one clip per digit (`name(n)`, n = 0 the last), on a
/// sheet of four `w * h` cells a row; mode 0 hides leading zeros (the last digit always shows).
pub fn num_draw(
    flash: &mut Flash,
    name: impl Fn(i32) -> String,
    count: i32,
    value: i32,
    (w, h): (i32, i32),
    mode: i32,
) {
    let mut value = value.abs();
    let mut started = false;
    for i in (0..count).rev() {
        let clip = flash.find(None, &name(i));
        let d = value / 10i32.pow(i as u32);
        let digit = if d == 0 && started {
            0
        } else if d != 0 {
            value -= d * 10i32.pow(i as u32);
            started = true;
            d
        } else if mode != 0 || i == 0 {
            0
        } else {
            flash.clip_set_flags(clip, PROP_VISIBLE, false);
            continue;
        };
        flash.clip_set_flags(clip, PROP_VISIBLE, true);
        flash.clip_set_uv(
            clip,
            Uv {
                u0: w * (digit % 4),
                v0: h * (digit / 4),
                u1: w * (digit % 4) + w,
                v1: h * (digit / 4) + h,
                tex: 0,
            },
        );
    }
}

/// `Num_DrawChild(flash, parent, fmt, first 0, count, value, w, h, mode, parentFmt)`: as `num_draw`, each digit's
/// clip found under a parent, `name(n)` giving (parent, clip) for digit `n`; mode 2 shows cell 10 (no digit) on
/// every clip.
#[allow(clippy::too_many_arguments)]
pub fn num_draw_child(
    flash: &mut Flash,
    name: impl Fn(i32) -> (String, String),
    count: i32,
    value: i32,
    (w, h): (i32, i32),
    mode: i32,
) {
    let mut value = value.abs();
    let mut started = false;
    for i in (0..count).rev() {
        let (parent, child) = name(i);
        let clip = flash.find(Some(&parent), &child);
        let digit = if mode != 2 {
            let d = value / 10i32.pow(i as u32);
            if d == 0 && started {
                0
            } else if d != 0 {
                value -= d * 10i32.pow(i as u32);
                started = true;
                d
            } else if mode != 0 || i == 0 {
                0
            } else {
                flash.clip_set_flags(clip, PROP_VISIBLE, false);
                continue;
            }
        } else {
            10
        };
        flash.clip_set_flags(clip, PROP_VISIBLE, true);
        flash.clip_set_uv(
            clip,
            Uv {
                u0: w * (digit % 4),
                v0: h * (digit / 4),
                u1: w * (digit % 4) + w,
                v1: h * (digit / 4) + h,
                tex: 0,
            },
        );
    }
}

/// `gLipSync` with `gLipSyncOpen` / `gLipSyncWaiting`: (start frame, open) keys of a voice line.
#[derive(Clone, Debug, Default)]
pub struct LipSync {
    active: bool,
    index: usize,
    keys: Vec<(u16, u16)>,
    time: f32,
    length: u16,
    open: bool,
    waiting: bool,
}

impl LipSync {
    /// `LipSync_Clear`.
    pub fn clear(&mut self) {
        let (open, waiting) = (self.open, self.waiting);
        *self = Self::default();
        (self.open, self.waiting) = (open, waiting);
    }

    /// `LipSync_Start`: count at +8, four-byte keys from +0x10; the last key's frame is the length.
    pub fn start(&mut self, data: &[u8]) {
        *self = Self::default();
        let count = data
            .get(8..12)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()).max(0) as usize)
            .unwrap_or(0);
        self.keys = data
            .get(0x10..)
            .unwrap_or(&[])
            .chunks_exact(4)
            .take(count)
            .map(|k| {
                (
                    u16::from_le_bytes([k[0], k[1]]),
                    u16::from_le_bytes([k[2], k[3]]),
                )
            })
            .collect();
        self.length = self.keys.last().map_or(0, |k| k.0);
        self.active = true;
        self.waiting = true;
    }

    /// `LipSync_Update`: waits for the voice to play, then takes the open state of the key the time falls in.
    pub fn update(&mut self, voice_stat: i32) {
        self.open = false;
        if self.waiting {
            if voice_stat != VOICE_PLAYING {
                return;
            }
            self.waiting = false;
        }
        if !self.active {
            return;
        }
        let found = (self.index..self.keys.len().saturating_sub(1)).find(|&i| {
            f32::from(self.keys[i].0) <= self.time && self.time < f32::from(self.keys[i + 1].0)
        });
        let Some(found) = found.filter(|_| self.time <= f32::from(self.length)) else {
            let waiting = self.waiting;
            *self = Self::default();
            self.waiting = waiting;
            return;
        };
        self.index = found;
        self.open = self.keys[found].1 != 0;
        self.time += 1.0;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

#[cfg(test)]
mod tests {
    use super::{LipSync, VOICE_PLAYING};

    #[test]
    fn mouth_follows_keys_once_the_voice_plays() {
        let mut data = vec![0u8; 0x10];
        data[8] = 3;
        for (t, open) in [(0u16, 1u16), (2, 0), (4, 0)] {
            data.extend(t.to_le_bytes());
            data.extend(open.to_le_bytes());
        }
        let mut lip = LipSync::default();
        lip.start(&data);
        lip.update(2);
        assert!(!lip.is_open());
        let open: Vec<bool> = (0..6)
            .map(|_| {
                lip.update(VOICE_PLAYING);
                lip.is_open()
            })
            .collect();
        assert_eq!(open, [true, true, false, false, false, false]);
    }
}
