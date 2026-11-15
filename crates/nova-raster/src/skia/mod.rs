//! Skia CPU backend.
//!
//! Chosen as the production backend because the browser Remotion renders with
//! is built on the same rasteriser: antialiasing, gradients, blurs and text
//! come out the same way without imitation. Only Skia's CPU raster surface is
//! used; no GPU context is ever created.

mod assets;
mod layers;
mod render;
mod text;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nova_core::{
    AssetId, AssetKind, Error, ErrorCode, Limits, NodeId, NodeKind, Rect, Result, Scene,
};
use resvg::usvg;
use skia_safe::{Image, Path, Typeface};

use crate::{FrameRenderer, RasterBackend};

#[derive(Debug, Clone)]
pub struct PrepareOptions {
    /// Allow the host's installed fonts as fallback. Turn off for output that
    /// is identical on every machine (then only template fonts are used).
    pub system_fonts: bool,
    pub limits: Limits,
    /// Render unchanging expensive layers once and reuse their pixels.
    /// Turning it off only costs speed; it exists for comparison and tests.
    pub layer_cache: bool,
    /// Memory the layer cache may hold, shared by all render threads.
    pub layer_cache_bytes: u64,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        PrepareOptions {
            system_fonts: true,
            limits: Limits::default(),
            layer_cache: true,
            layer_cache_bytes: 512 * 1024 * 1024,
        }
    }
}

/// What preparing the assets cost, for the job log.
#[derive(Debug, Clone, Default)]
pub struct PrepareReport {
    pub prepare_ms: f64,
    pub images: usize,
    pub svgs: usize,
    pub fonts: usize,
    /// Bitmaps prepared for image layers (several layers can share one).
    pub image_variants: usize,
    /// Memory held by those bitmaps, including mipmaps.
    pub decoded_bytes: u64,
    /// Nodes that are rendered through the layer cache.
    pub cached_layers: usize,
}

/// The bitmap an image layer draws, prepared for that layer.
pub(crate) struct ImagePlan {
    pub image: Image,
    /// Part of the asset, in the asset's own units, that `image` holds.
    pub covers: Rect,
    /// The bitmap is exactly the region the layer shows: draw all of it.
    pub exact: bool,
}

/// Immutable state shared by every render thread.
pub(crate) struct Shared {
    pub scene: Arc<Scene>,
    /// Layout size of every image/SVG asset, indexed by asset id.
    pub naturals: Vec<Option<(f32, f32)>>,
    /// Indexed by node id; `Some` for image layers that draw something.
    pub image_plans: Vec<Option<ImagePlan>>,
    /// (family name, typeface) for every font asset.
    pub fonts: Vec<(String, Typeface)>,
    pub system_fonts: bool,
    pub layers: layers::LayerCache,
    /// Indexed by node id: the node's drop shadow can be drawn as a blurred
    /// copy of its outline instead of filtering an offscreen layer.
    pub outline_shadow: Vec<bool>,
}

pub struct SkiaBackend {
    shared: Arc<Shared>,
}

/// Identity of a prepared bitmap: asset, source region, pixel size.
type VariantKey = (AssetId, [u32; 4], u32, u32);

