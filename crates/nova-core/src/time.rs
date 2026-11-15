/// Frame rate as an exact rational, e.g. 30/1 or 30000/1001.
///
/// The integer frame index is the single source of truth for time; seconds are
/// always derived from it, never accumulated, so there is no drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fps {
    pub num: u32,
    pub den: u32,
}

impl Fps {
    pub fn new(num: u32, den: u32) -> Option<Fps> {
        (num > 0 && den > 0).then_some(Fps { num, den })
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Presentation time of frame `n` in seconds: `n * den / num`.
    pub fn frame_to_seconds(self, frame: i64) -> f64 {
        (frame as f64 * self.den as f64) / self.num as f64
    }

    /// Duration of `frames` frames, in seconds.
    pub fn frames_to_duration(self, frames: f64) -> f64 {
        frames * self.den as f64 / self.num as f64
    }

    /// `num/den` as FFmpeg wants it for `-r`.
    pub fn to_ffmpeg(self) -> String {
        if self.den == 1 {
            self.num.to_string()
        } else {
            format!("{}/{}", self.num, self.den)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero() {
        assert!(Fps::new(0, 1).is_none());
        assert!(Fps::new(30, 0).is_none());
    }

    #[test]
    fn no_drift_over_long_durations() {
        let fps = Fps::new(30000, 1001).unwrap();
        // one hour of NTSC video: 107892 frames -> exactly 3599.9964 s
        let t = fps.frame_to_seconds(107_892);
        assert!((t - 107_892.0 * 1001.0 / 30000.0).abs() < 1e-9);
        assert_eq!(Fps::new(30, 1).unwrap().frame_to_seconds(450), 15.0);
    }

    #[test]
    fn ffmpeg_rate() {
        assert_eq!(Fps::new(30, 1).unwrap().to_ffmpeg(), "30");
        assert_eq!(Fps::new(30000, 1001).unwrap().to_ffmpeg(), "30000/1001");
    }
}
