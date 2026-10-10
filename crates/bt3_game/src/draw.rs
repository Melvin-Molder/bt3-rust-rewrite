use crate::loading::Quad;

/// One GS quad as the renderer draws it: corners top left, top right, bottom left, bottom right in screen pixels,
/// their texel coordinates, and the GS vertex colour (0x80 modulates by 1; alpha 0x80 is opaque).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenQuad {
    pub texture: Option<usize>,
    pub pt: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub rgba: [u8; 4],
}

impl From<Quad> for ScreenQuad {
    fn from(quad: Quad) -> Self {
        let [x0, y0, x1, y1] = quad.screen.map(|v| v as f32);
        let [u0, v0, u1, v1] = quad.uv.map(|v| v as f32);
        Self {
            texture: quad.texture,
            pt: [[x0, y0], [x1, y0], [x0, y1], [x1, y1]],
            uv: [[u0, v0], [u1, v0], [u0, v1], [u1, v1]],
            rgba: quad.rgba,
        }
    }
}
