//! Template -> Scene: semantic validation and normalisation.
//!
//! serde has already checked the structure. This pass checks everything a
//! schema cannot (references, ranges, which field belongs to which layer
//! type, resource limits) and collects *all* problems instead of stopping at
//! the first, so a template author — often an AI agent — can fix them in one
//! round. Nothing is rasterised here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use nova_core::{
    Asset, AssetId, AssetKind, AudioTrack, BlendMode, Color, Easing, Error, ErrorCode, Fill, Fit,
    Fps, FrameEntry, FramePack, GradientStop, Key, Lerp, Limits, LineCap, LineJoin, Node, NodeId,
    NodeKind, Prop, Scene, Shadow, Stroke, TextAlign, TextNode, Track,
};

use crate::assets::{content_hash, read_asset, resolve_asset_path};
use crate::dsl::*;

pub const SUPPORTED_VERSIONS: &[&str] = &["1.0", "1"];

#[derive(Debug, Clone)]
pub struct CompileOptions {
    /// Directory asset paths are resolved against.
    pub asset_root: PathBuf,
    pub limits: Limits,
}

/// Something that renders, but probably not the way the author intended.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub path: String,
    pub message: String,
}

#[derive(Debug)]
pub struct Compiled {
    pub scene: Scene,
    pub warnings: Vec<Warning>,
}

pub fn compile(template: &Template, opts: &CompileOptions) -> Result<Compiled, Vec<Error>> {
    let mut c = Compiler {
        opts,
        errors: Vec::new(),
        warnings: Vec::new(),
        nodes: Vec::new(),
        ids: HashSet::new(),
        assets: Vec::new(),
        asset_index: HashMap::new(),
        used_assets: HashSet::new(),
        font_families: HashSet::new(),
        layer_count: 0,
    };

    if !SUPPORTED_VERSIONS.contains(&template.schema_version.as_str()) {
        c.errors.push(Error::at(
            ErrorCode::UnsupportedVersion,
            "schema_version",
            format!(
                "version `{}` is not supported; this build understands \"1.0\"",
                template.schema_version
            ),
        ));
        return Err(c.errors);
    }

    let comp = &template.composition;
    let fps = c.composition(comp);
    let background = match &comp.background {
        None => Color::BLACK,
        Some(s) => {
            let col = c.color(s, "composition.background");
            // The output is always opaque: flatten a translucent background onto black.
            Color {
                r: col.r * col.a,
                g: col.g * col.a,
                b: col.b * col.a,
                a: 1.0,
            }
        }
    };

    c.compile_assets(&template.assets);
    let audio = c.compile_audio(&template.audio);

    let root = Scope {
        origin: 0,
        start: 0,
        end: comp.duration_frames as i64,
    };
    let roots = c.compile_layers(&template.layers, "layers", root, 1);

    for (i, def) in template.assets.iter().enumerate() {
        if def.kind != AssetType::Font
            && !c.used_assets.contains(&def.id)
            && c.asset_index.contains_key(&def.id)
        {
            c.warn(
                format!("assets[{i}]"),
                format!("asset `{}` is never used by a layer", def.id),
            );
        }
    }

    if !c.errors.is_empty() {
        return Err(c.errors);
    }
    let scene = Scene {
        id: comp.id.clone(),
        name: comp.name.clone(),
        width: comp.width,
        height: comp.height,
        fps: fps.expect("fps validated"),
        duration_frames: comp.duration_frames,
        background,
        nodes: c.nodes,
        roots,
        assets: c.assets,
        audio,
    };
    Ok(Compiled {
        scene,
        warnings: c.warnings,
    })
}

/// `frames.json` of a frame pack, as written by the baker.
#[derive(serde::Deserialize)]
struct PackManifest {
    nova_frames: u32,
    width: u32,
    height: u32,
    shards: Vec<String>,
    #[serde(default)]
    shaders: HashMap<String, String>,
    /// Per frame: [shard, offset, compressed length, raw length, content id].
    index: Vec<[u64; 5]>,
}

/// Time window a layer inherits from its parent, in composition frames.
#[derive(Clone, Copy)]
struct Scope {
    origin: i64,
    start: i64,
    end: i64,
}

struct Compiler<'a> {
    opts: &'a CompileOptions,
    errors: Vec<Error>,
    warnings: Vec<Warning>,
    nodes: Vec<Node>,
    ids: HashSet<String>,
    assets: Vec<Asset>,
    asset_index: HashMap<String, (AssetId, AssetKind)>,
    used_assets: HashSet<String>,
    /// Lower-cased families provided by font assets.
    font_families: HashSet<String>,
    layer_count: usize,
}

