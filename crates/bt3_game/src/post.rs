//! The frame redrawn through itself (`StgBlur_Draw`, `ScrWarp_Draw`): the GS copies the frame, or a part of it, into
//! the two 256 x 224 work buffers of frame blocks 0x150 / 0x16C (texture blocks 0x2A00 / 0x2D80) and draws them back
//! as textured triangles. A `PostOp` is one such draw: triangles textured from `source` into `target`, copied or
//! blended (ALPHA 0x44 with the vertex alpha; TEX0 TCC 0 and MODULATE, so the colour is the texel's times the
//! vertex's / 128 and the alpha the vertex's). They run after the effects and before the HUD.

/// A buffer the post ops read and write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostBuf {
    /// The frame being drawn (512 x 448).
    Frame,
    /// Work buffer 0 or 1 (256 x 224).
    Work(usize),
}

impl PostBuf {
    /// Width and height in pixels.
    pub fn size(self) -> [f32; 2] {
        match self {
            PostBuf::Frame => [512.0, 448.0],
            PostBuf::Work(_) => [256.0, 224.0],
        }
    }
}

/// A vertex: position in the target's pixels, texel in the source's pixels (GS UV), colour as the GS takes it
/// (128 = 1.0; alpha 128 = opaque).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PostVertex {
    pub xy: [f32; 2],
    pub uv: [f32; 2],
    pub rgba: [u8; 4],
}

/// One draw of the post pass.
#[derive(Clone, Debug, PartialEq)]
pub struct PostOp {
    pub source: PostBuf,
    pub target: PostBuf,
    pub blend: bool,
    pub tris: Vec<[PostVertex; 3]>,
}

impl PostOp {
    /// A sprite (`PRIM` 6): the rectangle `xy` (x0, y0, x1, y1) textured with the texels `uv`, one colour.
    pub fn sprite(
        source: PostBuf,
        target: PostBuf,
        blend: bool,
        xy: [f32; 4],
        uv: [f32; 4],
        rgba: [u8; 4],
    ) -> Self {
        let v = |x: usize, y: usize| PostVertex {
            xy: [xy[x], xy[y]],
            uv: [uv[x], uv[y]],
            rgba,
        };
        let c = [v(0, 1), v(2, 1), v(0, 3), v(2, 3)];
        Self {
            source,
            target,
            blend,
            tris: vec![[c[0], c[1], c[2]], [c[1], c[2], c[3]]],
        }
    }

    /// Adds a triangle strip (`PRIM` 4).
    pub fn strip(&mut self, verts: &[PostVertex]) {
        for w in verts.windows(3) {
            self.tris.push([w[0], w[1], w[2]]);
        }
    }
}

/// The colour of a packed GS RGBA word.
pub fn rgba(word: u32) -> [u8; 4] {
    word.to_le_bytes()
}
