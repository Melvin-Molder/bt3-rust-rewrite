//! The stage's ambience (`StgAmb_*`, src/battle/stg_ambient.c): a looping sound on the second music player
//! ("MAP", file 0x10BA4 + stage) whose volume a per-stage handler sets every frame (by the camera's height over
//! the water or its distance to a path), and the random thunder of stages 3, 4, 10 and 15 from the stage bank.

use crate::btl_snd::CharSound;
use crate::rng::LibcRandom;
use crate::title::TitleSound;

/// `MapBgm_Play(id + 0x10BA4)`: the stages' looping sounds.
const AMB_FILE: u32 = 0x10BA4;

/// Stage 10's path (x, z) the volume follows.
const PATH_10: [[f32; 2]; 8] = [
    [-986.8, -199.5],
    [-652.5, -100.6],
    [-412.8, 174.4],
    [-84.31, 308.1],
    [284.91, 305.2],
    [549.46, 98.8],
    [793.67, -139.5],
    [1017.5, -331.4],
];

/// `gStgAmb`.
#[derive(Clone, Debug, Default)]
pub(crate) struct StgAmb {
    /// The handler's stage (`update`), `None` once stopped.
    stage: Option<i32>,
    timer: i32,
    bgm_scale: f32,
    se_scale: f32,
    /// The volume last set (`MapBgm_SetVolume`), to send only changes.
    volume: Option<i32>,
    /// What the ambience asked of the sound players since the last frame.
    pub out: Vec<TitleSound>,
}

/// `StgAmb_Falloff`: 1 up to `near`, falling linearly to 0 at `far`.
fn falloff(x: f32, near: f32, far: f32) -> f32 {
    if far < x {
        0.0
    } else if near < x {
        1.0 - (x - near) / (far - near)
    } else {
        1.0
    }
}

/// The handler a stage uses (`StgAmb_Start`'s table): the variants of a stage share its handler.
fn handler(stage: i32) -> Option<i32> {
    Some(match stage {
        0 | 23 | 24 => 0,
        1 | 25 | 26 => 1,
        4 | 27 => 4,
        6 | 28 => 6,
        9 | 29 | 30 => 9,
        2..=22 => stage,
        _ => return None,
    })
}

impl StgAmb {
    /// `StgAmb_Start`: the stage's handler and its looping sound, silent.
    pub(crate) fn start(&mut self, stage: i32) {
        let out = std::mem::take(&mut self.out);
        *self = Self {
            stage: handler(stage),
            bgm_scale: 1.0,
            se_scale: 1.0,
            out,
            ..Self::default()
        };
        self.out
            .push(TitleSound::MapBgm(AMB_FILE + stage.max(0) as u32));
        self.set_volume(0);
    }

    /// `StgAmb_Stop`: silent and stopped.
    pub(crate) fn stop(&mut self) {
        self.set_volume(0);
        self.out.push(TitleSound::MapBgmStop);
        self.stage = None;
    }

    /// `StgAmb_SetBgmVolume`: a tenth of the volume, scaled.
    fn set_volume(&mut self, volume: i32) {
        let v = (volume as f32 * self.bgm_scale * 0.1) as i32;
        if self.volume != Some(v) {
            self.volume = Some(v);
            self.out.push(TitleSound::MapBgmVolume(v));
        }
    }

    /// The thunder of stages 3, 4, 10, 15: every 90..179 frames a stage sound 10 (`StgAmb_PlaySe(8, 10, ...)`) at
    /// a random volume and pan; `quiet` takes a quarter of it.
    fn thunder(
        &mut self,
        libc: &mut LibcRandom,
        lo: i32,
        hi_range: i32,
        quiet: bool,
        snd: &mut CharSound,
    ) {
        if self.timer == 0 {
            self.timer = libc.int_range(0, hi_range) % 90 + 90;
        }
        self.timer -= 1;
        if self.timer <= 0 {
            self.timer = libc.int_range(0, hi_range) % 90 + 90;
            let vol = libc.int_range(lo, 0x7F);
            let pan = libc.int_range(-15, 15);
            let vol = if quiet { vol >> 2 } else { vol };
            snd.play_se(
                1,
                10,
                (vol as f32 * self.se_scale) as i32,
                pan,
                0,
                &mut self.out,
            );
        }
    }