impl Compiler<'_> {
    fn err(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(Error::validation(path, message));
    }

    fn limit(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(Error::at(ErrorCode::Limit, path, message));
    }

    fn warn(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(Warning {
            path: path.into(),
            message: message.into(),
        });
    }

    fn color(&mut self, s: &str, path: &str) -> Color {
        match Color::parse(s) {
            Ok(c) => c,
            Err(m) => {
                self.err(path, m);
                Color::BLACK
            }
        }
    }

    fn composition(&mut self, comp: &Composition) -> Option<Fps> {
        let limits = &self.opts.limits.clone();
        if comp.id.trim().is_empty() {
            self.err("composition.id", "must not be empty");
        }
        if comp.width == 0 || comp.height == 0 {
            self.err("composition", "width and height must be greater than 0");
        } else {
            if !comp.width.is_multiple_of(2) || !comp.height.is_multiple_of(2) {
                self.err(
                    "composition",
                    format!(
                        "width and height must be even ({}x{} given): H.264 yuv420p cannot store odd dimensions",
                        comp.width, comp.height
                    ),
                );
            }
            if comp.width > limits.max_width || comp.height > limits.max_height {
                self.limit(
                    "composition",
                    format!(
                        "{}x{} exceeds the maximum of {}x{}",
                        comp.width, comp.height, limits.max_width, limits.max_height
                    ),
                );
            }
            let pixels = comp.width as u64 * comp.height as u64;
            if pixels > limits.max_total_pixels {
                self.limit(
                    "composition",
                    format!(
                        "{pixels} pixels per frame exceeds the limit of {}",
                        limits.max_total_pixels
                    ),
                );
            }
        }
        if comp.duration_frames == 0 {
            self.err("composition.duration_frames", "must be at least 1");
        } else if comp.duration_frames > limits.max_duration_frames {
            self.limit(
                "composition.duration_frames",
                format!(
                    "{} frames exceeds the limit of {}",
                    comp.duration_frames, limits.max_duration_frames
                ),
            );
        }
        let (num, den) = match comp.fps {
            FpsDef::Whole(n) => (n, 1),
            FpsDef::Rational { num, den } => (num, den),
        };
        let fps = Fps::new(num, den);
        match fps {
            None => self.err(
                "composition.fps",
                "numerator and denominator must be greater than 0",
            ),
            Some(f) if f.as_f64() > 240.0 => {
                self.err("composition.fps", "frame rates above 240 are not supported")
            }
            _ => {}
        }
        fps.or(Fps::new(30, 1))
    }

    fn compile_assets(&mut self, defs: &[AssetDef]) {
        let limits = self.opts.limits.clone();
        if defs.len() > limits.max_assets {
            self.limit(
                "assets",
                format!(
                    "{} assets exceeds the limit of {}",
                    defs.len(),
                    limits.max_assets
                ),
            );
            return;
        }
        for (i, def) in defs.iter().enumerate() {
            let path = format!("assets[{i}]");
            if def.id.trim().is_empty() {
                self.err(format!("{path}.id"), "must not be empty");
                continue;
            }
            if self.asset_index.contains_key(&def.id) {
                self.err(
                    format!("{path}.id"),
                    format!("duplicate asset id `{}`", def.id),
                );
                continue;
            }
            if def.family.is_some() && def.kind != AssetType::Font {
                self.err(
                    format!("{path}.family"),
                    "`family` is only valid on font assets",
                );
            }
            let kind = match def.kind {
                AssetType::Image => AssetKind::Image,
                AssetType::Svg => AssetKind::Svg,
                AssetType::Font => AssetKind::Font,
                AssetType::Frames => AssetKind::Frames,
            };
            let (file, bytes) =
                match read_asset(&self.opts.asset_root, &def.src, limits.max_asset_file_bytes) {
                    Ok(read) => read,
                    Err(mut e) => {
                        e.path = Some(format!("{path}.src"));
                        self.errors.push(e);
                        // Keep the id known so layers referencing it don't add noise.
                        self.asset_index.insert(def.id.clone(), (u32::MAX, kind));
                        continue;
                    }
                };
            let family = (kind == AssetKind::Font)
                .then(|| def.family.clone().unwrap_or_else(|| def.id.clone()));
            if let Some(f) = &family {
                self.font_families.insert(f.to_lowercase());
            }
            let pack = if kind == AssetKind::Frames {
                match self.frame_pack(&def.src, &file, &bytes) {
                    Ok(pack) => Some(Arc::new(pack)),
                    Err(m) => {
                        self.errors
                            .push(Error::at(ErrorCode::Asset, format!("{path}.src"), m));
                        self.asset_index.insert(def.id.clone(), (u32::MAX, kind));
                        continue;
                    }
                }
            } else {
                None
            };
            let id = self.assets.len() as AssetId;
            self.assets.push(Asset {
                id: def.id.clone(),
                kind,
                src: def.src.clone(),
                content_hash: content_hash(&bytes),
                bytes: Arc::from(bytes),
                family,
                pack,
            });
            self.asset_index.insert(def.id.clone(), (id, kind));
        }
    }

    /// Parse and validate a frame pack manifest. Shards must be plain file
    /// names next to the manifest; every index entry must lie inside its shard.
    fn frame_pack(&self, src: &str, file: &std::path::Path, bytes: &[u8]) -> Result<FramePack, String> {
        let limits = &self.opts.limits;
        let m: PackManifest =
            serde_json::from_slice(bytes).map_err(|e| format!("not a frame pack manifest: {e}"))?;
        if m.nova_frames != 1 {
            return Err(format!(
                "frame pack version {} is not supported (this build reads version 1)",
                m.nova_frames
            ));
        }
        if m.width == 0 || m.height == 0 || m.width > limits.max_width || m.height > limits.max_height {
            return Err(format!("frame pack size {}x{} is not allowed", m.width, m.height));
        }
        if m.index.is_empty() {
            return Err("frame pack has no frames".into());
        }
        if m.index.len() as u64 > limits.max_duration_frames as u64 {
            return Err(format!(
                "frame pack has {} frames; the limit is {}",
                m.index.len(),
                limits.max_duration_frames
            ));
        }
        if m.shards.is_empty() || m.shards.len() > limits.max_pack_shards {
            return Err(format!(
                "frame pack has {} shard files; between 1 and {} are allowed",
                m.shards.len(),
                limits.max_pack_shards
            ));
        }
        let dir = file.parent().ok_or("frame pack manifest has no folder")?.to_path_buf();
        let rel_dir = std::path::Path::new(src).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let mut shards = Vec::with_capacity(m.shards.len());
        let mut sizes = Vec::with_capacity(m.shards.len());
        for name in &m.shards {
            if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
                return Err(format!("shard `{name}` must be a plain file name next to the manifest"));
            }
            let rel = rel_dir.join(name);
            let real = resolve_asset_path(&self.opts.asset_root, &rel.to_string_lossy())?;
            sizes.push(std::fs::metadata(&real).map_err(|e| format!("`{name}`: {e}"))?.len());
            shards.push(real);
        }
        let mut index = Vec::with_capacity(m.index.len());
        for (f, e) in m.index.iter().enumerate() {
            let [shard, offset, len, raw_len, content] = *e;
            let ok = (shard as usize) < shards.len()
                && len > 0
                && len <= u32::MAX as u64
                && offset.checked_add(len).is_some_and(|end| end <= sizes[shard as usize])
                && raw_len <= limits.max_frame_bytes as u64
                && (content as usize) <= f;
            if !ok {
                return Err(format!("frame {f} of the pack has an invalid index entry"));
            }
            index.push(FrameEntry {
                shard: shard as u32,
                offset,
                len: len as u32,
                raw_len: raw_len as u32,
                content: content as u32,
            });
        }
        let mut shaders: Vec<(String, String)> = m.shaders.into_iter().collect();
        shaders.sort();
        if shaders.iter().any(|(_, src)| src.len() > 64 * 1024) {
            return Err("a shader in the frame pack is larger than 64 KiB".into());
        }
        Ok(FramePack {
            width: m.width,
            height: m.height,
            shards,
            index,
            shaders,
            dir,
        })
    }

    fn compile_audio(&mut self, defs: &[AudioDef]) -> Vec<AudioTrack> {
        let mut out = Vec::new();
        for (i, def) in defs.iter().enumerate() {
            let path = format!("audio[{i}]");
            let volume = def.volume.unwrap_or(1.0);
            if !(0.0..=16.0).contains(&volume) {
                self.err(format!("{path}.volume"), "must be between 0 and 16");
            }
            match resolve_asset_path(&self.opts.asset_root, &def.src) {
                Ok(p) => out.push(AudioTrack {
                    id: def.id.clone().unwrap_or_else(|| format!("audio{i}")),
                    path: p,
                    start_frame: def.start_frame.unwrap_or(0) as i64,
                    volume,
                }),
                Err(m) => self
                    .errors
                    .push(Error::at(ErrorCode::Asset, format!("{path}.src"), m)),
            }
        }
        out
    }

    fn compile_layers(
        &mut self,
        layers: &[Layer],
        path: &str,
        scope: Scope,
        depth: usize,
    ) -> Vec<NodeId> {
        if depth > self.opts.limits.max_nesting_depth {
            self.limit(
                path,
                format!(
                    "layers are nested deeper than {}",
                    self.opts.limits.max_nesting_depth
                ),
            );
            return Vec::new();
        }
        let mut out = Vec::with_capacity(layers.len());
        for (i, layer) in layers.iter().enumerate() {
            if let Some(id) = self.compile_layer(layer, &format!("{path}[{i}]"), scope, depth) {
                out.push(id);
            }
        }
        out
    }

    fn compile_layer(
        &mut self,
        l: &Layer,
        path: &str,
        scope: Scope,
        depth: usize,
    ) -> Option<NodeId> {
        self.layer_count += 1;
        if self.layer_count == self.opts.limits.max_layers + 1 {
            self.limit(
                path,
                format!("more than {} layers", self.opts.limits.max_layers),
            );
        }
        if self.layer_count > self.opts.limits.max_layers {
            return None;
        }
        let errors_before = self.errors.len();

        if l.id.trim().is_empty() {
            self.err(format!("{path}.id"), "must not be empty");
        } else if !self.ids.insert(l.id.clone()) {
            self.err(
                format!("{path}.id"),
                format!("duplicate layer id `{}`", l.id),
            );
        }
        self.check_fields(l, path);

        // --- time -----------------------------------------------------------
        let start_local = l.start_frame.unwrap_or(0);
        if let Some(end_local) = l.end_frame
            && end_local <= start_local
        {
            self.err(
                format!("{path}.end_frame"),
                format!("end_frame ({end_local}) must be greater than start_frame ({start_local})"),
            );
        }
        let origin = scope.origin + start_local;
        let end_global = l.end_frame.map_or(scope.end, |e| scope.origin + e);
        let own = Scope {
            origin,
            start: scope.start.max(origin),
            end: scope.end.min(end_global),
        };

        // --- common properties ------------------------------------------------
        let tf = l.transform.as_ref();
        let position = tf
            .and_then(|t| t.position)
            .unwrap_or(Vec2 { x: 0.0, y: 0.0 });
        let (sx, sy) = match tf.and_then(|t| t.scale) {
            None => (1.0, 1.0),
            Some(ScaleDef::Uniform(s)) => (s, s),
            Some(ScaleDef::Xy(v)) => (v.x, v.y),
        };
        let opacity = match (l.opacity, tf.and_then(|t| t.opacity)) {
            (Some(_), Some(_)) => {
                self.err(
                    format!("{path}.opacity"),
                    "set opacity either on the layer or in `transform`, not both",
                );
                1.0
            }
            (a, b) => a.or(b).unwrap_or(1.0),
        };
        if !(0.0..=1.0).contains(&opacity) {
            self.err(
                format!("{path}.opacity"),
                format!("must be between 0 and 1 (got {opacity})"),
            );
        }
        let anchor = match (l.anchor, tf.and_then(|t| t.anchor)) {
            (Some(_), Some(_)) => {
                self.err(
                    format!("{path}.anchor"),
                    "set anchor either on the layer or in `transform`, not both",
                );
                Vec2 { x: 0.0, y: 0.0 }
            }
            (a, b) => a.or(b).unwrap_or(Vec2 { x: 0.0, y: 0.0 }),
        };
        let blur = l.blur.unwrap_or(0.0);
        if blur < 0.0 {
            self.err(format!("{path}.blur"), "must not be negative");
        } else if blur > self.opts.limits.max_blur_sigma {
            self.limit(
                format!("{path}.blur"),
                format!("exceeds the limit of {}", self.opts.limits.max_blur_sigma),
            );
        }
        let shadow = l.shadow.as_ref().map(|s| {
            let b = s.blur.unwrap_or(0.0);
            if b < 0.0 {
                self.err(format!("{path}.shadow.blur"), "must not be negative");
            }
            Shadow {
                x: Prop::Const(s.x.unwrap_or(0.0)),
                y: Prop::Const(s.y.unwrap_or(0.0)),
                blur: Prop::Const(b),
                color: Prop::Const(self.color(&s.color, &format!("{path}.shadow.color"))),
            }
        });

        // --- type-specific content ----------------------------------------------
        let kind = self.compile_kind(l, path, own, depth)?;

        let mut node = Node {
            id: l.id.clone(),
            start: own.start,
            end: own.end,
            origin,
            pos_x: Prop::Const(position.x),
            pos_y: Prop::Const(position.y),
            scale_x: Prop::Const(sx),
            scale_y: Prop::Const(sy),
            rotation: Prop::Const(tf.and_then(|t| t.rotation).unwrap_or(0.0)),
            anchor_x: Prop::Const(anchor.x),
            anchor_y: Prop::Const(anchor.y),
            opacity: Prop::Const(opacity),
            blur: Prop::Const(blur),
            shadow,
            blend: l.blend_mode.map_or(BlendMode::Normal, blend_mode),
            kind,
        };

        let mut seen = HashSet::new();
        for (i, anim) in l.animations.iter().enumerate() {
            let apath = format!("{path}.animations[{i}]");
            let key = anim
                .property
                .strip_prefix("transform.")
                .unwrap_or(&anim.property)
                .to_owned();
            if !seen.insert(key) {
                self.err(
                    format!("{apath}.property"),
                    format!("`{}` is animated more than once", anim.property),
                );
                continue;
            }
            self.apply_animation(&mut node, anim, &apath);
        }

        if l.visible == Some(false) {
            return None;
        }
        if own.end <= own.start {
            // Only worth a warning if the layer is otherwise fine.
            if self.errors.len() == errors_before {
                self.warn(
                    path,
                    format!(
                        "layer `{}` is never visible: its frame range is empty",
                        l.id
                    ),
                );
            }
            return None;
        }
        if self.errors.len() > errors_before {
            return None;
        }
        let id = self.nodes.len() as NodeId;
        self.nodes.push(node);
        Some(id)
    }

    /// Reject fields that do not belong to this layer type.
    fn check_fields(&mut self, l: &Layer, path: &str) {
        use LayerType::*;
        let table: [(&str, bool, &[LayerType]); 12] = [
            (
                "size",
                l.size.is_some(),
                &[Rect, Ellipse, Path, Image, Svg, Frames, Group],
            ),
            ("corner_radius", l.corner_radius.is_some(), &[Rect, Group]),
            ("fill", l.fill.is_some(), &[Rect, Ellipse, Path]),
            ("stroke", l.stroke.is_some(), &[Rect, Ellipse, Path]),
            ("color", l.color.is_some(), &[Solid]),
            ("d", l.d.is_some(), &[Path]),
            ("fill_rule", l.fill_rule.is_some(), &[Path]),
            ("text", l.text.is_some(), &[Text]),
            ("asset", l.asset.is_some(), &[Image, Svg, Frames]),
            ("fit", l.fit.is_some(), &[Image, Svg]),
            ("layers", l.layers.is_some(), &[Group]),
            ("clip", l.clip.is_some(), &[Group]),
        ];
        for (name, present, allowed) in table {
            if present && !allowed.contains(&l.kind) {
                let names: Vec<&str> = allowed.iter().map(|t| t.as_str()).collect();
                self.err(
                    format!("{path}.{name}"),
                    format!(
                        "`{name}` is not valid on a `{}` layer (only on: {})",
                        l.kind.as_str(),
                        names.join(", ")
                    ),
                );
            }
        }
    }

    fn size(&mut self, l: &Layer, path: &str, required: bool) -> Option<[f32; 2]> {
        match l.size {
            Some(s) if s.width > 0.0 && s.height > 0.0 => Some([s.width, s.height]),
            Some(_) => {
                self.err(
                    format!("{path}.size"),
                    "width and height must be greater than 0",
                );
                None
            }
            None => {
                if required {
                    self.err(path, format!("a `{}` layer needs `size`", l.kind.as_str()));
                }
                None
            }
        }
    }

    fn compile_kind(
        &mut self,
        l: &Layer,
        path: &str,
        own: Scope,
        depth: usize,
    ) -> Option<NodeKind> {
        Some(match l.kind {
            LayerType::Solid => {
                let color = match &l.color {
                    Some(c) => self.color(c, &format!("{path}.color")),
                    None => {
                        self.err(path, "a `solid` layer needs `color`");
                        Color::BLACK
                    }
                };
                NodeKind::Solid {
                    color: Prop::Const(color),
                }
            }
            LayerType::Rect | LayerType::Ellipse => {
                let [w, h] = self.size(l, path, true).unwrap_or([1.0, 1.0]);
                let (fill, stroke) = self.paint(l, path);
                if l.kind == LayerType::Rect {
                    let radius = l.corner_radius.unwrap_or(0.0);
                    if radius < 0.0 {
                        self.err(format!("{path}.corner_radius"), "must not be negative");
                    }
                    NodeKind::Rect {
                        width: Prop::Const(w),
                        height: Prop::Const(h),
                        radius: Prop::Const(radius),
                        fill,
                        stroke,
                    }
                } else {
                    NodeKind::Ellipse {
                        width: Prop::Const(w),
                        height: Prop::Const(h),
                        fill,
                        stroke,
                    }
                }
            }
            LayerType::Path => {
                let data = l.d.clone().unwrap_or_default();
                if l.d.is_none() {
                    self.err(path, "a `path` layer needs `d` (SVG path data)");
                } else if data.trim().is_empty() {
                    self.err(format!("{path}.d"), "must not be empty");
                } else if data.len() > self.opts.limits.max_path_data_bytes {
                    self.limit(
                        format!("{path}.d"),
                        format!(
                            "path data is {} bytes; the limit is {}",
                            data.len(),
                            self.opts.limits.max_path_data_bytes
                        ),
                    );
                }
                let size = self.size(l, path, false);
                let (fill, stroke) = self.paint(l, path);
                NodeKind::Path {
                    data,
                    size,
                    fill,
                    stroke,
                    even_odd: matches!(l.fill_rule, Some(FillRuleDef::Evenodd)),
                }
            }
            LayerType::Text => match &l.text {
                Some(t) => NodeKind::Text(self.text(t, &format!("{path}.text"))),
                None => {
                    self.err(path, "a `text` layer needs `text`");
                    return None;
                }
            },
            LayerType::Image | LayerType::Svg => {
                let want = if l.kind == LayerType::Image {
                    AssetKind::Image
                } else {
                    AssetKind::Svg
                };
                let size = self.size(l, path, false);
                let fit = match l.fit {
                    None | Some(FitDef::Contain) => Fit::Contain,
                    Some(FitDef::Fill) => Fit::Fill,
                    Some(FitDef::Cover) => Fit::Cover,
                    Some(FitDef::None) => Fit::None,
                };
                let Some(asset_id) = &l.asset else {
                    self.err(path, format!("a `{}` layer needs `asset`", l.kind.as_str()));
                    return None;
                };
                let Some(&(asset, kind)) = self.asset_index.get(asset_id) else {
                    self.err(
                        format!("{path}.asset"),
                        format!("no asset with id `{asset_id}`"),
                    );
                    return None;
                };
                self.used_assets.insert(asset_id.clone());
                if kind != want {
                    let actual = match kind {
                        AssetKind::Image => "image",
                        AssetKind::Svg => "svg",
                        AssetKind::Font => "font",
                        AssetKind::Frames => "frame pack",
                    };
                    self.err(
                        format!("{path}.asset"),
                        format!(
                            "asset `{asset_id}` is a {actual}; a `{}` layer needs a {} asset",
                            l.kind.as_str(),
                            l.kind.as_str()
                        ),
                    );
                    return None;
                }
                if asset == u32::MAX {
                    return None; // the asset itself failed to load; already reported
                }
                NodeKind::Image { asset, size, fit }
            }
            LayerType::Frames => {
                let size = self.size(l, path, false);
                let Some(asset_id) = &l.asset else {
                    self.err(path, "a `frames` layer needs `asset`");
                    return None;
                };
                let Some(&(asset, kind)) = self.asset_index.get(asset_id) else {
                    self.err(
                        format!("{path}.asset"),
                        format!("no asset with id `{asset_id}`"),
                    );
                    return None;
                };
                self.used_assets.insert(asset_id.clone());
                if kind != AssetKind::Frames {
                    self.err(
                        format!("{path}.asset"),
                        format!("asset `{asset_id}` is not a frame pack; a `frames` layer needs a `frames` asset"),
                    );
                    return None;
                }
                if asset == u32::MAX {
                    return None; // the pack itself failed to load; already reported
                }
                let pack = self.assets[asset as usize].pack.as_ref()?;
                NodeKind::Frames {
                    asset,
                    size: size.unwrap_or([pack.width as f32, pack.height as f32]),
                    frames: pack.index.len() as u32,
                }
            }
            LayerType::Group => {
                let size = self.size(l, path, false);
                let clip = l.clip.unwrap_or(false);
                if clip && size.is_none() {
                    self.err(
                        format!("{path}.clip"),
                        "`clip` needs the group to have a `size`",
                    );
                }
                let radius = l.corner_radius.unwrap_or(0.0);
                if radius < 0.0 {
                    self.err(format!("{path}.corner_radius"), "must not be negative");
                } else if radius > 0.0 && !clip {
                    self.err(
                        format!("{path}.corner_radius"),
                        "on a group, `corner_radius` rounds the clip and needs `clip: true`",
                    );
                }
                let children = match &l.layers {
                    Some(children) => {
                        self.compile_layers(children, &format!("{path}.layers"), own, depth + 1)
                    }
                    None => {
                        self.err(path, "a `group` layer needs `layers`");
                        Vec::new()
                    }
                };
                NodeKind::Group {
                    children,
                    size,
                    clip,
                    radius,
                }
            }
        })
    }

    fn paint(&mut self, l: &Layer, path: &str) -> (Option<Fill>, Option<Stroke>) {
        let fill = l
            .fill
            .as_ref()
            .map(|f| self.fill(f, &format!("{path}.fill")));
        let stroke = l.stroke.as_ref().map(|s| {
            if s.width < 0.0 {
                self.err(format!("{path}.stroke.width"), "must not be negative");
            }
            let (ts, te) = (s.trim_start.unwrap_or(0.0), s.trim_end.unwrap_or(1.0));
            if !(0.0..=1.0).contains(&ts) || !(0.0..=1.0).contains(&te) {
                self.err(
                    format!("{path}.stroke"),
                    "trim_start and trim_end must be between 0 and 1",
                );
            }
            Stroke {
                color: Prop::Const(self.color(&s.color, &format!("{path}.stroke.color"))),
                width: Prop::Const(s.width),
                cap: match s.cap {
                    None | Some(LineCapDef::Butt) => LineCap::Butt,
                    Some(LineCapDef::Round) => LineCap::Round,
                    Some(LineCapDef::Square) => LineCap::Square,
                },
                join: match s.join {
                    None | Some(LineJoinDef::Miter) => LineJoin::Miter,
                    Some(LineJoinDef::Round) => LineJoin::Round,
                    Some(LineJoinDef::Bevel) => LineJoin::Bevel,
                },
                trim_start: Prop::Const(ts),
                trim_end: Prop::Const(te),
            }
        });
        if fill.is_none() && stroke.is_none() {
            self.err(
                path,
                format!("a `{}` layer needs `fill` and/or `stroke`", l.kind.as_str()),
            );
        }
        (fill, stroke)
    }

    fn fill(&mut self, def: &FillDef, path: &str) -> Fill {
        match def {
            FillDef::Color(s) => Fill::Solid(Prop::Const(self.color(s, path))),
            FillDef::Gradient(GradientDef::Linear { from, to, stops }) => {
                let (from, to) = (from.unwrap_or([0.0, 0.0]), to.unwrap_or([1.0, 0.0]));
                if from == to {
                    self.err(path, "linear gradient `from` and `to` must differ");
                }
                Fill::Linear {
                    from,
                    to,
                    stops: self.stops(stops, path),
                }
            }
            FillDef::Gradient(GradientDef::Radial {
                center,
                radius,
                stops,
            }) => {
                let radius = radius.unwrap_or(0.5);
                if radius <= 0.0 {
                    self.err(format!("{path}.radius"), "must be greater than 0");
                }
                Fill::Radial {
                    center: center.unwrap_or([0.5, 0.5]),
                    radius,
                    stops: self.stops(stops, path),
                }
            }
        }
    }

    fn stops(&mut self, defs: &[StopDef], path: &str) -> Vec<GradientStop> {
        if defs.len() < 2 {
            self.err(format!("{path}.stops"), "a gradient needs at least 2 stops");
        }
        let mut prev = 0.0_f32;
        let mut out = Vec::with_capacity(defs.len());
        for (i, s) in defs.iter().enumerate() {
            let spath = format!("{path}.stops[{i}]");
            if !(0.0..=1.0).contains(&s.offset) {
                self.err(format!("{spath}.offset"), "must be between 0 and 1");
            } else if s.offset < prev {
                self.err(format!("{spath}.offset"), "stop offsets must not decrease");
            }
            prev = s.offset;
            out.push(GradientStop {
                offset: s.offset,
                color: self.color(&s.color, &format!("{spath}.color")),
            });
        }
        out
    }

    fn text(&mut self, t: &TextDef, path: &str) -> TextNode {
        if t.value.len() > self.opts.limits.max_text_bytes {
            self.limit(
                format!("{path}.value"),
                format!(
                    "text is {} bytes; the limit is {}",
                    t.value.len(),
                    self.opts.limits.max_text_bytes
                ),
            );
        }
        if t.font_size <= 0.0 || t.font_size > 4096.0 {
            self.err(
                format!("{path}.font_size"),
                "must be greater than 0 and at most 4096",
            );
        }
        let weight = t.font_weight.unwrap_or(400);
        if !(1..=1000).contains(&weight) {
            self.err(format!("{path}.font_weight"), "must be between 1 and 1000");
        }
        let families: Vec<String> = match &t.font_family {
            FontFamilyDef::One(f) => vec![f.clone()],
            FontFamilyDef::Many(list) => list.clone(),
        };
        if families.is_empty() || families.iter().any(|f| f.trim().is_empty()) {
            self.err(
                format!("{path}.font_family"),
                "must name at least one font family",
            );
        } else if !self.font_families.contains(&families[0].to_lowercase()) {
            self.warn(
                format!("{path}.font_family"),
                format!(
                    "`{}` is not a font asset of this template; a system font will be used and the result \
                     can differ between machines. Add the font file to `assets` for reproducible output.",
                    families[0]
                ),
            );
        }
        if let Some(lh) = t.line_height
            && lh <= 0.0
        {
            self.err(format!("{path}.line_height"), "must be greater than 0");
        }
        if let Some(mw) = t.max_width
            && mw <= 0.0
        {
            self.err(format!("{path}.max_width"), "must be greater than 0");
        }
        if t.max_lines == Some(0) {
            self.err(format!("{path}.max_lines"), "must be at least 1");
        }
        let color = t
            .color
            .as_ref()
            .map_or(Color::WHITE, |c| self.color(c, &format!("{path}.color")));
        // Character indices -> byte offsets.
        let offsets: Vec<usize> = t
            .value
            .char_indices()
            .map(|(i, _)| i)
            .chain([t.value.len()])
            .collect();
        let chars = offsets.len() - 1;
        let mut spans: Vec<nova_core::TextSpan> = Vec::with_capacity(t.spans.len());
        for (i, s) in t.spans.iter().enumerate() {
            let spath = format!("{path}.spans[{i}]");
            let color = self.color(&s.color, &format!("{spath}.color"));
            if s.start >= s.end || s.end as usize > chars {
                self.err(&spath, format!("start must be less than end, and end at most {chars} (the text's length in characters)"));
                continue;
            }
            let (start, end) = (offsets[s.start as usize], offsets[s.end as usize]);
            if spans.last().is_some_and(|prev| start < prev.end) {
                self.err(&spath, "spans must be in order and must not overlap");
                continue;
            }
            spans.push(nova_core::TextSpan { start, end, color });
        }
        let reveal = t.reveal.as_ref().map(|r| {
            if r.count < 0.0 {
                self.err(format!("{path}.reveal.count"), "must not be negative");
            }
            nova_core::Reveal {
                unit: match r.unit {
                    None | Some(RevealUnitDef::Word) => nova_core::RevealUnit::Word,
                    Some(RevealUnitDef::Char) => nova_core::RevealUnit::Char,
                },
                count: Prop::Const(r.count),
                hidden: r.hidden_color.as_ref().map_or(Color::TRANSPARENT, |c| {
                    self.color(c, &format!("{path}.reveal.hidden_color"))
                }),
            }
        });
        let background = t.background.as_ref().map(|b| {
            let bpath = format!("{path}.background");
            let padding = match b.padding {
                None => [0.0; 4],
                Some(PaddingDef::All(p)) => [p; 4],
                Some(PaddingDef::Axes(a)) => [a.x, a.y, a.x, a.y],
                Some(PaddingDef::Sides(s)) => [s.left, s.top, s.right, s.bottom],
            };
            if padding.iter().any(|p| *p < 0.0) {
                self.err(format!("{bpath}.padding"), "must not be negative");
            }
            let radius = b.radius.unwrap_or(0.0);
            if radius < 0.0 {
                self.err(format!("{bpath}.radius"), "must not be negative");
            }
            let border = match (&b.border_color, b.border_width) {
                (Some(c), w) => {
                    let w = w.unwrap_or(1.0);
                    if w <= 0.0 {
                        self.err(format!("{bpath}.border_width"), "must be greater than 0");
                    }
                    Some((self.color(c, &format!("{bpath}.border_color")), w))
                }
                (None, Some(_)) => {
                    self.err(format!("{bpath}.border_width"), "needs `border_color`");
                    None
                }
                (None, None) => None,
            };
            if b.color.is_none() && border.is_none() {
                self.err(&bpath, "needs `color` and/or `border_color`");
            }
            nova_core::TextBox {
                color: b
                    .color
                    .as_ref()
                    .map(|c| self.color(c, &format!("{bpath}.color"))),
                radius,
                padding,
                border,
            }
        });
        TextNode {
            value: t.value.clone(),
            families,
            size: Prop::Const(t.font_size),
            weight,
            italic: t.italic.unwrap_or(false),
            color: Prop::Const(color),
            line_height: t.line_height,
            letter_spacing: Prop::Const(t.letter_spacing.unwrap_or(0.0)),
            align: match t.align {
                None | Some(TextAlignDef::Left) => TextAlign::Left,
                Some(TextAlignDef::Center) => TextAlign::Center,
                Some(TextAlignDef::Right) => TextAlign::Right,
                Some(TextAlignDef::Justify) => TextAlign::Justify,
            },
            max_width: t.max_width,
            max_lines: t.max_lines,
            hug: t.hug.unwrap_or(false),
            spans,
            reveal,
            background,
        }
    }

    // --- animations ---------------------------------------------------------------

    fn apply_animation(&mut self, node: &mut Node, anim: &AnimationDef, path: &str) {
        if anim.keyframes.is_empty() {
            self.err(format!("{path}.keyframes"), "needs at least one keyframe");
            return;
        }
        if anim.keyframes.len() > self.opts.limits.max_keyframes_per_track {
            self.limit(
                format!("{path}.keyframes"),
                format!(
                    "{} keyframes exceeds the limit of {}",
                    anim.keyframes.len(),
                    self.opts.limits.max_keyframes_per_track
                ),
            );
            return;
        }
        for (i, pair) in anim.keyframes.windows(2).enumerate() {
            if pair[1].frame <= pair[0].frame {
                self.err(
                    format!("{path}.keyframes[{}].frame", i + 1),
                    format!(
                        "keyframe frames must strictly increase ({} follows {})",
                        pair[1].frame, pair[0].frame
                    ),
                );
                return;
            }
        }
        let type_name = node.type_name();
        let slot = match find_slot(node, &anim.property) {
            Ok(slot) => slot,
            Err(reason) => {
                self.err(
                    format!("{path}.property"),
                    format!(
                        "cannot animate `{}` on this `{type_name}` layer: {reason}. Animatable here: {}",
                        anim.property,
                        animatable(type_name).join(", ")
                    ),
                );
                return;
            }
        };
        match slot {
            Slot::Num(p) => {
                if let Some(track) = self.track(&anim.keyframes, path, number) {
                    *p = Prop::Anim(track);
                }
            }
            Slot::Num2(a, b) => {
                if let Some(track) = self.track(&anim.keyframes, path, number) {
                    *a = Prop::Anim(track.clone());
                    *b = Prop::Anim(track);
                }
            }
            Slot::Col(p) => {
                if let Some(track) = self.track(&anim.keyframes, path, colour) {
                    *p = Prop::Anim(track);
                }
            }
        }
    }

    fn track<T: Lerp>(
        &mut self,
        keyframes: &[KeyframeDef],
        path: &str,
        convert: fn(&KeyValue) -> Result<T, String>,
    ) -> Option<Track<T>> {
        let mut keys = Vec::with_capacity(keyframes.len());
        let mut ok = true;
        for (i, kf) in keyframes.iter().enumerate() {
            let kpath = format!("{path}.keyframes[{i}]");
            let easing = match &kf.easing {
                None => Easing::Linear,
                Some(def) => match easing(def) {
                    Ok(e) => e,
                    Err(m) => {
                        self.err(format!("{kpath}.easing"), m);
                        ok = false;
                        Easing::Linear
                    }
                },
            };
            match convert(&kf.value) {
                Ok(value) => keys.push(Key {
                    frame: kf.frame,
                    value,
                    easing,
                }),
                Err(m) => {
                    self.err(format!("{kpath}.value"), m);
                    ok = false;
                }
            }
        }
        if !ok {
            return None;
        }
        Track::new(keys)
    }
}

