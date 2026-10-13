//! The controller watch (`PadWatch_*`, src/sys/pad_watch.c): every frame, at the frame's end (`Gfx_EndFrame`),
//! whether each controller port has a controller is read and debounced (a reading has to hold 7 frames before it
//! is taken), and from the taken readings and what the screen needs the warning is chosen: controller 1, 2 or both
//! missing. Both controllers are needed in a split-screen battle that is not a replay, or outside a battle on the
//! Vs mode's character and team select of 1P vs 2P; elsewhere only port 1 counts. The warning darkens the whole
//! screen and prints the boot file's text of it in the middle; the battle's pause poll opens the pause menu for
//! the missing controller. Loading screens, the boot pictures and the movies turn the warning off.
//!
//! Here port 1 is the keyboard with the first gamepad, so it always has a controller; port 2 has one while a second
//! gamepad is there.

use crate::draw::ScreenQuad;
use crate::font::{Font, rgba};

/// `PAD_WATCH_STABLE_FRAMES`: the frames a new reading has to hold after the first before it is taken.
const STABLE_FRAMES: i32 = 6;

/// `PadWatch`.
#[derive(Clone, Debug)]
pub struct PadWatch {
    /// The taken reading of each port: bit 0 connected.
    state: [i32; 2],
    /// The last frame's reading.
    raw: [i32; 2],
    /// Frames the reading has held, up to 6.
    count: [i32; 2],
    /// The port has changed or settled once.
    valid: [bool; 2],
    /// `PadWatch_SetEnabled(0)`: no warning is chosen or drawn.
    pub disabled: bool,
    /// `PAD_WATCH_MSG_*`: -1 none, 0 controller 1, 1 controller 2, 2 both.
    message: i32,
}

impl Default for PadWatch {
    /// `PadWatch_Init`: both ports start connected.
    fn default() -> Self {
        Self {
            state: [1; 2],
            raw: [1; 2],
            count: [0; 2],
            valid: [false; 2],
            disabled: false,
            message: 0,
        }
    }
}

impl PadWatch {
    /// `PadWatch_Update`: `connected` is each port's reading this frame, `both` whether the screen needs both
    /// controllers.
    pub fn update(&mut self, connected: [bool; 2], both: bool) {
        self.message = -1;
        let mut now = [0; 2];
        for i in 0..2 {
            let reading = i32::from(connected[i]);
            if self.raw[i] == reading {
                if self.count[i] >= STABLE_FRAMES {
                    self.valid[i] = true;
                    self.state[i] = self.raw[i];
                } else {
                    self.count[i] += 1;
                }
            } else {
                self.valid[i] = true;
                self.count[i] = 0;
                self.raw[i] = reading;
            }
            now[i] = self.state[i];
        }
        if self.disabled || !(self.valid[0] || self.valid[1]) {
            return;
        }
        if both {
            if now[0] & 1 == 0 {
                self.message = if now[1] & 1 == 0 { 2 } else { 0 };
            } else if now[1] & 1 == 0 {
                self.message = 1;
            }
        } else if now[0] & 1 == 0 {
            self.message = 0;
        }
    }

    /// `PadWatch_GetMissing`: with a warning up, the port that is missing (port 1 when both are; port 1 too if the
    /// taken readings no longer say which).
    pub fn missing(&self, both: bool) -> Option<usize> {
        if !(self.valid[0] || self.valid[1]) || self.message < 0 {
            return None;
        }
        let port = if self.state[0] & 1 == 0 {
            0
        } else if both && self.state[1] & 1 == 0 {
            1
        } else {
            0
        };
        Some(port)
    }

    /// `PadWatch_DrawMessage`: unless off or in a battle of mode 7, the screen darkened (a black sprite of alpha
    /// 0x60 over it all) and the warning's text (`texts`: the boot file's words 9..11) printed in white with a
    /// shadow, centred on the screen.
    pub fn draw(
        &self,
        battle_mode_7: bool,
        texts: &[Vec<u16>; 3],
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        if self.disabled || battle_mode_7 || !(self.valid[0] || self.valid[1]) || self.message < 0 {
            return;
        }
        let Some(text) = texts.get(self.message as usize) else {
            return;
        };
        // XYZ (0x6F00, 0x7100) .. (0x9100, 0x8F00): 16 pixels past every edge.
        let [x0, y0, x1, y1] = [-16.0, -16.0, 528.0, 464.0];
        quads.push(ScreenQuad {
            texture: None,
            pt: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
            uv: [[0.0; 2]; 4],
            rgba: [0, 0, 0, 0x60],
        });
        let height = font.height(text);
        font.push_style();
        font.style.align = 1;
        font.style.shadow_mode = 1;
        font.style.clip = [0, 0, 0x200, 0x1C0];
        font.style.color = rgba(0xFF, 0xFF, 0xFF, 0x80);
        font.style.shadow_color = rgba(0, 0, 0, 0x80);
        font.print_at(0x100, 0xE0 - height / 2, text);
        font.pop_style();
        font.flush(quads);
    }
}

/// The three warnings of the boot file (`boot`): the texts at the byte offsets of its words 9..11, each rounded
/// down to a word.
pub fn texts(boot: &[u8]) -> Option<[Vec<u16>; 3]> {
    let mut texts: [Vec<u16>; 3] = Default::default();
    for (i, text) in texts.iter_mut().enumerate() {
        let word = boot.get((9 + i) * 4..(10 + i) * 4)?;
        let start = u32::from_le_bytes(word.try_into().ok()?) as usize & !3;
        *text = boot
            .get(start..)?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .take_while(|&c| c != 0)
            .collect();
    }
    Some(texts)
}

#[cfg(test)]
mod tests {
    use super::PadWatch;

    /// The boot file's warnings, from the ISO in `rom/`: controller 1, 2 and both.
    #[test]
    fn real_warnings_are_in_the_boot_file() {
        use bt3_data::Iso9660;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(5);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let boot = archive.read_entry(&mut image, location.index).unwrap();
        let texts = super::texts(&boot).unwrap();
        let s: Vec<String> = texts.iter().map(|t| String::from_utf16_lossy(t)).collect();
        assert!(s[0].starts_with("\u{FEFF}No DUALSHOCK"), "{s:?}");
        assert!(s[2].contains("controllers"), "{s:?}");
    }

    #[test]
    fn a_pulled_out_controller_warns_after_seven_frames() {
        let mut w = PadWatch::default();
        // Both there: nothing, and the readings settle on the seventh frame.
        for _ in 0..7 {
            w.update([true, true], true);
            assert_eq!(w.missing(true), None);
        }
        // Controller 2 out: the reading changes at once, the taken one 7 frames later.
        for _ in 0..7 {
            w.update([true, false], true);
            assert_eq!(w.missing(true), None);
        }
        w.update([true, false], true);
        assert_eq!(w.missing(true), Some(1));
        // One controller is all the screen needs: no warning.
        w.update([true, false], false);
        assert_eq!(w.missing(false), None);
        // Off: none.
        w.disabled = true;
        w.update([true, false], true);
        assert_eq!(w.missing(true), None);
    }

    #[test]
    fn a_controller_missing_from_the_start_warns_once_settled() {
        let mut w = PadWatch::default();
        // The first reading differs from the "connected" start: the port is valid at once, but still taken as
        // connected for 7 frames.
        for _ in 0..7 {
            w.update([true, false], true);
            assert_eq!(w.missing(true), None);
        }
        w.update([true, false], true);
        assert_eq!(w.missing(true), Some(1));
    }
}
