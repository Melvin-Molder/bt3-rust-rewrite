use crate::rng::{GameRandom, LibcRandom};

pub const ITEM_COUNT: usize = 30;
const SPRITE_COUNT: usize = 16;

const SPR_DRAW: i32 = 1;
const SPR_END: i32 = 2;
const SPR_UNTEXTURED: i32 = 4;
const SPR_CLEAR: i32 = 5;

const FLAG_TAKEN: i32 = 1;
const FLAG_PRESSED: i32 = 2;

// The original's float literals are truncated to these bit patterns (src/sys/loading.c).
const PI: f32 = f32::from_bits(0x4049_0FDA);
const TAU: f32 = f32::from_bits(0x40C9_0FDA);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Sprite {
    flags: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    u0: i32,
    v0: i32,
    u1: i32,
    v1: i32,
    tex: i32,
    r: i32,
    g: i32,
    b: i32,
    a: i32,
}

const fn spr(
    flags: i32,
    (x0, y0, x1, y1): (i32, i32, i32, i32),
    (u0, v0, u1, v1): (i32, i32, i32, i32),
    tex: i32,
    (r, g, b, a): (i32, i32, i32, i32),
) -> Sprite {
    Sprite {
        flags,
        x0,
        y0,
        x1,
        y1,
        u0,
        v0,
        u1,
        v1,
        tex,
        r,
        g,
        b,
        a,
    }
}

const END: Sprite = spr(SPR_END, (0, 0, 0, 0), (0, 0, 0, 0), 0, (0, 0, 0, 0));
const GREY: (i32, i32, i32, i32) = (0x80, 0x80, 0x80, 0x80);

