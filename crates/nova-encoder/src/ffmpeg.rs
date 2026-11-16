//! FFmpeg as a child process, fed raw frames over stdin.
//!
//! NOVA never builds a shell command: arguments go to the process as an
//! array, so nothing from a template can be interpreted by a shell. The
//! output is written to a temporary name and renamed only after FFmpeg exits
//! successfully, so a half-written file is never mistaken for a finished one.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::JoinHandle;
use std::time::Instant;

use nova_core::{AudioTrack, Error, ErrorCode, Fps, Result};

use crate::{FrameSink, PixelFormat, yuv};

/// How much of FFmpeg's stderr is kept for error messages.
const STDERR_KEEP_BYTES: usize = 8 * 1024;

/// x264 speed/quality trade-off. Profiles are configuration, not constants:
/// the exact values used are recorded in the job report.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeProfile {
    pub name: String,
    pub preset: String,
    pub crf: u8,
    /// x264 `-tune` (`grain`, `film`, `animation`, ...); `None` leaves it out.
    pub tune: Option<String>,
}

impl EncodeProfile {
    pub const NAMES: &'static [&'static str] = &["preview", "balanced", "high_quality", "master"];
    pub const TUNES: &'static [&'static str] =
        &["film", "animation", "grain", "stillimage", "fastdecode", "zerolatency"];

    pub fn named(name: &str) -> Option<EncodeProfile> {
        let (preset, crf, tune) = match name {
            // quick look; file size and fine detail are not the priority
            "preview" => ("veryfast", 23, None),
            // x264's default preset at CRF 18: the settings to compare
            // against a Remotion render at the same CRF
            "balanced" => ("medium", 18, None),
            "high_quality" => ("slow", 15, None),
            // a file to keep and to upload: platforms encode it again, so it
            // should carry fine texture (film grain, hairlines) instead of
            // smoothing it. `grain` keeps noise as noise; costs bitrate.
            "master" => ("medium", 17, Some("grain")),
            _ => return None,
        };
        Some(EncodeProfile {
            name: name.to_owned(),
            preset: preset.to_owned(),
            crf,
            tune: tune.map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub ffmpeg: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: Fps,
    pub frame_count: u32,
    pub profile: EncodeProfile,
    /// x264 threads; `None` lets x264 decide.
    pub threads: Option<u32>,
    pub audio: Vec<AudioTrack>,
    /// More files encoded from the same frames in the same FFmpeg run, each
    /// resized (a 1080p delivery file next to a 4K master, for example).
    pub extra: Vec<ScaledOutput>,
    /// Encode the main file at this size instead of the frame size (Lanczos).
    /// Frames drawn larger than the delivery size are supersampled this way.
    pub resize: Option<(u32, u32)>,
}

/// A second output: the frames resized to `width` x `height` (Lanczos), with
/// its own encode settings and the same audio.
#[derive(Debug, Clone)]
pub struct ScaledOutput {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub profile: EncodeProfile,
}

impl EncoderConfig {
    /// The full FFmpeg argument list (everything after the program name).
    /// `write_to` is where the main file goes; `extra_to` has one path for
    /// each entry of `extra`.
    pub fn args(&self, write_to: &Path, extra_to: &[PathBuf]) -> Vec<OsString> {
        assert_eq!(extra_to.len(), self.extra.len());
        let mut a: Vec<OsString> = Vec::new();
        let mut push = |s: &str| a.push(OsString::from(s));
        for s in ["-hide_banner", "-loglevel", "error", "-y"] {
            push(s);
        }
        // Input 0: raw I420 frames on stdin, already BT.709 limited range.
        for s in ["-f", "rawvideo", "-pixel_format", "yuv420p"] {
            push(s);
        }
        push("-video_size");
        push(&format!("{}x{}", self.width, self.height));
        push("-framerate");
        push(&self.fps.to_ffmpeg());
        for s in [
            "-color_range",
            "tv",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
        ] {
            push(s);
        }
        for s in ["-i", "pipe:0"] {
            push(s);
        }
        for track in &self.audio {
            a.push(OsString::from("-i"));
            a.push(track.path.clone().into_os_string());
        }

        // One filter graph feeds every output: the video is split and resized
        // for the extra files, the mixed audio is split alongside it.
        let outputs = 1 + self.extra.len();
        // accurate_rnd + full_chroma_int: no shortcuts in the resize; the
        // matrix is passed through untouched (BT.709 in and out).
        let scale = |w: u32, h: u32| format!("scale={w}:{h}:flags=lanczos+accurate_rnd+full_chroma_int");
        let filtered_video = self.resize.is_some() || !self.extra.is_empty();
        let mut graph = String::new();
        if !self.extra.is_empty() {
            let main = if self.resize.is_some() { "s0" } else { "v0" };
            graph.push_str(&format!("[0:v]split={outputs}[{main}]"));
            for i in 1..outputs {
                graph.push_str(&format!("[s{i}]"));
            }
            if let Some((w, h)) = self.resize {
                graph.push_str(&format!(";[s0]{}[v0]", scale(w, h)));
            }
            for (i, e) in self.extra.iter().enumerate() {
                graph.push_str(&format!(";[s{n}]{}[v{n}]", scale(e.width, e.height), n = i + 1));
            }
        } else if let Some((w, h)) = self.resize {
            graph.push_str(&format!("[0:v]{}[v0]", scale(w, h)));
        }
        let audio = audio_filter_graph(&self.audio, self.fps);
        if let Some(audio) = &audio {
            if !graph.is_empty() {
                graph.push(';');
            }
            graph.push_str(audio);
            if outputs > 1 {
                graph.push_str(&format!(";[aout]asplit={outputs}"));
                for i in 0..outputs {
                    graph.push_str(&format!("[a{i}]"));
                }
            }
        }
        let mut push = |s: &str| a.push(OsString::from(s));
        if !graph.is_empty() {
            push("-filter_complex");
            push(&graph);
        }

        let duration = format!("{:.6}", self.fps.frame_to_seconds(self.frame_count as i64));
        let mut files = vec![(write_to, &self.profile)];
        files.extend(extra_to.iter().map(PathBuf::as_path).zip(self.extra.iter().map(|e| &e.profile)));
        for (i, (file, profile)) in files.into_iter().enumerate() {
            let mut push = |s: &str| a.push(OsString::from(s));
            push("-map");
            push(&if filtered_video { format!("[v{i}]") } else { "0:v:0".to_owned() });
            match &audio {
                Some(_) => {
                    push("-map");
                    push(&if outputs > 1 { format!("[a{i}]") } else { "[aout]".to_owned() });
                    for s in ["-c:a", "aac", "-b:a", "192k"] {
                        push(s);
                    }
                }
                None => push("-an"),
            }
            for s in ["-c:v", "libx264", "-preset"] {
                push(s);
            }
            push(&profile.preset);
            push("-crf");
            push(&profile.crf.to_string());
            if let Some(tune) = &profile.tune {
                push("-tune");
                push(tune);
            }
            for s in ["-pix_fmt", "yuv420p"] {
                push(s);
            }
            // Tag the stream so players use the matrix the frames were made with.
            for s in [
                "-color_range",
                "tv",
                "-colorspace",
                "bt709",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
            ] {
                push(s);
            }
            if let Some(t) = self.threads {
                push("-threads");
                push(&t.to_string());
            }
            // Stop at the video's length even if an audio track is longer.
            push("-frames:v");
            push(&self.frame_count.to_string());
            push("-t");
            push(&duration);
            for s in ["-movflags", "+faststart", "-f", "mp4"] {
                push(s);
            }
            a.push(file.as_os_str().to_owned());
        }
        a
    }
}

/// `[1:a]adelay=..,volume=..[a0];...;[a0][a1]amix=..[aout]`, or `None` without audio.
fn audio_filter_graph(tracks: &[AudioTrack], fps: Fps) -> Option<String> {
    if tracks.is_empty() {
        return None;
    }
    let mut graph = String::new();
    for (i, t) in tracks.iter().enumerate() {
        let delay_ms = (fps.frame_to_seconds(t.start_frame.max(0)) * 1000.0).round() as u64;
        let label = if tracks.len() == 1 {
            "aout".to_owned()
        } else {
            format!("a{i}")
        };
        graph.push_str(&format!(
            "[{}:a]adelay={delay_ms}:all=1,volume={:.4}[{label}];",
            i + 1,
            t.volume
        ));
    }
    if tracks.len() > 1 {
        for i in 0..tracks.len() {
            graph.push_str(&format!("[a{i}]"));
        }
        // normalize=0: keep each track at the level the template asked for.
        graph.push_str(&format!(
            "amix=inputs={}:duration=longest:normalize=0[aout]",
            tracks.len()
        ));
    } else {
        graph.pop(); // trailing ';'
    }
    Some(graph)
}

#[derive(Debug, Clone)]
pub struct EncodeReport {
    pub output: PathBuf,
    pub frames_written: u64,
    /// Time spent blocked writing into the pipe: how long the encoder held
    /// the renderer back.
    pub pipe_write_ms: f64,
    /// From spawning FFmpeg to its exit.
    pub total_ms: f64,
    pub output_bytes: u64,
    /// Size of each extra output, in `EncoderConfig::extra` order.
    pub extra_bytes: Vec<u64>,
    pub command: Vec<String>,
}

/// A running FFmpeg encode.
pub struct FfmpegSink {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<String>>,
    tmp: PathBuf,
    /// Temporary file of each extra output, in `config.extra` order.
    extra_tmp: Vec<PathBuf>,
    config: EncoderConfig,
    frame_len: usize,
    frames_written: u64,
    pipe_write_ms: f64,
    started: Instant,
    command: Vec<String>,
    finished: bool,
}

impl FfmpegSink {
    pub fn start(config: EncoderConfig) -> Result<FfmpegSink> {
        if !config.width.is_multiple_of(2) || !config.height.is_multiple_of(2) {
            return Err(Error::new(
                ErrorCode::Encode,
                "yuv420p output needs even width and height",
            ));
        }
        let dir = match config.output.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_owned(),
            _ => PathBuf::from("."),
        };
        if !dir.is_dir() {
            return Err(Error::new(
                ErrorCode::Io,
                format!("output directory `{}` does not exist", dir.display()),
            ));
        }
        let name = config
            .output
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("output.mp4");
        let tmp = dir.join(format!(".{name}.{}.partial", std::process::id()));
        if let Some((w, h)) = config.resize
            && (!w.is_multiple_of(2) || !h.is_multiple_of(2) || w == 0 || h == 0)
        {
            return Err(Error::new(
                ErrorCode::Encode,
                format!("the output size {w}x{h} needs even, non-zero width and height"),
            ));
        }
        let mut extra_tmp = Vec::new();
        for e in &config.extra {
            if !e.width.is_multiple_of(2) || !e.height.is_multiple_of(2) || e.width == 0 || e.height == 0 {
                return Err(Error::new(
                    ErrorCode::Encode,
                    format!("extra output `{}` needs even, non-zero width and height", e.output.display()),
                ));
            }
            let dir = match e.output.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_owned(),
                _ => PathBuf::from("."),
            };
            if !dir.is_dir() {
                return Err(Error::new(
                    ErrorCode::Io,
                    format!("output directory `{}` does not exist", dir.display()),
                ));
            }
            let name = e.output.file_name().and_then(|n| n.to_str()).unwrap_or("output.mp4");
            extra_tmp.push(dir.join(format!(".{name}.{}.partial", std::process::id())));
        }
        let args = config.args(&tmp, &extra_tmp);
        let command: Vec<String> = std::iter::once(config.ffmpeg.to_string_lossy().into_owned())
            .chain(args.iter().map(|a| a.to_string_lossy().into_owned()))
            .collect();

        let mut child = Command::new(&config.ffmpeg)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                Error::new(
                    ErrorCode::Encode,
                    format!("cannot start FFmpeg (`{}`): {e}", config.ffmpeg.display()),
                )
            })?;
        let stdin = child.stdin.take();
        let mut stderr_pipe = child.stderr.take().expect("stderr piped");
        // Drain stderr on its own thread so FFmpeg can never block on a full
        // pipe; keep only the tail.
        let stderr = std::thread::spawn(move || {
            let mut kept: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = stderr_pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > STDERR_KEEP_BYTES * 2 {
                    kept.drain(..kept.len() - STDERR_KEEP_BYTES);
                }
            }
            if kept.len() > STDERR_KEEP_BYTES {
                kept.drain(..kept.len() - STDERR_KEEP_BYTES);
            }
            String::from_utf8_lossy(&kept).trim().to_owned()
        });

        Ok(FfmpegSink {
            child,
            stdin,
            stderr: Some(stderr),
            tmp,
            extra_tmp,
            frame_len: yuv::i420_len(config.width as usize, config.height as usize),
            config,
            frames_written: 0,
            pipe_write_ms: 0.0,
            started: Instant::now(),
            command,
            finished: false,
        })
    }

    /// Remove every partial file.
    fn discard(&self) {
        let _ = std::fs::remove_file(&self.tmp);
        for t in &self.extra_tmp {
            let _ = std::fs::remove_file(t);
        }
    }

    fn collect_stderr(&mut self) -> String {
        self.stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default()
    }

    /// FFmpeg stopped reading: find out why and turn it into one clear error.
    fn failure(&mut self, context: &str) -> Error {
        self.stdin = None;
        let status = self.child.wait();
        let stderr = self.collect_stderr();
        self.discard();
        self.finished = true;
        let status = status
            .map(|s| s.to_string())
            .unwrap_or_else(|e| e.to_string());
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!("\n--- ffmpeg stderr ---\n{stderr}")
        };
        Error::new(
            ErrorCode::Encode,
            format!("{context} (ffmpeg {status}){detail}"),
        )
    }

    /// Close the pipe, wait for FFmpeg and publish the file. Only a zero exit
    /// status and a non-empty file count as success.
    pub fn finish(mut self) -> Result<EncodeReport> {
        self.stdin = None; // EOF tells FFmpeg the stream is complete
        let status = self
            .child
            .wait()
            .map_err(|e| Error::new(ErrorCode::Encode, format!("waiting for FFmpeg: {e}")));
        let stderr = self.collect_stderr();
        self.finished = true;
        let status = match status {
            Ok(s) => s,
            Err(e) => {
                self.discard();
                return Err(e);
            }
        };
        let problem = if !status.success() {
            let detail = if stderr.is_empty() {
                String::new()
            } else {
                format!("\n--- ffmpeg stderr ---\n{stderr}")
            };
            Some(format!("FFmpeg failed ({status}){detail}"))
        } else if self.frames_written != self.config.frame_count as u64 {
            Some(format!(
                "only {} of {} frames were written to the encoder",
                self.frames_written, self.config.frame_count
            ))
        } else {
            None
        };
        if let Some(message) = problem {
            self.discard();
            return Err(Error::new(ErrorCode::Encode, message));
        }
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let output_bytes = size(&self.tmp);
        if output_bytes == 0 || self.extra_tmp.iter().any(|t| size(t) == 0) {
            self.discard();
            return Err(Error::new(
                ErrorCode::Encode,
                "FFmpeg exited successfully but produced an empty file",
            ));
        }
        let mut moves = vec![(self.tmp.clone(), self.config.output.clone())];
        moves.extend(self.extra_tmp.iter().cloned().zip(self.config.extra.iter().map(|e| e.output.clone())));
        let mut extra_bytes = Vec::new();
        for (i, (from, to)) in moves.iter().enumerate() {
            if i > 0 {
                extra_bytes.push(size(from));
            }
            if let Err(e) = std::fs::rename(from, to) {
                self.discard();
                return Err(Error::io(
                    format!("cannot move the finished video to `{}`", to.display()),
                    e,
                ));
            }
        }
        Ok(EncodeReport {
            output: self.config.output.clone(),
            frames_written: self.frames_written,
            pipe_write_ms: self.pipe_write_ms,
            total_ms: self.started.elapsed().as_secs_f64() * 1000.0,
            output_bytes,
            extra_bytes,
            command: self.command.clone(),
        })
    }
}