fn number(v: &KeyValue) -> Result<f32, String> {
    match v {
        KeyValue::Number(n) if n.is_finite() => Ok(*n as f32),
        KeyValue::Number(_) => Err("must be a finite number".into()),
        KeyValue::Text(s) => Err(format!("this property is numeric; got the string `{s}`")),
    }
}

fn colour(v: &KeyValue) -> Result<Color, String> {
    match v {
        KeyValue::Text(s) => Color::parse(s),
        KeyValue::Number(n) => Err(format!(
            "this property is a colour; got the number {n} (use \"#RRGGBB\")"
        )),
    }
}

fn easing(def: &EasingDef) -> Result<Easing, String> {
    match def {
        EasingDef::Name(name) => Easing::from_name(name).ok_or_else(|| {
            format!(
                "unknown easing `{name}`. Use one of: {}, or {{\"cubic_bezier\": [x1,y1,x2,y2]}}, \
                 or {{\"spring\": {{\"stiffness\": 100, \"damping\": 10, \"mass\": 1}}}}",
                Easing::NAMES.join(", ")
            )
        }),
        EasingDef::Bezier {
            cubic_bezier: [x1, y1, x2, y2],
        } => {
            if !(0.0..=1.0).contains(x1) || !(0.0..=1.0).contains(x2) {
                return Err("cubic_bezier x1 and x2 must be between 0 and 1".into());
            }
            if !y1.is_finite() || !y2.is_finite() {
                return Err("cubic_bezier values must be finite".into());
            }
            Ok(Easing::CubicBezier {
                x1: *x1,
                y1: *y1,
                x2: *x2,
                y2: *y2,
            })
        }
        EasingDef::Spring { spring } => {
            let (stiffness, damping, mass) = (
                spring.stiffness.unwrap_or(100.0),
                spring.damping.unwrap_or(10.0),
                spring.mass.unwrap_or(1.0),
            );
            if stiffness <= 0.0 || mass <= 0.0 || damping < 0.0 {
                return Err("spring stiffness and mass must be greater than 0, damping must not be negative".into());
            }
            Ok(Easing::Spring {
                stiffness,
                damping,
                mass,
            })
        }
    }
}

