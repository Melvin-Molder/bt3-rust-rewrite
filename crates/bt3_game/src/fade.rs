use crate::loading::Quad;

/// A fade slot of src/sys/fade.c: a colour ramp from `from` to `to` over `seconds * 30` steps (slot 0 black,
/// slot 2 the stage change's colour).
#[derive(Clone, Copy, Debug, Default)]
pub struct Fade {
    active: bool,
    done: bool,
    color: [f32; 4],
    from: [f32; 4],
    delta: [f32; 4],
    ramp_value: f32,
    ramp_frames: f32,
    ramp_step: f32,
}

/// `gFadeColors[0]`: clear and opaque black.
const BLACK: [[f32; 4]; 2] = [[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 128.0]];

impl Fade {
    /// `Fade_Start(0, FADE_IN, seconds)`: black to clear.
    pub fn start_in(seconds: f32) -> Self {
        Self::start(BLACK[1], BLACK[0], seconds)
    }

    /// `Fade_Start(0, FADE_OUT, seconds)`: clear to black.
    pub fn start_out(seconds: f32) -> Self {
        Self::start(BLACK[0], BLACK[1], seconds)
    }

    /// `Fade_Start(slot, dir, seconds)` with the slot's colours (`gFadeColors[slot]`: clear, opaque): `fade_in`
    /// from opaque to clear, else from clear to opaque.
    pub fn start_colored(colors: [[f32; 4]; 2], fade_in: bool, seconds: f32) -> Self {
        if fade_in {
            Self::start(colors[1], colors[0], seconds)
        } else {
            Self::start(colors[0], colors[1], seconds)
        }
    }

    fn start(from: [f32; 4], to: [f32; 4], seconds: f32) -> Self {
        let frames = seconds * 30.0;
        let mut fade = Self {
            active: true,
            done: false,
            color: from,
            from,
            delta: std::array::from_fn(|k| to[k] - from[k]),
            ramp_value: 0.0,
            ramp_frames: frames,
            ramp_step: 1.0 / frames,
        };
        // Fade_Start already performs the first step.
        fade.update();
        fade
    }

    /// `FadeSlot_Update`, run by `Gfx_BeginFrame` every frame.
    pub fn update(&mut self) {
        if !self.active {
            return;
        }
        self.color = std::array::from_fn(|k| self.delta[k] * self.ramp_value + self.from[k]);
        if self.done {
            if self.color[3] as i32 == 0 {
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

    /// `Fade_IsDone`: the ramp has ended, or the slot is off.
    pub fn is_done(&self) -> bool {
        self.done || !self.active
    }

    /// What `Fade_DrawScreen` draws over the finished frame, if the slot is on.
    pub fn quad(&self) -> Option<Quad> {
        self.active.then(|| Quad {
            texture: None,
            screen: [0, 0, 512, 448],
            uv: [0; 4],
            rgba: self.color.map(|c| c as u32 as u8),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Fade;

    #[test]
    fn half_second_fade_out_is_done_after_fifteen_steps() {
        let mut fade = Fade::start_out(0.5);
        let mut updates = 0;
        while !fade.is_done() {
            fade.update();
            updates += 1;
        }
        assert_eq!(updates, 14);
        fade.update();
        assert_eq!(fade.quad().unwrap().rgba[3], 128);
    }

    #[test]
    fn instant_fade_out_covers_the_screen_on_the_next_update() {
        let mut fade = Fade::start_out(0.0);
        assert!(fade.is_done());
        assert_eq!(fade.quad().unwrap().rgba[3], 0);
        fade.update();
        assert_eq!(fade.quad().unwrap().rgba[3], 128);
    }
}
