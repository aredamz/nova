//! The `nova` binary from the outside: commands, output files and exit codes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn nova() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nova"))
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn example() -> (PathBuf, PathBuf) {
    let dir = repo().join("examples/product-ad");
    (dir.join("template.json"), dir.join("data.json"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn run(cmd: &mut Command) -> Output {
    cmd.output().expect("run nova")
}

fn write_template(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("t.json");
    std::fs::write(&path, body).unwrap();
    path
}

const SMALL: &str = r##"{
  "schema_version": "1.0",
  "composition": { "id": "small", "width": 320, "height": 180, "fps": 30, "duration_frames": 45, "background": "#202040" },
  "layers": [
    { "id": "dot", "type": "ellipse", "size": { "width": 60, "height": 60 }, "fill": "#ffd166",
      "transform": { "position": { "x": 40, "y": 90 }, "anchor": { "x": 0.5, "y": 0.5 } },
      "animations": [ { "property": "transform.position.x",
        "keyframes": [ { "frame": 0, "value": 40, "easing": "ease_in_out_cubic" }, { "frame": 30, "value": 280 } ] } ] }
  ]
}"##;

#[test]
fn validate_accepts_the_example_and_reports_json() {
    let (template, data) = example();
    let out = run(nova()
        .arg("validate")
        .arg(&template)
        .arg("--data")
        .arg(&data)
        .arg("--no-system-fonts")
        .arg("--json"));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["composition"]["duration_frames"], 450);
    assert_eq!(
        json["warnings"].as_array().unwrap().len(),
        0,
        "the example bundles its fonts"
    );
}

#[test]
fn validate_lists_every_problem_and_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let bad = write_template(
        dir.path(),
        r##"{ "schema_version": "1.0",
             "composition": { "id": "x", "width": 321, "height": 180, "fps": 30, "duration_frames": 10 },
             "layers": [
               { "id": "a", "type": "rect", "fill": "#zzz" },
               { "id": "a", "type": "image", "asset": "missing" } ] }"##,
    );
    let out = run(nova().arg("validate").arg(&bad));
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    for needle in [
        "must be even",
        "needs `size`",
        "duplicate layer id `a`",
        "no asset with id `missing`",
    ] {
        assert!(err.contains(needle), "missing `{needle}` in:\n{err}");
    }

    let json_out = run(nova().arg("validate").arg(&bad).arg("--json"));
    assert_eq!(json_out.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(json["ok"], false);
    assert!(json["errors"].as_array().unwrap().len() >= 4);
    assert_eq!(json["errors"][0]["retryable"], false);
}

#[test]
fn missing_data_is_reported_before_rendering() {
    let (template, _) = example();
    let dir = tempfile::tempdir().unwrap();
    let out_file = dir.path().join("never.mp4");
    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&out_file)
        .arg("-q"));
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("no value for data slot `product.name`"));
    assert!(!out_file.exists());
}

#[test]
fn inspect_lists_required_data_slots() {
    let (template, data) = example();
    let needs = run(nova().arg("inspect").arg(&template));
    assert!(needs.status.success());
    assert!(text(&needs.stdout).contains("{{product.price}}"));

    let full = run(nova()
        .arg("inspect")
        .arg(&template)
        .arg("--data")
        .arg(&data)
        .arg("--no-system-fonts"));
    assert!(full.status.success(), "{}", text(&full.stderr));
    let s = text(&full.stdout);
    assert!(
        s.contains("1920x1080") && s.contains("photo-shadow") && s.contains("Kasut Larian Aero"),
        "{s}"
    );
}

#[test]
fn preview_and_sheet_write_pngs() {
    let dir = tempfile::tempdir().unwrap();
    let template = write_template(dir.path(), SMALL);
    let png = dir.path().join("f.png");
    let out = run(nova()
        .arg("preview")
        .arg(&template)
        .arg("--frame")
        .arg("15")
        .arg("-o")
        .arg(&png));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let bytes = std::fs::read(&png).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");

    let sheet = dir.path().join("sheet.png");
    let out = run(nova()
        .arg("sheet")
        .arg(&template)
        .arg("--count")
        .arg("6")
        .arg("--columns")
        .arg("3")
        .arg("-o")
        .arg(&sheet));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(std::fs::metadata(&sheet).unwrap().len() > 1000);

    let beyond = run(nova()
        .arg("preview")
        .arg(&template)
        .arg("--frame")
        .arg("45")
        .arg("-o")
        .arg(&png));
    assert_eq!(
        beyond.status.code(),
        Some(2),
        "frame 45 does not exist in a 45-frame video"
    );
}

