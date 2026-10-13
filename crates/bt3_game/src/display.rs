//! How the finished frame reaches the window. Every pass draws into the console's 512 x 448 frame, as on the PS2;
//! only the finished frame is scaled to the window, the way a television scales the console's output. The frame
//! is never drawn at a higher resolution.

use bevy::camera::ScalingMode;
use bevy::image::ImageSampler;
use bevy::prelude::Resource;
use bevy::window::{MonitorSelection, WindowMode};

use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// The shape the frame is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    /// 4:3, as a television shows the 512 x 448 frame (its pixels a little wider than tall).
    Tv,
    /// Square pixels: 512:448.
    Pixel,
    /// The whole window, whatever its shape.
    Stretch,
}

/// How the frame's pixels are scaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Blended between neighbours.
    Linear,
    /// Each pixel a solid block.
    Nearest,
}

/// The display settings (`--aspect`, `--filter`, `--scale`, `--fullscreen`; F11 switches full screen).
#[derive(Clone, Copy, Debug, Resource)]
pub struct DisplaySettings {
    pub aspect: Aspect,
    pub filter: Filter,
    /// The window's size at start, in multiples of the frame shown at the aspect.
    pub scale: u32,
    pub fullscreen: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            aspect: Aspect::Tv,
            filter: Filter::Linear,
            scale: 2,
            fullscreen: false,
        }
    }
}

impl DisplaySettings {
    /// `--aspect`'s value.
    pub fn parse_aspect(value: &str) -> Option<Aspect> {
        match value {
            "tv" | "4:3" => Some(Aspect::Tv),
            "pixel" | "square" => Some(Aspect::Pixel),
            "stretch" => Some(Aspect::Stretch),
            _ => None,
        }
    }

    /// `--filter`'s value.
    pub fn parse_filter(value: &str) -> Option<Filter> {
        match value {
            "linear" | "smooth" => Some(Filter::Linear),
            "nearest" | "sharp" => Some(Filter::Nearest),
            _ => None,
        }
    }

    /// The frame's size in the window's world units: as wide as the aspect makes it, 448 high.
    pub fn shown_size(&self) -> (f32, f32) {
        let h = SCREEN_HEIGHT as f32;
        match self.aspect {
            Aspect::Tv => (h * 4.0 / 3.0, h),
            Aspect::Pixel | Aspect::Stretch => (SCREEN_WIDTH as f32, h),
        }
    }

    /// The window camera's scaling: the frame as large as fits, bars round it; or the whole window.
    pub fn scaling(&self) -> ScalingMode {
        let (width, height) = self.shown_size();
        match self.aspect {
            Aspect::Stretch => ScalingMode::Fixed { width, height },
            Aspect::Tv | Aspect::Pixel => ScalingMode::AutoMin {
                min_width: width,
                min_height: height,
            },
        }
    }

    /// The window's size at start, in physical pixels.
    pub fn window_size(&self) -> (u32, u32) {
        let (width, height) = self.shown_size();
        let scale = self.scale.max(1) as f32;
        (
            (width * scale).round() as u32,
            (height * scale).round() as u32,
        )
    }

    /// The window's mode at start.
    pub fn window_mode(&self) -> WindowMode {
        if self.fullscreen {
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        } else {
            WindowMode::Windowed
        }
    }

    /// The finished frame's sampler.
    pub fn sampler(&self) -> ImageSampler {
        match self.filter {
            Filter::Linear => ImageSampler::linear(),
            Filter::Nearest => ImageSampler::nearest(),
        }
    }
}

/// F11: the window between full screen (borderless, on its monitor) and a window.
pub fn toggle_fullscreen(mode: WindowMode) -> WindowMode {
    match mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tv_shape_is_four_by_three_and_the_window_twice_it() {
        let d = DisplaySettings::default();
        let (w, h) = d.shown_size();
        assert_eq!(((w * 3.0).round(), h), (1792.0, 448.0));
        assert_eq!(d.window_size(), (1195, 896));
        assert!(matches!(
            d.scaling(),
            ScalingMode::AutoMin { min_width, min_height } if (min_width * 3.0 - min_height * 4.0).abs() < 1e-3
        ));
    }

    #[test]
    fn square_pixels_keep_the_frame_s_own_shape_and_stretch_fills() {
        let pixel = DisplaySettings {
            aspect: Aspect::Pixel,
            scale: 1,
            ..DisplaySettings::default()
        };
        assert_eq!(pixel.window_size(), (SCREEN_WIDTH, SCREEN_HEIGHT));
        let stretch = DisplaySettings {
            aspect: Aspect::Stretch,
            ..pixel
        };
        assert!(matches!(
            stretch.scaling(),
            ScalingMode::Fixed {
                width: 512.0,
                height: 448.0
            }
        ));
    }

    #[test]
    fn the_values_parse_and_f11_switches() {
        assert_eq!(DisplaySettings::parse_aspect("4:3"), Some(Aspect::Tv));
        assert_eq!(DisplaySettings::parse_aspect("square"), Some(Aspect::Pixel));
        assert_eq!(DisplaySettings::parse_aspect("16:9"), None);
        assert_eq!(
            DisplaySettings::parse_filter("sharp"),
            Some(Filter::Nearest)
        );
        let full = toggle_fullscreen(WindowMode::Windowed);
        assert!(matches!(full, WindowMode::BorderlessFullscreen(_)));
        assert_eq!(toggle_fullscreen(full), WindowMode::Windowed);
    }
}
