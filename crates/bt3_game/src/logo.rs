use crate::loading::Quad;

/// Sections of the unpacked logo pack, in `Logo_ShowAll` order.
pub const PICTURE_SECTIONS: [usize; 6] = [6, 7, 8, 3, 4, 5];
pub const TILES_PER_PICTURE: usize = 8;

const FRAMES: i32 = 0x78;
const FADE_FRAMES: i32 = 0x3C;

/// `(skippable, fade in from black, fade out to black)` per `Logo_ShowAll` call.
const LOGOS: [(bool, bool, bool); 6] = [
    (true, true, false),
    (true, false, false),
    (true, false, false),
    (true, false, false),
    (true, false, false),
    (false, false, true),
];

const FADE_MAX: i32 = 0x80;
const FADE_IN: i32 = 1;
const FADE_OUT: i32 = 2;
const FADE_IN_DONE: i32 = 4;
const FADE_OUT_DONE: i32 = 8;

/// `gColorFade` (src/sys/color_fade.c): one full-screen colour with an integer alpha ramp.
#[derive(Clone, Copy, Debug)]
struct ColorFade {
    flags: i32,
    rgb: [u8; 3],
    alpha: i32,
    step: i32,
}

impl ColorFade {
    fn start_in(black: bool, frames: i32) -> Self {
        Self {
            flags: FADE_IN,
            rgb: fade_color(black),
            alpha: FADE_MAX,
            step: FADE_MAX / frames,
        }
    }

    fn start_out(black: bool, frames: i32) -> Self {
        Self {
            flags: FADE_OUT,
            rgb: fade_color(black),
            alpha: 0,
            step: FADE_MAX / frames,
        }
    }

    fn update(&mut self) {
        if self.flags & FADE_IN != 0 {
            if self.alpha > 0 {
                self.alpha -= self.step;
            } else {
                self.flags = FADE_IN_DONE;
            }
        }
        if self.flags & FADE_OUT != 0 {
            if self.alpha < self.step + FADE_MAX {
                self.alpha += self.step;
            } else {
                self.flags = FADE_OUT_DONE;
            }
        }
    }

    fn quad(&self) -> Option<Quad> {
        let a = self.alpha.clamp(0, FADE_MAX) as u8;
        (a != 0).then(|| Quad {
            texture: None,
            screen: [0, 0, 512, 448],
            uv: [0; 4],
            rgba: [self.rgb[0], self.rgb[1], self.rgb[2], a],
        })
    }
}

fn fade_color(black: bool) -> [u8; 3] {
    if black { [0; 3] } else { [0xFF; 3] }
}

/// The boot pictures of `FirstRun_Main`: `Logo_ShowAll`, one `Logo_Show` loop per picture, at 60 Hz.
pub struct LogoSequence {
    index: usize,
    timer: i32,
    faded_in: bool,
    fade: ColorFade,
}

impl LogoSequence {
    pub fn new() -> Self {
        Self::show(0)
    }

