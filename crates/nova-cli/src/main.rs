//! `nova` — the NOVA 2D motion graphics renderer.

mod bench;
mod doctor;
mod load;
mod render;
mod report;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "nova",
    version,
    about = "NOVA — CPU-only 2D motion graphics renderer (JSON template in, MP4 out)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a template completely (structure, references, assets, fonts) without rendering
    Validate(ValidateArgs),
    /// Show what a template contains: layers, timing, assets, data slots, reusable frames
    Inspect(InspectArgs),
    /// Render one frame to a PNG
    Preview(PreviewArgs),
    /// Render evenly spaced frames onto one contact-sheet PNG
    Sheet(SheetArgs),
    /// Render the composition to an MP4
    Render(render::RenderArgs),
    /// Measure render-only and end-to-end throughput across thread counts
    Benchmark(bench::BenchArgs),
    /// Report CPU, memory, FFmpeg and font setup of this machine
    Doctor(DoctorArgs),
    /// Print the JSON Schema of the template format
    Schema,
}

/// Options shared by every command that reads a template.
#[derive(Args, Clone)]
pub struct InputArgs {
    /// Template JSON file
    pub template: PathBuf,
    /// JSON file with values for the template's {{slots}}
    #[arg(long, value_name = "FILE")]
    pub data: Option<PathBuf>,
    /// Folder asset paths are resolved against [default: the template's folder]
    #[arg(long, value_name = "DIR")]
    pub assets_root: Option<PathBuf>,
    /// Use only fonts listed in the template; never fall back to this machine's fonts
    #[arg(long)]
    pub no_system_fonts: bool,
    /// Draw everything at this multiple of the template's size (2 turns 1080x1920 into 2160x3840).
    /// Shapes, text and baked frames are drawn again at the larger size, not enlarged
    #[arg(long, value_name = "FACTOR", default_value_t = 1.0)]
    pub scale: f32,
}

#[derive(Args)]
struct ValidateArgs {
    #[command(flatten)]
    input: InputArgs,
    /// Machine-readable result on stdout
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct InspectArgs {
    #[command(flatten)]
    input: InputArgs,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct PreviewArgs {
    #[command(flatten)]
    input: InputArgs,
    /// Frame index to render
    #[arg(long, default_value_t = 0)]
    frame: u32,
    /// PNG file to write
    #[arg(long, short, value_name = "FILE")]
    output: PathBuf,
}

#[derive(Args)]
struct SheetArgs {
    #[command(flatten)]
    input: InputArgs,
    /// How many frames to show, spread evenly over the composition
    #[arg(long, default_value_t = 12)]
    count: u32,
    /// Explicit frame indices instead of --count, e.g. 0,15,30
    #[arg(long, value_delimiter = ',', value_name = "N,N,...")]
    frames: Vec<u32>,
    #[arg(long, default_value_t = 4)]
    columns: u32,
    /// Width of each frame on the sheet, in pixels
    #[arg(long, default_value_t = 480)]
    cell_width: u32,
    #[arg(long, short, value_name = "FILE")]
    output: PathBuf,
}

#[derive(Args)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,
}

/// Exit codes: 0 ok, 1 runtime failure, 2 the template (or data) is invalid.
fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Validate(a) => load::cmd_validate(&a.input, a.json),
        Command::Inspect(a) => load::cmd_inspect(&a.input, a.json),
        Command::Preview(a) => render::cmd_preview(&a.input, a.frame, &a.output),
        Command::Sheet(a) => render::cmd_sheet(
            &a.input,
            a.count,
            &a.frames,
            a.columns,
            a.cell_width,
            &a.output,
        ),
        Command::Render(a) => render::cmd_render(a),
        Command::Benchmark(a) => bench::cmd_benchmark(a),
        Command::Doctor(a) => doctor::cmd_doctor(a.json),
        Command::Schema => {
            println!(
                "{}",
                serde_json::to_string_pretty(&nova_template::json_schema()).expect("schema")
            );
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            failure.print();
            ExitCode::from(failure.exit_code())
        }
    }
}