fn blend_mode(b: BlendModeDef) -> BlendMode {
    match b {
        BlendModeDef::Normal => BlendMode::Normal,
        BlendModeDef::Multiply => BlendMode::Multiply,
        BlendModeDef::Screen => BlendMode::Screen,
        BlendModeDef::Overlay => BlendMode::Overlay,
        BlendModeDef::Darken => BlendMode::Darken,
        BlendModeDef::Lighten => BlendMode::Lighten,
        BlendModeDef::ColorDodge => BlendMode::ColorDodge,
        BlendModeDef::ColorBurn => BlendMode::ColorBurn,
        BlendModeDef::HardLight => BlendMode::HardLight,
        BlendModeDef::SoftLight => BlendMode::SoftLight,
        BlendModeDef::Difference => BlendMode::Difference,
        BlendModeDef::Exclusion => BlendMode::Exclusion,
        BlendModeDef::Add => BlendMode::Add,
    }
}

/// A place an animation track can be attached to.
enum Slot<'a> {
    Num(&'a mut Prop<f32>),
    /// One track driving two properties (uniform scale).
    Num2(&'a mut Prop<f32>, &'a mut Prop<f32>),
    Col(&'a mut Prop<Color>),
}

const COMMON_PROPS: &[&str] = &[
    "transform.position.x",
    "transform.position.y",
    "transform.scale",
    "transform.scale.x",
    "transform.scale.y",
    "transform.rotation",
    "transform.anchor.x",
    "transform.anchor.y",
    "opacity",
    "blur",
    "shadow.x",
    "shadow.y",
    "shadow.blur",
    "shadow.color",
];

fn animatable(type_name: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = COMMON_PROPS.to_vec();
    let shape = [
        "fill.color",
        "stroke.color",
        "stroke.width",
        "stroke.trim_start",
        "stroke.trim_end",
    ];
    match type_name {
        "solid" => v.push("color"),
        "rect" => {
            v.extend(["size.width", "size.height", "corner_radius"]);
            v.extend(shape);
        }
        "ellipse" => {
            v.extend(["size.width", "size.height"]);
            v.extend(shape);
        }
        "path" => v.extend(shape),
        "text" => v.extend([
            "text.color",
            "text.font_size",
            "text.letter_spacing",
            "text.reveal",
        ]),
        _ => {}
    }
    v
}

fn find_slot<'a>(node: &'a mut Node, property: &str) -> Result<Slot<'a>, String> {
    let p = property.strip_prefix("transform.").unwrap_or(property);
    match p {
        "position.x" => return Ok(Slot::Num(&mut node.pos_x)),
        "position.y" => return Ok(Slot::Num(&mut node.pos_y)),
        "scale" => return Ok(Slot::Num2(&mut node.scale_x, &mut node.scale_y)),
        "scale.x" => return Ok(Slot::Num(&mut node.scale_x)),
        "scale.y" => return Ok(Slot::Num(&mut node.scale_y)),
        "rotation" => return Ok(Slot::Num(&mut node.rotation)),
        "anchor.x" => return Ok(Slot::Num(&mut node.anchor_x)),
        "anchor.y" => return Ok(Slot::Num(&mut node.anchor_y)),
        "opacity" => return Ok(Slot::Num(&mut node.opacity)),
        "blur" => return Ok(Slot::Num(&mut node.blur)),
        "shadow.x" | "shadow.y" | "shadow.blur" | "shadow.color" => {
            let shadow = node
                .shadow
                .as_mut()
                .ok_or("the layer has no `shadow` to animate")?;
            return Ok(match p {
                "shadow.x" => Slot::Num(&mut shadow.x),
                "shadow.y" => Slot::Num(&mut shadow.y),
                "shadow.blur" => Slot::Num(&mut shadow.blur),
                _ => Slot::Col(&mut shadow.color),
            });
        }
        _ => {}
    }

    fn fill_slot<'a>(fill: &'a mut Option<Fill>) -> Result<Slot<'a>, String> {
        match fill {
            Some(Fill::Solid(c)) => Ok(Slot::Col(c)),
            Some(_) => Err("gradient fills cannot be animated".into()),
            None => Err("the layer has no `fill`".into()),
        }
    }
    fn stroke_slot<'a>(stroke: &'a mut Option<Stroke>, p: &str) -> Result<Slot<'a>, String> {
        let s = stroke.as_mut().ok_or("the layer has no `stroke`")?;
        Ok(match p {
            "stroke.color" => Slot::Col(&mut s.color),
            "stroke.width" => Slot::Num(&mut s.width),
            "stroke.trim_start" => Slot::Num(&mut s.trim_start),
            _ => Slot::Num(&mut s.trim_end),
        })
    }
    let is_stroke = matches!(
        p,
        "stroke.color" | "stroke.width" | "stroke.trim_start" | "stroke.trim_end"
    );

    match &mut node.kind {
        NodeKind::Solid { color } if p == "color" => Ok(Slot::Col(color)),
        NodeKind::Rect {
            width,
            height,
            radius,
            fill,
            stroke,
        } => match p {
            "size.width" => Ok(Slot::Num(width)),
            "size.height" => Ok(Slot::Num(height)),
            "corner_radius" => Ok(Slot::Num(radius)),
            "fill.color" => fill_slot(fill),
            _ if is_stroke => stroke_slot(stroke, p),
            _ => Err("no such property".into()),
        },
        NodeKind::Ellipse {
            width,
            height,
            fill,
            stroke,
        } => match p {
            "size.width" => Ok(Slot::Num(width)),
            "size.height" => Ok(Slot::Num(height)),
            "fill.color" => fill_slot(fill),
            _ if is_stroke => stroke_slot(stroke, p),
            _ => Err("no such property".into()),
        },
        NodeKind::Path { fill, stroke, .. } => match p {
            "fill.color" => fill_slot(fill),
            _ if is_stroke => stroke_slot(stroke, p),
            _ => Err("no such property".into()),
        },
        NodeKind::Text(t) => match p {
            "text.color" | "color" => Ok(Slot::Col(&mut t.color)),
            "text.font_size" | "font_size" => Ok(Slot::Num(&mut t.size)),
            "text.letter_spacing" | "letter_spacing" => Ok(Slot::Num(&mut t.letter_spacing)),
            "text.reveal" | "reveal" => match &mut t.reveal {
                Some(r) => Ok(Slot::Num(&mut r.count)),
                None => Err("the text has no `reveal` to animate".into()),
            },
            _ => Err("no such property".into()),
        },
        _ => Err("no such property".into()),
    }
}
