//! `nova doctor`: what this machine offers the renderer.
//!
//! A vCPU is not a dedicated core. This reports what is actually available —
//! CPU quota, steal time, SIMD features — so thread counts and benchmark
//! results can be interpreted honestly.

use std::process::Command;

use nova_encoder::ffmpeg::tool_path;
use nova_scheduler::available_threads;
use serde_json::{Value, json};

use crate::report::{Failure, human_bytes};

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn cpu_model() -> Option<String> {
    let info = read("/proc/cpuinfo")?;
    let line = info.lines().find(|l| l.starts_with("model name"))?;
    Some(line.split(':').nth(1)?.trim().to_owned())
}

fn meminfo(key: &str) -> Option<u64> {
    let info = read("/proc/meminfo")?;
    let line = info.lines().find(|l| l.starts_with(key))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
}

/// cgroup v2 CPU limit in CPUs (e.g. 4.0), if one is set.
fn cpu_quota() -> Option<f64> {
    let raw = read("/sys/fs/cgroup/cpu.max")?;
    let mut parts = raw.split_whitespace();
    let (quota, period) = (parts.next()?, parts.next()?);
    if quota == "max" {
        return None;
    }
    Some(quota.parse::<f64>().ok()? / period.parse::<f64>().ok()?)
}

/// cgroup v2 memory limit in bytes, if one is set.
fn memory_limit() -> Option<u64> {
    read("/sys/fs/cgroup/memory.max")?.trim().parse().ok()
}

/// Share of CPU time stolen by the hypervisor since boot, in percent.
fn steal_percent() -> Option<f64> {
    let stat = read("/proc/stat")?;
    let line = stat.lines().next()?;
    let fields: Vec<f64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    if fields.len() < 8 {
        return None;
    }
    let total: f64 = fields.iter().take(8).sum();
    (total > 0.0).then(|| fields[7] * 100.0 / total)
}

fn simd_features() -> Vec<&'static str> {
    let mut out = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        for (name, present) in [
            ("sse4.2", std::arch::is_x86_feature_detected!("sse4.2")),
            ("avx", std::arch::is_x86_feature_detected!("avx")),
            ("avx2", std::arch::is_x86_feature_detected!("avx2")),
            ("fma", std::arch::is_x86_feature_detected!("fma")),
            ("avx512f", std::arch::is_x86_feature_detected!("avx512f")),
        ] {
            if present {
                out.push(name);
            }
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            out.push("neon");
        }
    }
    out
}

fn first_line(tool: &str, arg: &str) -> Option<String> {
    let out = Command::new(tool_path(tool)).arg(arg).output().ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .to_owned()
    })
}

fn has_encoder(name: &str) -> bool {
    Command::new(tool_path("ffmpeg"))
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.split_whitespace().nth(1) == Some(name))
        })
        .unwrap_or(false)
}

fn temp_writable() -> bool {
    let probe = std::env::temp_dir().join(format!("nova-doctor-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

pub fn cmd_doctor(json: bool) -> Result<(), Failure> {
    let cpus = available_threads();
    let quota = cpu_quota();
    let steal = steal_percent();
    let simd = simd_features();
    let ffmpeg = first_line("ffmpeg", "-version");
    let ffprobe = first_line("ffprobe", "-version");
    let x264 = ffmpeg.is_some() && has_encoder("libx264");
    let fonts = nova_raster::sheet::system_font_family_count();
    let mem_total = meminfo("MemTotal:");
    let mem_available = meminfo("MemAvailable:");
    let mem_limit = memory_limit();
    let tmp_ok = temp_writable();

    let mut problems: Vec<String> = Vec::new();
    if ffmpeg.is_none() {
        problems.push("ffmpeg was not found: install it or set NOVA_FFMPEG (rendering to MP4 is impossible without it)".into());
    } else if !x264 {
        problems.push("this ffmpeg has no libx264 encoder: H.264 output will fail".into());
    }
    if ffprobe.is_none() {
        problems.push("ffprobe was not found: finished videos cannot be verified".into());
    }
    if !tmp_ok {
        problems.push(format!(
            "the temp directory {} is not writable",
            std::env::temp_dir().display()
        ));
    }
    if let Some(q) = quota
        && q < cpus as f64
    {
        problems.push(format!(
            "CPU quota is {q:.1} CPUs but {cpus} are visible: use --threads {} or renders will be throttled",
            q.floor().max(1.0)
        ));
    }
    if let Some(s) = steal
        && s > 5.0
    {
        problems.push(format!("{s:.1}% of CPU time has been stolen by the hypervisor: benchmarks on this host will vary"));
    }
    if cfg!(target_arch = "x86_64") && !simd.contains(&"avx2") {
        problems.push("no AVX2: rasterisation falls back to slower SSE code paths".into());
    }

    if json {
        let out: Value = json!({
            "nova_version": env!("CARGO_PKG_VERSION"),
            "backend": "skia-cpu",
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpu_model": cpu_model(),
            "logical_cpus": cpus,
            "cpu_quota": quota,
            "cpu_steal_percent": steal,
            "simd": simd,
            "memory_total_bytes": mem_total,
            "memory_available_bytes": mem_available,
            "memory_limit_bytes": mem_limit,
            "ffmpeg": ffmpeg,
            "ffprobe": ffprobe,
            "libx264": x264,
            "system_font_families": fonts,
            "temp_dir": std::env::temp_dir(),
            "temp_dir_writable": tmp_ok,
            "problems": problems,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    let or_unknown = |v: Option<String>| v.unwrap_or_else(|| "unknown".into());
    println!(
        "NOVA {}  (raster backend: Skia CPU, no GPU used)",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "  system     {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("  cpu        {}", or_unknown(cpu_model()));
    println!(
        "  threads    {cpus} logical{}",
        quota
            .map(|q| format!(", cgroup quota {q:.1} CPUs"))
            .unwrap_or_else(|| ", no cgroup quota".into())
    );
    println!(
        "  simd       {}",
        if simd.is_empty() {
            "none detected".into()
        } else {
            simd.join(" ")
        }
    );
    println!(
        "  steal      {}",
        steal
            .map(|s| format!("{s:.2}% since boot"))
            .unwrap_or_else(|| "unknown".into())
    );
    println!(
        "  memory     {} total, {} available{}",
        or_unknown(mem_total.map(human_bytes)),
        or_unknown(mem_available.map(human_bytes)),
        mem_limit
            .map(|l| format!(", cgroup limit {}", human_bytes(l)))
            .unwrap_or_default()
    );
    println!(
        "  ffmpeg     {}",
        ffmpeg.unwrap_or_else(|| "NOT FOUND".into())
    );
    println!(
        "  libx264    {}",
        if x264 { "available" } else { "NOT AVAILABLE" }
    );
    println!(
        "  ffprobe    {}",
        ffprobe.unwrap_or_else(|| "NOT FOUND".into())
    );
    println!(
        "  fonts      {fonts} system families (templates should still bundle their own fonts)"
    );
    println!(
        "  temp dir   {} ({})",
        std::env::temp_dir().display(),
        if tmp_ok { "writable" } else { "NOT writable" }
    );
    if problems.is_empty() {
        println!("no problems found");
    } else {
        println!("problems:");
        for p in &problems {
            println!("  - {p}");
        }
    }
    Ok(())
}
