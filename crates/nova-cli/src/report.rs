//! Error reporting and small formatting helpers shared by the commands.

use nova_core::{Error, ErrorCode};
use serde_json::{Value, json};

/// Why a command failed.
pub struct Failure {
    pub errors: Vec<Error>,
    /// Print as JSON on stdout instead of text on stderr.
    pub json: bool,
}

impl Failure {
    pub fn one(error: Error) -> Failure {
        Failure {
            errors: vec![error],
            json: false,
        }
    }

    pub fn many(errors: Vec<Error>) -> Failure {
        Failure {
            errors,
            json: false,
        }
    }

    pub fn with_json(mut self, json: bool) -> Failure {
        self.json = json;
        self
    }

    /// 2 when the input is at fault (fix the template), 1 otherwise.
    pub fn exit_code(&self) -> u8 {
        let input_fault = |c: ErrorCode| {
            matches!(
                c,
                ErrorCode::InvalidTemplate
                    | ErrorCode::UnsupportedVersion
                    | ErrorCode::Validation
                    | ErrorCode::MissingData
                    | ErrorCode::Asset
                    | ErrorCode::Limit
            )
        };
        if !self.errors.is_empty() && self.errors.iter().all(|e| input_fault(e.code)) {
            2
        } else {
            1
        }
    }

    pub fn print(&self) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({ "ok": false, "errors": errors_json(&self.errors) })
                )
                .unwrap()
            );
            return;
        }
        for e in &self.errors {
            match &e.path {
                Some(p) => eprintln!("error[{}] {p}: {}", e.code.as_str(), e.message),
                None => eprintln!("error[{}] {}", e.code.as_str(), e.message),
            }
        }
        if self.errors.len() > 1 {
            eprintln!("{} errors", self.errors.len());
        }
    }
}

impl From<Error> for Failure {
    fn from(e: Error) -> Failure {
        Failure::one(e)
    }
}

impl From<Vec<Error>> for Failure {
    fn from(errors: Vec<Error>) -> Failure {
        Failure::many(errors)
    }
}

pub fn errors_json(errors: &[Error]) -> Value {
    Value::Array(
        errors
            .iter()
            .map(|e| json!({ "code": e.code.as_str(), "path": e.path, "message": e.message, "retryable": e.code.is_retryable() }))
            .collect(),
    )
}

/// Peak resident memory of this process (Linux), in bytes.
pub fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

pub fn human_duration(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        let whole = seconds.round() as u64;
        format!("{}m{:02}s", whole / 60, whole % 60)
    }
}

/// Round for JSON reports: three decimals is plenty for milliseconds.
pub fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_separate_bad_input_from_runtime_failures() {
        let bad_input = Failure::many(vec![
            Error::new(ErrorCode::Validation, "x"),
            Error::new(ErrorCode::MissingData, "y"),
        ]);
        assert_eq!(bad_input.exit_code(), 2);
        assert_eq!(
            Failure::one(Error::new(ErrorCode::Encode, "ffmpeg died")).exit_code(),
            1
        );
        assert_eq!(
            Failure::one(Error::new(ErrorCode::Cancelled, "stop")).exit_code(),
            1
        );
    }

    #[test]
    fn formatting() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(8_294_400), "7.9 MiB");
        assert_eq!(human_duration(9.44), "9.4s");
        assert_eq!(human_duration(125.0), "2m05s");
        assert_eq!(r3(1.23456), 1.235);
    }
}
