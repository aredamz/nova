//! NOVA output stage.
//!
//! Frames leave the scheduler through a [`FrameSink`], strictly in order.
//! The production sink pipes raw I420 frames into an FFmpeg process, which
//! owns everything about codecs and containers. The other sinks exist to
//! measure rendering without encoding and to test determinism.

pub mod ffmpeg;
pub mod yuv;

use nova_core::Result;

pub use ffmpeg::{EncodeProfile, EncodeReport, EncoderConfig, FfmpegSink, ProbeInfo, probe};

/// Pixel layout a sink wants its frames in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Tightly packed RGBA8, opaque.
    Rgba,
    /// Planar I420, BT.709 limited range.
    Yuv420p,
}

impl PixelFormat {
    pub fn frame_len(self, width: u32, height: u32) -> usize {
        match self {
            PixelFormat::Rgba => width as usize * height as usize * 4,
            PixelFormat::Yuv420p => yuv::i420_len(width as usize, height as usize),
        }
    }
}

/// Receives finished frames in increasing index order, exactly once each.
pub trait FrameSink {
    fn format(&self) -> PixelFormat;

    /// A blocking `write` is the backpressure signal: while it blocks, the
    /// scheduler stops handing out new frames once its window is full.
    fn write(&mut self, index: u32, frame: &[u8]) -> Result<()>;
}

/// Discards frames. Used to measure render-only throughput.
pub struct NullSink {
    format: PixelFormat,
    pub frames: u64,
}

impl NullSink {
    pub fn new(format: PixelFormat) -> NullSink {
        NullSink { format, frames: 0 }
    }
}

impl FrameSink for NullSink {
    fn format(&self) -> PixelFormat {
        self.format
    }

    fn write(&mut self, _index: u32, _frame: &[u8]) -> Result<()> {
        self.frames += 1;
        Ok(())
    }
}

/// Records a hash of every frame, to compare two renders bit for bit without
/// keeping the pixels.
pub struct HashSink {
    format: PixelFormat,
    pub hashes: Vec<(u32, u64)>,
}

impl HashSink {
    pub fn new(format: PixelFormat) -> HashSink {
        HashSink {
            format,
            hashes: Vec::new(),
        }
    }

    /// One hash over all frame hashes, in order.
    pub fn digest(&self) -> u64 {
        let mut h = FNV_OFFSET;
        for (index, frame_hash) in &self.hashes {
            for b in index
                .to_le_bytes()
                .iter()
                .chain(frame_hash.to_le_bytes().iter())
            {
                h = (h ^ *b as u64).wrapping_mul(FNV_PRIME);
            }
        }
        h
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over 8-byte words (fast enough for full frames, stable everywhere).
pub fn hash_frame(frame: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    let mut chunks = frame.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(FNV_PRIME);
    }
    for &b in chunks.remainder() {
        h = (h ^ b as u64).wrapping_mul(FNV_PRIME);
    }
    h
}

impl FrameSink for HashSink {
    fn format(&self) -> PixelFormat {
        self.format
    }

    fn write(&mut self, index: u32, frame: &[u8]) -> Result<()> {
        self.hashes.push((index, hash_frame(frame)));
        Ok(())
    }
}