impl FrameSink for FfmpegSink {
    fn format(&self) -> PixelFormat {
        PixelFormat::Yuv420p
    }

    fn write(&mut self, index: u32, frame: &[u8]) -> Result<()> {
        if frame.len() != self.frame_len {
            return Err(Error::new(
                ErrorCode::Internal,
                format!(
                    "frame {index} is {} bytes, the encoder expects {}",
                    frame.len(),
                    self.frame_len
                ),
            ));
        }
        let t0 = Instant::now();
        let result = match self.stdin.as_mut() {
            Some(stdin) => stdin.write_all(frame),
            None => {
                return Err(Error::new(
                    ErrorCode::Encode,
                    "the encoder is already closed",
                ));
            }
        };
        self.pipe_write_ms += t0.elapsed().as_secs_f64() * 1000.0;
        match result {
            Ok(()) => {
                self.frames_written += 1;
                Ok(())
            }
            Err(e) => Err(self.failure(&format!(
                "FFmpeg stopped accepting frames at frame {index}: {e}"
            ))),
        }
    }
}

impl Drop for FfmpegSink {
    /// Abandoned mid-encode (error or cancellation elsewhere): stop FFmpeg
    /// and leave no partial file behind.
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.collect_stderr();
        self.discard();
    }
}

/// What ffprobe reports about a finished file.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub frame_rate: String,
    pub frames: u64,
    pub color_space: Option<String>,
    pub color_range: Option<String>,
    pub has_audio: bool,
    pub duration_seconds: f64,
}

