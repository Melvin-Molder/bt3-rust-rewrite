use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use bt3_data::Iso9660;

use crate::fade::Fade;
use crate::loading::Quad;

pub const PSS_PATH: &str = "DATA/ZS3USOP.PSS";
pub const ADX_PATH: &str = "DATA/ZS3USOP.ADX";
/// `Movie_PlayEnding`'s files: the story's ending movie.
pub const ENDING_PSS_PATH: &str = "DATA/ZS3USED.PSS";
pub const ENDING_ADX_PATH: &str = "DATA/ZS3USED.ADX";
pub const WIDTH: u32 = 512;
pub const HEIGHT: u32 = 448;
const FRAME_BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;
// MOVIE_BUF_SIZE: the original reads the stream 0x100 sectors at a time.
const READ_CHUNK: usize = 0x80000;
const SKIP_FADE_SECONDS: f32 = 0.5;

/// `Adx_PlayFilePaused(ADX_CH_BGM, adx, 0x40, 0)` through `Adx_CalcOutVol`, in decibels.
/// Uses channel gain 0x3C (gAdxChannelGainTbl[0]) and the default music option, 9.
pub fn soundtrack_volume_db() -> f32 {
    crate::sound::out_volume(crate::sound::CH_BGM, 0x40) as f32 / 10.0
}

/// The opening video's pictures, read from the ISO and decoded by ffmpeg in the background.
pub struct VideoStream {
    frames: Mutex<Receiver<Vec<u8>>>,
    child: Child,
}

impl VideoStream {
    /// `Movie_Open`'s video: the .PSS file `pss` streamed from the ISO through `ffmpeg`.
    pub fn open(iso_path: &Path, pss: &'static str) -> io::Result<Self> {
        let mut iso = Iso9660::open(iso_path)?;
        let size = iso.file_size(pss)?;
        let mut child = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "mpeg", "-i", "pipe:0", "-map", "0:v:0"])
            .args([
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-fps_mode",
                "passthrough",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("ffmpeg stdin is piped");
        let mut stdout = child.stdout.take().expect("ffmpeg stdout is piped");
        thread::spawn(move || {
            let mut offset = 0u64;
            while offset < size {
                let len = READ_CHUNK.min((size - offset) as usize);
                let Ok(chunk) = iso.read_file_range(pss, offset, len) else {
                    break;
                };
                if stdin.write_all(&chunk).is_err() {
                    break;
                }
                offset += len as u64;
            }
        });
        let (sender, frames) = mpsc::sync_channel(8);
        thread::spawn(move || {
            loop {
                let mut frame = vec![0; FRAME_BYTES];
                if stdout.read_exact(&mut frame).is_err() || sender.send(frame).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            frames: Mutex::new(frames),
            child,
        })
    }

    /// `sceMpegGetPicture`: waits for the next picture; `None` once the stream has ended.
    pub fn next_picture(&self) -> Option<Vec<u8>> {
        self.frames.lock().ok()?.recv().ok()
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What one movie frame shows: a new picture to upload (if any) and the quads to draw.
pub struct MovieFrame {
    pub picture: Option<Vec<u8>>,
    pub quads: Vec<Quad>,
}

/// `Movie_Run` (src/sys/movie.c): one picture per frame until the stream ends or Start skips it.
pub struct MoviePlayer {
    texture: usize,
    fade: Option<Fade>,
    skip: bool,
    wait: u32,
    finished: bool,
}

impl MoviePlayer {
    pub fn new(texture: usize) -> Self {
        Self {
            texture,
            fade: None,
            skip: false,
            wait: 0,
            finished: false,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// One loop iteration; `start_pressed` is pad 0 PADG_START going down this frame.
    pub fn run_frame(
        &mut self,
        start_pressed: bool,
        next_picture: impl FnOnce() -> Option<Vec<u8>>,
    ) -> MovieFrame {
        if self.finished {
            return MovieFrame {
                picture: None,
                quads: Vec::new(),
            };
        }
        if let Some(fade) = &mut self.fade {
            fade.update();
        }
        if self.skip {
            if self.fade.is_some_and(|fade| fade.is_done()) {
                if self.wait >= 3 {
                    return self.finish();
                }
                self.wait += 1;
            }
        } else if start_pressed {
            self.fade = Some(Fade::start_out(SKIP_FADE_SECONDS));
            self.skip = true;
        }
        let Some(picture) = next_picture() else {
            return self.finish();
        };
        let mut quads = vec![Quad {
            texture: Some(self.texture),
            screen: [0, 0, WIDTH as i32, HEIGHT as i32],
            uv: [0, 0, WIDTH as i32, HEIGHT as i32],
            rgba: [0x80; 4],
        }];
        quads.extend(self.fade.and_then(|fade| fade.quad()));
        MovieFrame {
            picture: Some(picture),
            quads,
        }
    }

    /// The exit path: sound stopped, an instant fade to black, then the fade slot is reset.
    fn finish(&mut self) -> MovieFrame {
        self.finished = true;
        MovieFrame {
            picture: None,
            quads: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ENDING_ADX_PATH, ENDING_PSS_PATH, Iso9660, MoviePlayer, soundtrack_volume_db};

    #[test]
    fn soundtrack_plays_at_minus_twelve_point_nine_decibels() {
        assert_eq!(soundtrack_volume_db(), -12.9);
    }

    #[test]
    fn plays_every_picture_until_the_stream_ends() {
        let mut player = MoviePlayer::new(0);
        let mut pictures = vec![vec![1u8], vec![2u8]].into_iter();
        let mut shown = 0;
        while !player.is_finished() {
            if player
                .run_frame(false, || pictures.next())
                .picture
                .is_some()
            {
                shown += 1;
            }
        }
        assert_eq!(shown, 2);
    }

    #[test]
    fn start_fades_out_for_half_a_second_then_ends_four_frames_after_the_fade() {
        let mut player = MoviePlayer::new(0);
        let frame = player.run_frame(true, || Some(Vec::new()));
        assert_eq!(frame.quads[1].rgba[3], 0);
        let mut frames = 1;
        while !player.is_finished() {
            player.run_frame(true, || Some(Vec::new()));
            frames += 1;
        }
        // The fade is done on call 15 (14 updates after the start), calls 15-17 count `wait` to 3, call 18 leaves.
        assert_eq!(frames, 18);
    }

    /// The ending movie's files from the ISO in `rom/`, when it is there: the video, and the soundtrack decoded.
    #[test]
    fn real_ending_movie_files_are_on_the_disc() {
        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        assert!(image.file_size(ENDING_PSS_PATH).unwrap() > 1 << 20);
        let adx = image.read_file(ENDING_ADX_PATH).unwrap();
        let wav = crate::adx_to_wav(&adx).unwrap();
        assert!(wav.len() > adx.len());
    }

    #[test]
    fn only_start_is_read_before_skipping() {
        let mut player = MoviePlayer::new(0);
        let frame = player.run_frame(false, || Some(Vec::new()));
        assert_eq!(frame.quads.len(), 1);
    }
}
