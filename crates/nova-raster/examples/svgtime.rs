//! Time resvg on one SVG file: `cargo run --release --example svgtime -- file.svg [out.png]`
use resvg::{tiny_skia, usvg};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).expect("read svg");
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let t0 = Instant::now();
    let tree = usvg::Tree::from_data(&data, &opt).expect("parse");
    let t1 = Instant::now();
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    println!(
        "parse {:.1} ms, raster {:.1} ms",
        (t1 - t0).as_secs_f64() * 1e3,
        t1.elapsed().as_secs_f64() * 1e3
    );
    if let Some(out) = args.get(2) {
        pixmap.save_png(out).unwrap();
    }
}