/// One GS sprite as `Sprite_DrawList` emits it, in screen pixels and texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub texture: Option<usize>,
    pub screen: [i32; 4],
    pub uv: [i32; 4],
    pub rgba: [u8; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Item {
    flags: i32,
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Pulse {
    rect: [i32; 4],
    size: f32,
    angle: f32,
    amp: f32,
    speed: f32,
}

/// Fade slot 0 (black), as started by `Load_RunBlocking`.
#[derive(Clone, Copy, Debug)]
struct Fade {
    active: bool,
    done: bool,
    alpha: f32,
    ramp_value: f32,
    ramp_frames: f32,
    ramp_step: f32,
}

impl Fade {
    fn start_in(seconds: f32) -> Self {
        let frames = seconds * 30.0;
        let mut fade = Self {
            active: true,
            done: false,
            alpha: 128.0,
            ramp_value: 0.0,
            ramp_frames: frames,
            ramp_step: 1.0 / frames,
        };
        fade.update();
        fade
    }

    fn update(&mut self) {
        if !self.active {
            return;
        }
        self.alpha = 128.0 + -128.0 * self.ramp_value;
        if self.done {
            if self.alpha as i32 == 0 {
                self.active = false;
            }
        } else if self.step_ramp() {
            self.done = true;
        }
    }

    fn step_ramp(&mut self) -> bool {
        self.ramp_frames -= 1.0;
        if self.ramp_frames <= 0.0001 {
            self.ramp_value = 1.0;
            return true;
        }
        self.ramp_value = (self.ramp_value + self.ramp_step).min(1.0);
        false
    }

    fn quad(&self) -> Option<Quad> {
        self.active.then(|| Quad {
            texture: None,
            screen: [0, 0, 512, 448],
            uv: [0; 4],
            rgba: [0, 0, 0, self.alpha as u32 as u8],
        })
    }
}

/// `gLoadScreen` and the code that drives it (src/sys/loading.c).
pub struct LoadingScreen {
    timer: u32,
    period: u16,
    count: u16,
    presses: i16,
    frame: u16,
    cycles: u16,
    screen_type: u16,
    target: i32,
    items: [Item; ITEM_COUNT],
    flags: i32,
    pulse: Pulse,
    sprites: [Sprite; SPRITE_COUNT],
    fade: Fade,
    game_random: GameRandom,
    libc_random: LibcRandom,
}

impl LoadingScreen {
    /// `Load_RunBlocking` up to its loop: `Load_InitScreen`, then fade slot 0 in over one second.
    /// `forced_type` replaces the `Rand_Range(3)` pick, which only avoids `last_type`.
    pub fn new(
        mut game_random: GameRandom,
        libc_random: LibcRandom,
        last_type: Option<u16>,
        forced_type: Option<u16>,
    ) -> Self {
        let screen_type = forced_type.unwrap_or_else(|| {
            loop {
                let candidate = game_random.range(3) as u16;
                if Some(candidate) != last_type {
                    break candidate;
                }
            }
        });
        let (period, presses, cycles) = match screen_type {
            0 => (12, 1, 3),
            1 => (0, 2, 1),
            _ => (12, 5, 5),
        };
        let mut screen = Self {
            timer: 0,
            period,
            count: 0,
            presses,
            frame: 0,
            cycles,
            screen_type,
            target: 0,
            items: [Item::default(); ITEM_COUNT],
            flags: 0,
            pulse: Pulse::default(),
            sprites: [Sprite::default(); SPRITE_COUNT],
            fade: Fade::start_in(1.0),
            game_random,
            libc_random,
        };
        screen.build_sprites();
        screen
    }

    pub fn screen_type(&self) -> u16 {
        self.screen_type
    }

    /// One 30 Hz iteration of the `Load_RunBlocking` loop; returns what that frame draws, in order.
    /// `pad_pressed` is whether bit 0x200 went down on pad 0 / pad 1 this frame.
    pub fn run_frame(&mut self, pad_pressed: [bool; 2]) -> Vec<Quad> {
        self.fade.update();
        self.update();
        let mut quads = self.draw();
        quads.extend(self.fade.quad());
        self.read_input(pad_pressed);
        quads
    }

    fn build_sprites(&mut self) {
        let mut list = Vec::with_capacity(SPRITE_COUNT);
        match self.screen_type {
            0 => {
                list.push(spr(
                    SPR_CLEAR,
                    (0, 0, 0x200, 0x1C0),
                    (0, 0, 0, 0),
                    0,
                    (0x22, 0x49, 0x68, 0x80),
                ));
                list.push(END);
                list.push(spr(
                    SPR_DRAW,
                    (0x2D, 0x109, 0x6D, 0x189),
                    (0, 0, 0x40, 0x80),
                    1,
                    GREY,
                ));
                list.push(spr(
                    SPR_DRAW,
                    (0x13, 0x13E, 0x93, 0x1A9),
                    (0, 0x15, 0x80, 0x80),
                    0,
                    GREY,
                ));
                list.push(END);
                list.push(spr(
                    SPR_DRAW,
                    (0x71, 0x145, 0x88, 0x156),
                    (0, 0, 0x17, 0x11),
                    0,
                    GREY,
                ));
                for i in 0..9 {
                    list.push(spr(
                        SPR_DRAW,
                        (0x71, 0x13F - i * 6, 0x88, 0x148 - i * 6),
                        (0x20, 0, 0x37, 9),
                        0,
                        GREY,
                    ));
                }
                list.push(END);
            }
            1 => {
                list.push(spr(
                    SPR_CLEAR,
                    (0, 0, 0x200, 0x1C0),
                    (0, 0, 0, 0),
                    0,
                    (0x46, 0x10, 0x12, 0x80),
                ));
                list.push(END);
                list.push(spr(
                    SPR_DRAW,
                    (0x1A, 0x129, 0x9A, 0x1A9),
                    (0, 0, 0x80, 0x80),
                    0,
                    GREY,
                ));
                list.push(END);
                list.push(spr(
                    SPR_DRAW,
                    (0x79, 0x141, 0xBC, 0x180),
                    (0, 0x40, 0x43, 0x7F),
                    2,
                    GREY,
                ));
                list.push(END);
                for i in 0..3 {
                    list.push(spr(
                        SPR_DRAW,
                        (0x8B + i * 12, 0x151, 0x99 + i * 12, 0x16D),
                        (0x20, 0x1C, 0x30, 0x38),
                        2,
                        GREY,
                    ));
                }
                list.push(END);
            }
            _ => {
                list.push(spr(
                    SPR_CLEAR,
                    (0, 0, 0x200, 0x1C0),
                    (0, 0, 0, 0),
                    0,
                    (0x1F, 0x52, 0x18, 0x80),
                ));
                list.push(END);
                list.push(spr(
                    SPR_DRAW,
                    (0, 0, 0x40, 0x40),
                    (0x40, 0x40, 0x80, 0x80),
                    1,
                    GREY,
                ));
                list.push(END);
                list.push(spr(
                    SPR_DRAW,
                    (0, 0, 0x40, 0x40),
                    (0x40, 0, 0x80, 0x40),
                    1,
                    GREY,
                ));
                list.push(END);
                list.push(spr(
                    SPR_DRAW,
                    (0, 0, 0x40, 0x80),
                    (0, 0, 0x40, 0x80),
                    0,
                    GREY,
                ));
                list.push(END);
            }
        }
        self.sprites = [Sprite::default(); SPRITE_COUNT];
        self.sprites[..list.len()].copy_from_slice(&list);

        match self.screen_type {
            1 => {
                let rest = self.sprites[4];
                self.pulse = Pulse {
                    rect: [rest.x0, rest.y0, rest.x1, rest.y1],
                    amp: 0.5,
                    speed: 0.2,
                    ..Pulse::default()
                };
            }
            2 => {
                self.items = [Item::default(); ITEM_COUNT];
                for item in &mut self.items {
                    item.x = self.libc_random.int_range(0x10, 0x19C);
                    item.y = self.libc_random.int_range(0x30, 0x160);
                }
                self.target = self.game_random.range(ITEM_COUNT as u32) as i32;
            }
            _ => {}
        }
    }

    fn update(&mut self) {
        match self.screen_type {
            0 => {
                if u32::from(self.period) < self.timer || self.presses == 0 {
                    self.presses = 1;
                    self.frame ^= 1;
                    self.timer = 0;
                    if self.frame == 0 {
                        self.cycles -= 1;
                        if self.cycles == 0 {
                            if self.count < 100 {
                                self.count += 1;
                            }
                            self.cycles = 3;
                        }
                    }
                }
            }
            1 => {
                if self.presses == 0 {
                    self.frame ^= 1;
                    self.presses = self.frame as i16 + 2;
                    if self.frame == 0 {
                        self.cycles -= 1;
                        if self.cycles == 0 {
                            if self.count < 999 {
                                self.count += 1;
                            }
                            self.cycles = 1;
                        }
                    }
                }
            }
            _ => {
                if u32::from(self.period) < self.timer || self.flags & FLAG_PRESSED != 0 {
                    self.timer = 0;
                    self.frame ^= 1;
                    if self.flags & FLAG_TAKEN != 0 {
                        if usize::from(self.count) < ITEM_COUNT {
                            loop {
                                self.target = self.game_random.range(ITEM_COUNT as u32) as i32;
                                if self.items[self.target as usize].flags & 1 == 0 {
                                    break;
                                }
                            }
                        } else {
                            self.count = 0;
                            self.build_sprites();
                        }
                        self.flags &= !FLAG_TAKEN;
                    }
                }
                if self.presses == 0 && self.frame == 1 {
                    self.items[self.target as usize].flags |= 1;
                    self.presses = 5;
                    self.flags |= FLAG_TAKEN;
                    self.count += 1;
                }
            }
        }
        self.timer = self.timer.wrapping_add(1);
    }

    fn read_input(&mut self, pad_pressed: [bool; 2]) {
        self.flags &= !FLAG_PRESSED;
        for pressed in pad_pressed {
            if pressed {
                self.presses = (self.presses - 1).max(0);
                self.flags |= FLAG_PRESSED;
            }
        }
    }

    fn draw(&mut self) -> Vec<Quad> {
        let mut quads = Vec::new();
        match self.screen_type {
            0 => {
                self.draw_list(&mut quads, false, 0, 0, 0);
                self.sprites[2].u0 = i32::from(self.frame) << 6;
                self.sprites[2].u1 = self.sprites[2].u0 + 0x40;
                self.draw_list(&mut quads, true, 0, 0, 2);
                for i in 0..10 {
                    self.sprites[5 + i].flags = 0;
                }
                for i in 0..usize::from(self.count % 10) {
                    self.sprites[5 + i].flags = SPR_DRAW;
                }
                self.draw_list(&mut quads, true, 0, 0, 5);
                for i in 0..10 {
                    self.sprites[5 + i].flags = SPR_DRAW;
                }
                for i in 0..i32::from(self.count / 10) {
                    self.draw_list(&mut quads, true, 30 + i * 25, 60, 5);
                }
            }
            1 => {
                self.draw_list(&mut quads, false, 0, 0, 0);
                self.sprites[2].tex = i32::from(self.frame);
                self.draw_list(&mut quads, true, 0, 0, 2);
                self.pulse.size += self.pulse.angle.sin() * self.pulse.amp;
                self.pulse.angle += self.pulse.speed;
                if self.pulse.angle >= PI {
                    self.pulse.angle -= TAU;
                }
                let size = self.pulse.size as i32;
                self.sprites[4].y0 = self.pulse.rect[1] - size;
                self.sprites[4].y1 = self.pulse.rect[3] + size;
                self.sprites[4].x0 = self.pulse.rect[0] - size;
                self.sprites[4].x1 = self.pulse.rect[2] + size;
                self.draw_list(&mut quads, true, 0, 0, 4);
                if self.count != 0 {
                    let digits = to_digits(i32::from(self.count));
                    for (i, digit) in digits.into_iter().enumerate() {
                        let sprite = &mut self.sprites[6 + i];
                        sprite.flags = i32::from(digit != 10);
                        sprite.v0 = i32::from(digit >> 3) * 28;
                        sprite.v1 = sprite.v0 + 28;
                        sprite.u0 = i32::from(digit & 7) << 4;
                        sprite.u1 = sprite.u0 + 16;
                    }
                    let shift = match count_digits(i32::from(self.count)) {
                        1 => -14,
                        2 => -7,
                        _ => 0,
                    };
                    self.draw_list(&mut quads, true, shift, 0, 6);
                } else {
                    self.draw_list(&mut quads, true, 0, 0, 6);
                }
            }
            _ => {
                self.draw_list(&mut quads, false, 0, 0, 0);
                let target = self.items[self.target as usize];
                for (i, item) in self.items.into_iter().enumerate() {
                    if i as i32 != self.target
                        && item.flags & 1 == 0
                        && item.y + 0x20 <= target.y + 0x40
                    {
                        self.draw_list(&mut quads, true, item.x, item.y, 4);
                    }
                }
                if self.flags & FLAG_TAKEN != 0 {
                    self.sprites[6].u0 = 0;
                    self.sprites[6].u1 = 0x40;
                    self.sprites[6].tex = 1;
                    self.draw_list(&mut quads, true, target.x, target.y, 2);
                } else {
                    self.sprites[6].u0 = i32::from(self.frame) << 6;
                    self.sprites[6].u1 = self.sprites[6].u0 + 0x40;
                    self.sprites[6].tex = 0;
                }
                self.draw_list(&mut quads, true, target.x, target.y - 0x40, 6);
                for (i, item) in self.items.into_iter().enumerate() {
                    if i as i32 != self.target
                        && item.flags & 1 == 0
                        && item.y + 0x20 > target.y + 0x40
                    {
                        self.draw_list(&mut quads, true, item.x, item.y, 4);
                    }
                }
            }
        }
        quads
    }

    /// `Sprite_DrawList`: draws records from `start` up to the first one flagged END.
    fn draw_list(&self, quads: &mut Vec<Quad>, textured: bool, x: i32, y: i32, start: usize) {
        if self.sprites[start].a == 0 {
            return;
        }
        for sprite in self.sprites[start..]
            .iter()
            .take_while(|s| s.flags & SPR_END == 0)
        {
            if sprite.flags & SPR_DRAW == 0 {
                continue;
            }
            let texture =
                (textured && sprite.flags & SPR_UNTEXTURED == 0).then_some(sprite.tex as usize);
            quads.push(Quad {
                texture,
                screen: [sprite.x0 + x, sprite.y0 + y, sprite.x1 + x, sprite.y1 + y],
                uv: [sprite.u0, sprite.v0, sprite.u1, sprite.v1],
                rgba: [
                    sprite.r as u8,
                    sprite.g as u8,
                    sprite.b as u8,
                    sprite.a as u8,
                ],
            });
        }
    }
}

/// `Num_ToDigits(digits, value, 3, 0)` for a non-zero value: leading zeros become 10 (blank).
fn to_digits(value: i32) -> [u8; 3] {
    let pow10 = [1, 10, 100];
    let mut value = value.abs();
    let mut digits = [0u8; 3];
    let mut started = false;
    for (slot, i) in (1..3).rev().enumerate() {
        let d = value / pow10[i];
        if d != 0 {
            digits[slot] = d as u8;
            value -= d * pow10[i];
            started = true;
        } else {
            digits[slot] = if started { 0 } else { 10 };
        }
    }
    digits[2] = value as u8;
    digits
}

/// `Num_CountDigits`.
fn count_digits(value: i32) -> u32 {
    let value = value.abs();
    let mut digits = 1;
    while value >= 10i32.pow(digits) {
        digits += 1;
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::{LoadingScreen, count_digits, to_digits};
    use crate::rng::{GameRandom, LibcRandom};

    fn screen(screen_type: u16) -> LoadingScreen {
        LoadingScreen::new(
            GameRandom::from_key(&[1, 2, 3, 4]),
            LibcRandom::new(1),
            None,
            Some(screen_type),
        )
    }

    #[test]
    fn digits_blank_leading_zeros_only() {
        assert_eq!(to_digits(5), [10, 10, 5]);
        assert_eq!(to_digits(40), [10, 4, 0]);
        assert_eq!(to_digits(105), [1, 0, 5]);
        assert_eq!(count_digits(9), 1);
        assert_eq!(count_digits(10), 2);
        assert_eq!(count_digits(999), 3);
    }

    #[test]
    fn type_zero_steps_by_itself_every_thirteenth_frame() {
        let mut screen = screen(0);
        screen.presses = 1;
        let frames: Vec<u16> = (0..27)
            .map(|_| {
                screen.run_frame([false, false]);
                screen.frame
            })
            .collect();
        assert_eq!(frames[0], 0);
        assert_eq!(frames[12], 0);
        assert_eq!(frames[13], 1);
        assert_eq!(frames[26], 0);
    }

    #[test]
    fn type_zero_counts_after_three_full_cycles_of_presses() {
        let mut screen = screen(0);
        for _ in 0..6 {
            screen.run_frame([true, false]);
            screen.run_frame([false, false]);
        }
        assert_eq!(screen.count, 1);
    }

    #[test]
    fn type_one_draws_three_blank_digits_before_the_first_point() {
        let mut screen = screen(1);
        let quads = screen.run_frame([false, false]);
        let digits: Vec<_> = quads
            .iter()
            .filter(|q| q.uv == [0x20, 0x1C, 0x30, 0x38])
            .collect();
        assert_eq!(digits.len(), 3);
    }

    #[test]
    fn type_one_pulse_offset_is_truncated_to_whole_pixels() {
        let mut screen = screen(1);
        for _ in 0..4 {
            screen.run_frame([false, false]);
        }
        let size = screen.pulse.size as i32;
        assert!(screen.pulse.size > 0.0 && screen.pulse.size.fract() != 0.0);
        assert_eq!(screen.sprites[4].x0, 0x79 - size);
    }

    #[test]
    fn type_two_scatters_items_in_the_original_ranges() {
        let screen = screen(2);
        for item in screen.items {
            assert!((0x10..=0x19C).contains(&item.x));
            assert!((0x30..=0x160).contains(&item.y));
        }
        assert!((0..30).contains(&screen.target));
    }

    #[test]
    fn type_two_draws_every_untaken_item_except_the_target() {
        let mut screen = screen(2);
        let quads = screen.run_frame([false, false]);
        let items = quads
            .iter()
            .filter(|quad| quad.texture == Some(1) && quad.uv == [0x40, 0, 0x80, 0x40])
            .count();
        assert_eq!(items, 29);
    }

    #[test]
    fn type_two_takes_an_item_then_picks_an_untaken_target() {
        let mut screen = screen(2);
        let first = screen.target;
        screen.frame = 1;
        screen.presses = 0;
        screen.run_frame([false, false]);
        assert_eq!(screen.items[first as usize].flags & 1, 1);
        assert_eq!(screen.count, 1);
        screen.run_frame([true, false]);
        screen.run_frame([false, false]);
        assert_ne!(screen.target, first);
        assert_eq!(screen.items[screen.target as usize].flags & 1, 0);
    }

    #[test]
    fn type_two_rescatters_after_all_thirty_items() {
        let mut screen = screen(2);
        for item in &mut screen.items {
            item.flags = 1;
        }
        let before = screen.items;
        screen.count = 30;
        screen.flags = 1;
        screen.timer = 13;
        screen.run_frame([false, false]);
        assert_eq!(screen.count, 0);
        assert!(screen.items.iter().all(|item| item.flags == 0));
        assert_ne!(screen.items, before);
    }

    #[test]
    fn both_pads_each_count_one_press() {
        let mut screen = screen(1);
        screen.run_frame([true, true]);
        assert_eq!(screen.presses, 0);
    }

    #[test]
    fn fade_in_lasts_thirty_frames_then_turns_off() {
        let mut screen = screen(0);
        let alphas: Vec<Option<u8>> = (0..31)
            .map(|_| {
                screen.run_frame([false, false]);
                screen.fade.quad().map(|quad| quad.rgba[3])
            })
            .collect();
        assert_eq!(alphas[0], Some(123));
        assert_eq!(alphas[28], Some(4));
        assert_eq!(alphas[29], None);
    }
}