impl SkiaBackend {
    /// Decode every asset once and validate everything that can only be
    /// checked by the rasteriser (image data, SVGs, fonts, path syntax).
    /// All problems are reported together.
    pub fn prepare(
        scene: Arc<Scene>,
        opts: &PrepareOptions,
    ) -> std::result::Result<(SkiaBackend, PrepareReport), Vec<Error>> {
        let started = Instant::now();
        let mut errors = Vec::new();
        let mut report = PrepareReport::default();

        // Fonts first: SVG text needs them.
        let mut fonts = Vec::new();
        let mut fontdb = usvg::fontdb::Database::new();
        for asset in scene.assets.iter().filter(|a| a.kind == AssetKind::Font) {
            match assets::load_typeface(&asset.bytes) {
                Ok(typeface) => {
                    fonts.push((
                        asset.family.clone().unwrap_or_else(|| asset.id.clone()),
                        typeface,
                    ));
                    fontdb.load_font_data(asset.bytes.to_vec());
                    report.fonts += 1;
                }
                Err(m) => errors.push(asset_error(&asset.id, &asset.src, m)),
            }
        }
        let needs_system_fonts_for_svg = opts.system_fonts
            && scene
                .assets
                .iter()
                .any(|a| a.kind == AssetKind::Svg && assets::svg_has_text(&a.bytes));
        if needs_system_fonts_for_svg {
            fontdb.load_system_fonts();
        }
        let fontdb = Arc::new(fontdb);

        // Decode rasters and parse SVGs. The decoded pixels only live until
        // the per-layer bitmaps below have been made from them.
        let mut decoded: HashMap<AssetId, assets::Decoded> = HashMap::new();
        let mut svgs: HashMap<AssetId, assets::SvgDoc> = HashMap::new();
        let mut naturals: Vec<Option<(f32, f32)>> = vec![None; scene.assets.len()];
        for (i, asset) in scene.assets.iter().enumerate() {
            let id = i as AssetId;
            match asset.kind {
                AssetKind::Font => {}
                AssetKind::Image => match assets::decode_image(&asset.bytes, &opts.limits) {
                    Ok(img) => {
                        naturals[i] = Some((img.width as f32, img.height as f32));
                        decoded.insert(id, img);
                        report.images += 1;
                    }
                    Err(m) => errors.push(asset_error(&asset.id, &asset.src, m)),
                },
                AssetKind::Svg => match assets::parse_svg(&asset.bytes, Arc::clone(&fontdb)) {
                    Ok(doc) => {
                        naturals[i] = Some(doc.natural);
                        svgs.insert(id, doc);
                        report.svgs += 1;
                    }
                    Err(m) => errors.push(asset_error(&asset.id, &asset.src, m)),
                },
            }
        }

        // The largest scale every node is ever drawn at, animations included.
        let mut max_scale = vec![1.0f32; scene.nodes.len()];
        for &root in &scene.roots {
            collect_max_scales(&scene, root, 1.0, &mut max_scale);
        }

        // One bitmap per (asset, region, size); layers that agree share it.
        let mut variants: HashMap<VariantKey, Image> = HashMap::new();
        let mut originals: HashMap<AssetId, Image> = HashMap::new();
        let mut image_plans: Vec<Option<ImagePlan>> = Vec::with_capacity(scene.nodes.len());
        for (i, node) in scene.nodes.iter().enumerate() {
            let plan = match &node.kind {
                NodeKind::Image { asset, size, fit } => {
                    naturals[*asset as usize].and_then(|natural| {
                        let bx = size.map_or(natural, |s| (s[0], s[1]));
                        let (src, dst) = fit.rects(natural, bx);
                        if src.is_empty() || dst.is_empty() {
                            return None;
                        }
                        // Pixels the shown region occupies on screen at its largest.
                        let want = (dst.width() * max_scale[i], dst.height() * max_scale[i]);
                        let made = if let Some(doc) = svgs.get(asset) {
                            svg_plan(*asset, doc, src, want, &opts.limits, &mut variants)
                        } else {
                            raster_plan(
                                *asset,
                                &decoded[asset],
                                src,
                                want,
                                &mut variants,
                                &mut originals,
                            )
                        };
                        match made {
                            Ok(plan) => Some(plan),
                            Err(m) => {
                                let a = scene.asset(*asset);
                                errors.push(asset_error(&a.id, &a.src, m));
                                None
                            }
                        }
                    })
                }
                _ => None,
            };
            image_plans.push(plan);
        }
        report.image_variants = variants.len() + originals.len();
        report.decoded_bytes = variants
            .values()
            .chain(originals.values())
            .map(|img| {
                img.width() as u64 * img.height() as u64 * 4 * if img.has_mipmaps() { 4 } else { 3 }
                    / 3
            })
            .sum();

        for node in &scene.nodes {
            if let NodeKind::Path { data, .. } = &node.kind {
                let ok =
                    !data.contains('\0') && Path::from_svg(data).is_some_and(|p| !p.is_empty());
                if !ok {
                    errors.push(Error::at(
                        ErrorCode::Validation,
                        format!("layer `{}`", node.id),
                        "`d` is not valid SVG path data (or draws nothing)",
                    ));
                }
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }
        let outline_shadow: Vec<bool> = scene
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| has_outline_shadow(node, image_plans[i].as_ref()))
            .collect();
        let layers = layers::LayerCache::new(&scene, opts.layer_cache, opts.layer_cache_bytes);
        report.cached_layers = layers.isolated_count();
        report.prepare_ms = started.elapsed().as_secs_f64() * 1000.0;
        let shared = Arc::new(Shared {
            scene,
            naturals,
            image_plans,
            fonts,
            system_fonts: opts.system_fonts,
            layers,
            outline_shadow,
        });
        Ok((SkiaBackend { shared }, report))
    }

    pub fn scene(&self) -> &Arc<Scene> {
        &self.shared.scene
    }

    /// Bytes currently held by the layer cache.
    pub fn layer_cache_bytes(&self) -> u64 {
        self.shared.layers.bytes()
    }
}

impl RasterBackend for SkiaBackend {
    fn name(&self) -> &'static str {
        "skia-cpu"
    }

    fn new_renderer(&self) -> Result<Box<dyn FrameRenderer>> {
        Ok(Box::new(render::SkiaRenderer::new(Arc::clone(
            &self.shared,
        ))))
    }

    fn release_frames_before(&self, frame: u32) {
        self.shared.layers.release_before(frame as i64);
    }
}