/// Inspect an output file with ffprobe (counts real packets, so a truncated
/// file is caught).
pub fn probe(ffprobe: &Path, file: &Path) -> Result<ProbeInfo> {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-count_packets", "-show_entries"])
        .arg("stream=codec_type,codec_name,width,height,pix_fmt,r_frame_rate,nb_read_packets,color_space,color_range:format=duration")
        .args(["-of", "json"])
        .arg(file)
        .output()
        .map_err(|e| Error::new(ErrorCode::Encode, format!("cannot run ffprobe (`{}`): {e}", ffprobe.display())))?;
    if !out.status.success() {
        return Err(Error::new(
            ErrorCode::Encode,
            format!(
                "ffprobe rejected `{}`: {}",
                file.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::new(ErrorCode::Encode, format!("unreadable ffprobe output: {e}")))?;
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| {
            Error::new(
                ErrorCode::Encode,
                format!("`{}` has no video stream", file.display()),
            )
        })?;
    let text = |v: &serde_json::Value| v.as_str().map(str::to_owned);
    Ok(ProbeInfo {
        codec: text(&video["codec_name"]).unwrap_or_default(),
        width: video["width"].as_u64().unwrap_or(0) as u32,
        height: video["height"].as_u64().unwrap_or(0) as u32,
        pix_fmt: text(&video["pix_fmt"]).unwrap_or_default(),
        frame_rate: text(&video["r_frame_rate"]).unwrap_or_default(),
        frames: video["nb_read_packets"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        color_space: text(&video["color_space"]),
        color_range: text(&video["color_range"]),
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        duration_seconds: json["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
    })
}

/// Locate an FFmpeg tool: `$NOVA_FFMPEG` / `$NOVA_FFPROBE`, else the name on PATH.
pub fn tool_path(tool: &str) -> PathBuf {
    let var = format!("NOVA_{}", tool.to_uppercase());
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(tool))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(audio: Vec<AudioTrack>) -> EncoderConfig {
        EncoderConfig {
            ffmpeg: "ffmpeg".into(),
            output: "out.mp4".into(),
            width: 1920,
            height: 1080,
            fps: Fps::new(30000, 1001).unwrap(),
            frame_count: 300,
            profile: EncodeProfile::named("balanced").unwrap(),
            threads: Some(4),
            audio,
            extra: Vec::new(),
            resize: None,
        }
    }

    fn joined(c: &EncoderConfig) -> String {
        c.args(Path::new("tmp.partial"), &[])
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn video_only_arguments() {
        let s = joined(&config(vec![]));
        assert!(s.contains(
            "-f rawvideo -pixel_format yuv420p -video_size 1920x1080 -framerate 30000/1001"
        ));
        assert!(s.contains("-i pipe:0"));
        assert!(s.contains("-c:v libx264 -preset medium -crf 18 -pix_fmt yuv420p"));
        assert!(s.contains("-colorspace bt709") && s.contains("-color_range tv"));
        assert!(s.contains("-threads 4") && s.contains("-frames:v 300") && s.contains("-an"));
        assert!(s.ends_with("-movflags +faststart -f mp4 tmp.partial"));
    }

    #[test]
    fn audio_tracks_are_delayed_and_mixed() {
        let track = |p: &str, start, volume| AudioTrack {
            id: p.into(),
            path: p.into(),
            start_frame: start,
            volume,
        };
        let one = joined(&config(vec![track("voice.wav", 0, 1.0)]));
        assert!(one.contains("-i voice.wav"));
        assert!(
            one.contains("[1:a]adelay=0:all=1,volume=1.0000[aout]"),
            "{one}"
        );
        assert!(one.contains("-map [aout] -c:a aac"));

        let two = joined(&config(vec![
            track("voice.wav", 0, 1.0),
            track("music.mp3", 30, 0.35),
        ]));
        // 30 frames at 29.97 fps = 1001 ms
        assert!(
            two.contains("[2:a]adelay=1001:all=1,volume=0.3500[a1]"),
            "{two}"
        );
        assert!(two.contains("[a0][a1]amix=inputs=2:duration=longest:normalize=0[aout]"));
    }

    #[test]
    fn extra_outputs_share_the_frames_and_the_audio() {
        let mut c = config(vec![AudioTrack {
            id: "mix".into(),
            path: "mix.wav".into(),
            start_frame: 0,
            volume: 1.0,
        }]);
        c.width = 2160;
        c.height = 3840;
        c.profile = EncodeProfile::named("master").unwrap();
        c.extra = vec![ScaledOutput {
            output: "small.mp4".into(),
            width: 1080,
            height: 1920,
            profile: EncodeProfile::named("balanced").unwrap(),
        }];
        let args = c.args(Path::new("big.partial"), &[PathBuf::from("small.partial")]);
        let line = args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" ");
        assert!(
            line.contains("[0:v]split=2[v0][s1];[s1]scale=1080:1920:flags=lanczos+accurate_rnd+full_chroma_int[v1]"),
            "{line}"
        );
        assert!(line.contains("[aout];[aout]asplit=2[a0][a1]"), "{line}");
        // each file has its own maps and encode settings, and both carry the BT.709 tags
        let (first, second) = line.split_once("big.partial").unwrap();
        assert!(first.contains("-map [v0] -map [a0]") && first.contains("-preset medium -crf 17 -tune grain"), "{first}");
        assert!(second.contains("-map [v1] -map [a1]") && second.contains("-preset medium -crf 18 -pix_fmt"), "{second}");
        assert!(second.contains("-colorspace bt709") && second.ends_with("small.partial"), "{second}");
        // without extras nothing changes: frames go straight from the pipe
        let plain = joined(&config(vec![]));
        assert!(plain.contains("-map 0:v:0 -an") && !plain.contains("split"), "{plain}");
    }

    #[test]
    fn a_resized_main_output_is_the_only_encode() {
        let mut c = config(vec![]);
        c.width = 2160;
        c.height = 3840;
        c.resize = Some((1080, 1920));
        let line = joined(&c);
        assert!(line.contains("-video_size 2160x3840"), "{line}");
        assert!(
            line.contains("-filter_complex [0:v]scale=1080:1920:flags=lanczos+accurate_rnd+full_chroma_int[v0] -map [v0] -an"),
            "{line}"
        );
        assert_eq!(line.matches("libx264").count(), 1, "{line}");
        // together with an extra file, both come from the split
        c.extra = vec![ScaledOutput {
            output: "small.mp4".into(),
            width: 540,
            height: 960,
            profile: EncodeProfile::named("balanced").unwrap(),
        }];
        let args = c.args(Path::new("big.partial"), &[PathBuf::from("small.partial")]);
        let line = args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" ");
        assert!(line.contains("[0:v]split=2[s0][s1];[s0]scale=1080:1920:"), "{line}");
        assert!(line.contains("[v0];[s1]scale=540:960:"), "{line}");
    }

    #[test]
    fn a_path_with_spaces_stays_one_argument() {
        let mut c = config(vec![]);
        c.audio = vec![AudioTrack {
            id: "a".into(),
            path: "my music; rm -rf.mp3".into(),
            start_frame: 0,
            volume: 1.0,
        }];
        let args = c.args(Path::new("out dir/tmp.partial"), &[]);
        assert!(args.contains(&OsString::from("my music; rm -rf.mp3")));
        assert_eq!(args.last().unwrap(), &OsString::from("out dir/tmp.partial"));
    }

    #[test]
    fn profiles() {
        assert_eq!(EncodeProfile::named("preview").unwrap().preset, "veryfast");
        assert_eq!(EncodeProfile::named("high_quality").unwrap().crf, 15);
        assert!(EncodeProfile::named("ludicrous").is_none());
    }
}