    fn show(index: usize) -> Self {
        Self {
            index,
            timer: 0,
            faded_in: false,
            fade: ColorFade::start_in(LOGOS[index].1, FADE_FRAMES),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.index >= LOGOS.len()
    }

    /// One `Logo_Show` frame; `skip_pressed` is pad 0 Start or Cross going down this frame.
    pub fn run_frame(&mut self, skip_pressed: bool) -> Vec<Quad> {
        if self.is_finished() {
            return Vec::new();
        }
        let (skippable, _, out_black) = LOGOS[self.index];
        self.fade.update();
        let mut quads = picture_quads(self.index * TILES_PER_PICTURE);
        quads.extend(self.fade.quad());
        if self.fade.flags & FADE_IN_DONE != 0 {
            self.faded_in = true;
        }
        if self.fade.flags & FADE_OUT != 0 {
            return quads;
        }
        if self.fade.flags & FADE_OUT_DONE != 0 {
            if self.index + 1 < LOGOS.len() {
                *self = Self::show(self.index + 1);
            } else {
                self.index = LOGOS.len();
            }
            return quads;
        }
        if self.faded_in {
            self.timer += 1;
            if self.timer >= FRAMES + 2 {
                self.fade = ColorFade::start_out(out_black, FADE_FRAMES);
            }
        }
        if skippable && skip_pressed {
            self.fade = ColorFade::start_out(out_black, FADE_FRAMES / 2);
        }
        quads
    }
}

/// `Sprite_DrawPicture(pic, 0, 0, 0x80)`: 4 x 2 tiles of 128x256 texels, each shown 128x224.
fn picture_quads(first_texture: usize) -> Vec<Quad> {
    let mut quads = Vec::with_capacity(TILES_PER_PICTURE);
    for row in 0..2 {
        let (y0, y1) = if row == 0 { (0, 0xE0) } else { (0xE0, 0x1C0) };
        for col in 0..4 {
            quads.push(Quad {
                texture: Some(first_texture + col + row * 4),
                screen: [col as i32 * 0x80, y0, col as i32 * 0x80 + 0x80, y1],
                uv: [0, 0, 0x80, 0x100],
                rgba: [0x80; 4],
            });
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::{FADE_OUT, LogoSequence};

    fn fade_alpha(logos: &mut LogoSequence, skip: bool) -> Option<u8> {
        let quads = logos.run_frame(skip);
        quads.get(8).map(|quad| quad.rgba[3])
    }

    #[test]
    fn first_logo_fades_in_from_black_over_sixty_four_steps() {
        let mut logos = LogoSequence::new();
        let first = logos.run_frame(false);
        assert_eq!(first[8].rgba, [0, 0, 0, 126]);
        let alphas: Vec<_> = (0..63).map(|_| fade_alpha(&mut logos, false)).collect();
        assert_eq!(alphas[61], Some(2));
        assert_eq!(alphas[62], None);
        assert!(!logos.faded_in);
        fade_alpha(&mut logos, false);
        assert!(logos.faded_in);
        assert_eq!(logos.timer, 1);
    }

    #[test]
    fn picture_holds_until_the_timer_reaches_frames_plus_two() {
        let mut logos = LogoSequence::new();
        for _ in 0..65 + 120 {
            logos.run_frame(false);
        }
        assert_eq!(logos.timer, 121);
        assert_eq!(logos.fade.flags & FADE_OUT, 0);
        logos.run_frame(false);
        assert_eq!(logos.fade.flags & FADE_OUT, FADE_OUT);
        assert_eq!(logos.fade.rgb, [0xFF; 3]);
    }

    #[test]
    fn skip_fades_out_twice_as_fast_and_ignores_presses_while_fading_out() {
        let mut logos = LogoSequence::new();
        logos.run_frame(true);
        assert_eq!(logos.fade.step, 4);
        logos.run_frame(true);
        assert_eq!(logos.fade.alpha, 4);
        assert_eq!(logos.fade.step, 4);
    }

    #[test]
    fn last_logo_cannot_be_skipped_and_the_sequence_ends_in_black() {
        let mut logos = LogoSequence::show(5);
        for _ in 0..70 {
            logos.run_frame(true);
        }
        assert_eq!(logos.fade.flags & FADE_OUT, 0);
        let mut frames = 70;
        while !logos.is_finished() {
            logos.run_frame(true);
            frames += 1;
        }
        // In-done on frame 65 (timer 1), fade-out starts at timer 122 (frame 186), alpha 130 after 65 more, done next frame.
        assert_eq!(frames, 65 + 121 + 65 + 1);
        assert_eq!(logos.fade.rgb, [0; 3]);
    }

    #[test]
    fn six_pictures_play_in_order() {
        let mut logos = LogoSequence::new();
        let mut first_textures = Vec::new();
        while !logos.is_finished() {
            let quads = logos.run_frame(false);
            if first_textures.last() != Some(&quads[0].texture) {
                first_textures.push(quads[0].texture);
            }
        }
        let expected: Vec<_> = (0..6).map(|picture| Some(picture * 8)).collect();
        assert_eq!(first_textures, expected);
    }
}