/// A drop shadow is a blurred copy of what the node covers. When the node is
/// one fully opaque, unblurred shape at full opacity, that is simply its
/// outline, and the shadow can be drawn directly as a blurred shape: no
/// offscreen layer, no filtering of the content. Decided from the scene
/// alone so a node never switches between the two methods mid-video.
fn has_outline_shadow(node: &nova_core::Node, plan: Option<&ImagePlan>) -> bool {
    use nova_core::{BlendMode, Fill};
    let opaque_fill = |fill: &Option<Fill>| match fill {
        Some(Fill::Solid(c)) => c.as_const().is_some_and(|c| c.a >= 1.0),
        Some(Fill::Linear { stops, .. }) | Some(Fill::Radial { stops, .. }) => {
            stops.iter().all(|s| s.color.a >= 1.0)
        }
        None => false,
    };
    node.shadow.is_some()
        && node.blend == BlendMode::Normal
        && node.blur.as_const().is_some_and(|b| *b <= 0.0)
        && node.opacity.as_const().is_some_and(|o| *o >= 1.0)
        && match &node.kind {
            NodeKind::Rect {
                fill, stroke: None, ..
            }
            | NodeKind::Ellipse {
                fill, stroke: None, ..
            } => opaque_fill(fill),
            NodeKind::Image { .. } => plan.is_some_and(|p| p.image.is_opaque()),
            _ => false,
        }
}

fn asset_error(id: &str, src: &str, message: String) -> Error {
    Error::at(ErrorCode::Asset, format!("asset `{id}` ({src})"), message)
}

/// For every node, the largest factor it is ever scaled by on screen: its own
/// scale and every ancestor's, each at its animated maximum.
fn collect_max_scales(scene: &Scene, id: NodeId, parent: f32, out: &mut [f32]) {
    let node = scene.node(id);
    let scale = parent * node.scale_x.max_abs().max(node.scale_y.max_abs());
    out[id as usize] = scale;
    if let NodeKind::Group { children, .. } = &node.kind {
        for &child in children {
            collect_max_scales(scene, child, scale, out);
        }
    }
}

/// SVG: rasterise the shown region from the vectors at the largest size it
/// will be displayed, so it is sharp at any zoom the scene uses.
fn svg_plan(
    asset: AssetId,
    doc: &assets::SvgDoc,
    src: Rect,
    want: (f32, f32),
    limits: &Limits,
    variants: &mut HashMap<VariantKey, Image>,
) -> std::result::Result<ImagePlan, String> {
    let cap = limits.max_svg_raster_side as f32;
    let shrink = (cap / want.0).min(cap / want.1).min(1.0);
    let (w, h) = (
        ((want.0 * shrink).ceil() as u32).max(1),
        ((want.1 * shrink).ceil() as u32).max(1),
    );
    let key = (
        asset,
        [
            src.left.to_bits(),
            src.top.to_bits(),
            src.right.to_bits(),
            src.bottom.to_bits(),
        ],
        w,
        h,
    );
    let image = match variants.get(&key) {
        Some(img) => img.clone(),
        None => {
            let pixels =
                assets::rasterize_svg(doc, (src.left, src.top, src.width(), src.height()), w, h)?;
            let img = assets::image_from_premul(&pixels, w, h, true)?;
            variants.insert(key, img.clone());
            img
        }
    };
    Ok(ImagePlan {
        image,
        covers: src,
        exact: true,
    })
}

/// Raster image: if the layer always shows it smaller than its pixels,
/// prepare a high-quality downscale at the largest size needed. Otherwise
/// (shown at about 1:1 or enlarged) draw from the original pixels.
fn raster_plan(
    asset: AssetId,
    decoded: &assets::Decoded,
    src: Rect,
    want: (f32, f32),
    variants: &mut HashMap<VariantKey, Image>,
    originals: &mut HashMap<AssetId, Image>,
) -> std::result::Result<ImagePlan, String> {
    let (iw, ih) = (decoded.width, decoded.height);
    // Snap the shown region to whole source pixels.
    let x0 = (src.left.round().max(0.0) as u32).min(iw - 1);
    let y0 = (src.top.round().max(0.0) as u32).min(ih - 1);
    let x1 = (src.right.round() as u32).clamp(x0 + 1, iw);
    let y1 = (src.bottom.round() as u32).clamp(y0 + 1, ih);
    let (cw, ch) = (x1 - x0, y1 - y0);
    let (w, h) = ((want.0.ceil() as u32).max(1), (want.1.ceil() as u32).max(1));

    if (w as f32) < cw as f32 * 0.95 && (h as f32) < ch as f32 * 0.95 {
        let key = (asset, [x0, y0, x1, y1], w, h);
        let image = match variants.get(&key) {
            Some(img) => img.clone(),
            None => {
                let pixels = assets::resize(decoded, (x0, y0, cw, ch), w, h);
                let img = assets::image_from_premul(&pixels, w, h, true)?;
                variants.insert(key, img.clone());
                img
            }
        };
        return Ok(ImagePlan {
            image,
            covers: Rect::new(x0 as f32, y0 as f32, x1 as f32, y1 as f32),
            exact: true,
        });
    }
    let image = match originals.get(&asset) {
        Some(img) => img.clone(),
        None => {
            let img = assets::image_from_premul(&decoded.pixels, iw, ih, true)?;
            originals.insert(asset, img.clone());
            img
        }
    };
    Ok(ImagePlan {
        image,
        covers: Rect::from_wh(iw as f32, ih as f32),
        exact: false,
    })
}