    /// `StgAmb_Update`: the stage's handler. `cam` is the camera's position (`gBtlCamView`), `water` the stage's
    /// water level (`BtlStage_GetWaterLevel`: 0 without water).
    pub(crate) fn update(
        &mut self,
        cam: Option<[f32; 3]>,
        water: f32,
        libc: &mut LibcRandom,
        snd: &mut CharSound,
    ) {
        let Some(stage) = self.stage else {
            return;
        };
        // The camera's distance above (or below) the water.
        let water_k = |near_under: f32, far_under: f32, near_over: f32, far_over: f32| {
            cam.map(|c| {
                if c[1] < water {
                    falloff(water - c[1], near_under, far_under)
                } else {
                    falloff(c[1] - water, near_over, far_over)
                }
            })
        };
        match stage {
            0 | 1 | 5 | 6 | 8 => self.set_volume(0x50),
            7 => self.set_volume(0x40),
            9 => self.set_volume(0x80),
            13 => self.set_volume(0),
            16..=22 => self.set_volume(0x7F),
            2 => {
                if let Some(c) = cam {
                    let vol = if c[1] < water {
                        falloff(water - c[1], 20.0, 180.0) * 128.0
                    } else {
                        falloff(c[1] - water, 20.0, 250.0) * 96.0
                    };
                    self.set_volume(vol as i32);
                }
            }
            3 => {
                self.thunder(libc, 0x3F, 0xFFFF, false, snd);
                if let Some(k) = water_k(0.0, 700.0, 0.0, 700.0) {
                    self.set_volume((k * 128.0) as i32);
                }
            }
            4 => {
                self.thunder(libc, 0x3F, 0xFFFF, true, snd);
                self.set_volume(0x3F);
            }
            10 => {
                self.thunder(libc, 0x3F, 0xFFFF, true, snd);
                if let (Some(c), Some(k)) = (cam, water_k(20.0, 180.0, 20.0, 250.0)) {
                    let best = PATH_10
                        .iter()
                        .map(|p| ((p[0] - c[0]).powi(2) + (p[1] - c[2]).powi(2)).sqrt())
                        .fold(f32::INFINITY, f32::min);
                    self.set_volume((k * falloff(best, 280.0, 550.0) * 80.0) as i32);
                }
            }
            11 => {
                if let Some(k) = water_k(20.0, 150.0, 20.0, 150.0) {
                    self.set_volume((k * 128.0) as i32);
                }
            }
            12 => {
                if let Some(k) = water_k(20.0, 300.0, 20.0, 300.0) {
                    self.set_volume((k * 96.0) as i32);
                }
            }
            14 => {
                if let Some(k) = water_k(20.0, 1200.0, 20.0, 1200.0) {
                    self.set_volume((k * 96.0) as i32);
                }
            }
            15 => {
                self.thunder(libc, 0x5F, 0x10000, false, snd);
                self.set_volume(0x60);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handlers_follow_the_stage_table() {
        assert_eq!(handler(24), Some(0));
        assert_eq!(handler(27), Some(4));
        assert_eq!(handler(30), Some(9));
        assert_eq!(handler(14), Some(14));
        assert_eq!(handler(31), None);
        assert_eq!(falloff(10.0, 20.0, 180.0), 1.0);
        assert_eq!(falloff(100.0, 20.0, 180.0), 0.5);
        assert_eq!(falloff(200.0, 20.0, 180.0), 0.0);
    }

    #[test]
    fn the_loop_starts_silent_and_thunder_comes_back_every_90_to_179_frames() {
        let mut amb = StgAmb::default();
        amb.start(15);
        assert_eq!(
            amb.out,
            [
                TitleSound::MapBgm(0x10BA4 + 15),
                TitleSound::MapBgmVolume(0)
            ]
        );
        amb.out.clear();
        let mut snd = CharSound::default();
        // Without the stage bank no thunder sounds, but the timer still runs on libc rand().
        let mut libc = LibcRandom::new(1);
        amb.update(None, 0.0, &mut libc, &mut snd);
        // 0x60 * 0.1.
        assert_eq!(amb.out, [TitleSound::MapBgmVolume(9)]);
        assert!((89..179).contains(&amb.timer), "{}", amb.timer);
        amb.stop();
        assert_eq!(amb.out.last(), Some(&TitleSound::MapBgmStop));
    }
}