#[test]
fn render_writes_a_verified_mp4_and_a_json_report() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipped: ffmpeg not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let template = write_template(dir.path(), SMALL);
    let mp4 = dir.path().join("out.mp4");
    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&mp4)
        .args(["--profile", "preview", "--threads", "2", "--json"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["frames"], 45);
    assert_eq!(report["verified_frames"], 45);
    assert_eq!(report["backend"], "skia-cpu");
    assert_eq!(report["encode"]["preset"], "veryfast");
    // frames 30..45 are still: 14 of them repeat frame 30
    assert_eq!(report["render"]["frames_reused"], 14);
    assert!(mp4.exists());

    // a frame range renders just that range
    let part = dir.path().join("part.mp4");
    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&part)
        .args([
            "--start",
            "10",
            "--end",
            "20",
            "--json",
            "--profile",
            "preview",
        ]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["verified_frames"], 10);
}

#[test]
fn scale_draws_larger_and_also_writes_the_template_size() {
    let dir = tempfile::tempdir().unwrap();
    let template = write_template(dir.path(), SMALL);
    // a preview at twice the size
    let png = dir.path().join("big.png");
    let out = run(nova()
        .arg("preview")
        .arg(&template)
        .args(["--scale", "2", "--frame", "0", "-o"])
        .arg(&png));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let bytes = std::fs::read(&png).unwrap();
    let dim = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!((dim(16), dim(20)), (640, 360));

    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipped: ffmpeg not installed");
        return;
    }
    let (big, small) = (dir.path().join("big.mp4"), dir.path().join("small.mp4"));
    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&big)
        .arg("--also")
        .arg(&small)
        .args(["--scale", "2", "--profile", "master", "--preset", "veryfast", "--threads", "2", "--json"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["composition"]["width"], 640);
    assert_eq!(report["encode"]["tune"], "grain");
    assert_eq!(report["encode"]["also"]["width"], 320);
    assert_eq!(report["encode"]["also"]["height"], 180);
    assert_eq!(report["verified_frames"], 45);
    let size = |p: &Path| {
        let o = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-count_packets"])
            .args(["-show_entries", "stream=width,height,nb_read_packets", "-of", "csv=p=0"])
            .arg(p)
            .output()
            .unwrap();
        text(&o.stdout).trim().to_owned()
    };
    assert_eq!(size(&big), "640,360,45");
    assert_eq!(size(&small), "320,180,45");

    // --supersample: drawn at 2x, only the template-size file is written
    let only = dir.path().join("only.mp4");
    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&only)
        .args(["--scale", "2", "--supersample", "--preset", "veryfast", "--threads", "2", "--json"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["composition"]["width"], 640);
    assert_eq!(report["output_size"]["width"], 320);
    assert_eq!(size(&only), "320,180,45");
    // without --scale there is nothing to reduce
    let out = run(nova().arg("render").arg(&template).arg("-o").arg(&only).arg("--supersample"));
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("--scale above 1"), "{}", text(&out.stderr));

    // --also at the output's own size is a mistake, not a silent duplicate
    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&big)
        .arg("--also")
        .arg(&small));
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("same size"), "{}", text(&out.stderr));
}

#[test]
fn bad_options_are_rejected_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let template = write_template(dir.path(), SMALL);
    let mp4 = dir.path().join("o.mp4");
    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&mp4)
        .args(["--profile", "warp"]));
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("unknown profile `warp`"));

    let out = run(nova()
        .arg("render")
        .arg(&template)
        .arg("-o")
        .arg(&mp4)
        .args(["--start", "40", "--end", "40"]));
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("is empty"));

    let out = run(nova().arg("validate").arg(dir.path().join("nope.json")));
    assert_eq!(
        out.status.code(),
        Some(1),
        "an unreadable file is an I/O failure, not a template error"
    );
}

#[test]
fn schema_and_doctor_run() {
    let schema = run(nova().arg("schema"));
    assert!(schema.status.success());
    let json: serde_json::Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert!(json["properties"]["layers"].is_object());

    let doctor = run(nova().arg("doctor").arg("--json"));
    assert!(doctor.status.success());
    let json: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(json["backend"], "skia-cpu");
    assert!(json["logical_cpus"].as_u64().unwrap() >= 1);
}

#[test]
fn the_checked_in_schema_is_current() {
    let out = run(nova().arg("schema"));
    let generated: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let on_disk: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo().join("schemas/template-v1.schema.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        generated, on_disk,
        "run `nova schema > schemas/template-v1.schema.json`"
    );
}
