//! Baked frame packs: reading a frame's display list from disk.
//!
//! A pack is produced outside NOVA (for example by running a Remotion
//! composition in Node, without a browser). Every frame is a short list of
//! operations: an SVG document to rasterise, or a full-frame SkSL shader
//! pass. NOVA trusts a pack no more than a template: shards and referenced
//! images must lie inside the pack's folder, and every size is checked
//! before memory is allocated.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use nova_core::FramePack;
use resvg::usvg;

/// An SVG document to draw over what is already there.
pub const OP_SVG: u8 = 1;
/// JSON `{ "shader": key, "uniforms": { name: number | [numbers] } }`.
pub const OP_SHADER: u8 = 2;

pub(crate) struct PackRuntime {
    pub pack: Arc<FramePack>,
    files: Vec<Mutex<File>>,
    /// Parser options for the pack's SVG documents (fonts, image loading).
    pub options: usvg::Options<'static>,
}

/// Font selection for baked text, following the browser the frames were
/// written for: a named family that is not loaded may be replaced by the
/// system's metric-compatible alias (Arial -> Liberation Sans), but never by
/// an arbitrary substitute — then the next family in the list is tried.
fn font_selector() -> usvg::FontSelectionFn<'static> {
    use usvg::fontdb;
    // family name as written -> family to query, or None when unavailable
    let resolved: Mutex<HashMap<String, Option<String>>> = Mutex::new(HashMap::new());
    Box::new(move |font, db| {
        enum Pick {
            Generic(fontdb::Family<'static>),
            Named(String),
        }
        let has = |db: &fontdb::Database, name: &str| {
            db.faces()
                .any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
        };
        let mut picks = Vec::new();
        for family in font.families() {
            picks.push(match family {
                usvg::FontFamily::Serif => Pick::Generic(fontdb::Family::Serif),
                usvg::FontFamily::SansSerif => Pick::Generic(fontdb::Family::SansSerif),
                usvg::FontFamily::Cursive => Pick::Generic(fontdb::Family::Cursive),
                usvg::FontFamily::Fantasy => Pick::Generic(fontdb::Family::Fantasy),
                usvg::FontFamily::Monospace => Pick::Generic(fontdb::Family::Monospace),
                usvg::FontFamily::Named(name) => {
                    let mut map = resolved.lock().unwrap_or_else(|e| e.into_inner());
                    let hit = map.entry(name.clone()).or_insert_with(|| {
                        if has(db, name) {
                            return Some(name.clone());
                        }
                        // Skia asks fontconfig and rejects weak substitutes, as Chrome does.
                        skia_safe::FontMgr::new()
                            .match_family_style(name, skia_safe::FontStyle::normal())
                            .map(|t| t.family_name())
                            .filter(|alias| has(db, alias))
                    });
                    match hit {
                        Some(n) => Pick::Named(n.clone()),
                        None => continue,
                    }
                }
            });
        }
        let mut names: Vec<fontdb::Family> = picks
            .iter()
            .map(|p| match p {
                Pick::Generic(g) => *g,
                Pick::Named(n) => fontdb::Family::Name(n),
            })
            .collect();
        names.push(fontdb::Family::Serif);
        let stretch = match font.stretch() {
            usvg::FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
            usvg::FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
            usvg::FontStretch::Condensed => fontdb::Stretch::Condensed,
            usvg::FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
            usvg::FontStretch::Normal => fontdb::Stretch::Normal,
            usvg::FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
            usvg::FontStretch::Expanded => fontdb::Stretch::Expanded,
            usvg::FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
            usvg::FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
        };
        let style = match font.style() {
            usvg::FontStyle::Normal => fontdb::Style::Normal,
            usvg::FontStyle::Italic => fontdb::Style::Italic,
            usvg::FontStyle::Oblique => fontdb::Style::Oblique,
        };
        db.query(&fontdb::Query {
            families: &names,
            weight: fontdb::Weight(font.weight()),
            stretch,
            style,
        })
    })
}

/// Fallback for a character the chosen font lacks, in the browser's order:
/// its standard font first (Times New Roman, i.e. what the system maps that
/// to), then the system's per-character fallback for the base family.
/// Without system fonts this ends at usvg's own search of the loaded fonts.
fn fallback_selector() -> usvg::FallbackSelectionFn<'static> {
    use usvg::fontdb;
    let default = usvg::FontResolver::default_fallback_selector();
    let cache: Mutex<HashMap<(String, char), Option<String>>> = Mutex::new(HashMap::new());
    Box::new(move |c, exclude, db| {
        let (family, weight, style, stretch) = {
            let base = db.face(*exclude.first()?)?;
            (base.families.first()?.0.clone(), base.weight, base.style, base.stretch)
        };
        let wanted = cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry((family.clone(), c))
            .or_insert_with(|| {
                let mgr = skia_safe::FontMgr::new();
                let normal = skia_safe::FontStyle::normal();
                let unichar = c as skia_safe::Unichar;
                mgr.match_family_style("Times New Roman", normal)
                    .filter(|t| t.unichar_to_glyph(unichar) != 0)
                    .or_else(|| mgr.match_family_style_character(&family, normal, &[], unichar))
                    .map(|t| t.family_name())
            })
            .clone();
        let found = wanted.and_then(|name| {
            db.query(&fontdb::Query {
                families: &[fontdb::Family::Name(&name)],
                weight,
                stretch,
                style,
            })
        });
        match found {
            Some(id) if !exclude.contains(&id) => Some(id),
            _ => default(c, exclude, db),
        }
    })
}

impl PackRuntime {
    pub fn open(
        pack: Arc<FramePack>,
        fontdb: Arc<usvg::fontdb::Database>,
    ) -> Result<PackRuntime, String> {
        let mut files = Vec::with_capacity(pack.shards.len());
        for path in &pack.shards {
            let file = File::open(path).map_err(|e| format!("`{}`: {e}", path.display()))?;
            files.push(Mutex::new(file));
        }
        let images = ImageCache {
            dir: pack.dir.clone(),
            loaded: Mutex::new(HashMap::new()),
        };
        let options = usvg::Options {
            resources_dir: None,
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
                resolve_string: Box::new(move |href, _opts| images.get(href)),
            },
            fontdb,
            font_resolver: usvg::FontResolver {
                select_font: font_selector(),
                select_fallback: fallback_selector(),
            },
            ..usvg::Options::default()
        };
        Ok(PackRuntime {
            pack,
            files,
            options,
        })
    }

    /// The decompressed display list of the frame with this content id.
    pub fn read(&self, content: u32) -> Result<Vec<u8>, String> {
        let entry = self
            .pack
            .index
            .get(content as usize)
            .ok_or("frame is not in the pack")?;
        let mut packed = vec![0u8; entry.len as usize];
        {
            let mut file = self.files[entry.shard as usize]
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            file.seek(SeekFrom::Start(entry.offset))
                .and_then(|_| file.read_exact(&mut packed))
                .map_err(|e| format!("cannot read frame {content} from the pack: {e}"))?;
        }
        // `take` stops a corrupt or hostile stream from inflating past the
        // size the manifest declared (and the limits accepted).
        let mut raw = Vec::with_capacity(entry.raw_len as usize);
        flate2::read::ZlibDecoder::new(packed.as_slice())
            .take(entry.raw_len as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|e| format!("frame {content} of the pack is corrupt: {e}"))?;
        if raw.len() != entry.raw_len as usize {
            return Err(format!(
                "frame {content} of the pack is {} bytes, the manifest says {}",
                raw.len(),
                entry.raw_len
            ));
        }
        Ok(raw)
    }

    pub fn shader_source(&self, key: &str) -> Option<&str> {
        self.pack
            .shaders
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, src)| src.as_str())
    }
}

/// Full-frame pictures that repeat from frame to frame, shared by all render
/// threads: the output of a shader pass (film grain changes every third
/// frame), a screen-sized gradient (a vignette is the same for a whole
/// scene). Keyed by everything that determines the pixels, so a hit is
/// always pixel-identical to rendering again.
pub(crate) struct OverlayCache {
    inner: Mutex<Overlays>,
}

struct Overlays {
    entries: Vec<(Vec<u8>, skia_safe::Image)>,
    /// Keys seen once but not stored: only something that comes back is
    /// worth the offscreen pass that storing it costs.
    seen: Vec<u64>,
}

const MAX_OVERLAYS: usize = 12;
const MAX_SEEN: usize = 256;

fn key_hash(key: &[u8]) -> u64 {
    key.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl OverlayCache {
    pub fn new() -> OverlayCache {
        OverlayCache {
            inner: Mutex::new(Overlays {
                entries: Vec::new(),
                seen: Vec::new(),
            }),
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<skia_safe::Image> {
        let mut c = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let at = c.entries.iter().position(|(k, _)| k == key)?;
        let hit = c.entries.remove(at);
        let image = hit.1.clone();
        c.entries.push(hit); // most recently used last
        Some(image)
    }

    /// True the second time a key is offered (and from then on).
    pub fn seen_before(&self, key: &[u8]) -> bool {
        let h = key_hash(key);
        let mut c = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if c.seen.contains(&h) {
            return true;
        }
        if c.seen.len() >= MAX_SEEN {
            c.seen.remove(0);
        }
        c.seen.push(h);
        false
    }

    pub fn put(&self, key: Vec<u8>, image: skia_safe::Image) {
        let mut c = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if c.entries.iter().any(|(k, _)| *k == key) {
            return;
        }
        if c.entries.len() >= MAX_OVERLAYS {
            c.entries.remove(0);
        }
        c.entries.push((key, image));
    }
}

/// Split a display list into `(kind, payload)` operations.
pub(crate) fn ops(blob: &[u8]) -> Result<Vec<(u8, &[u8])>, String> {
    let mut out = Vec::new();
    let mut rest = blob;
    while !rest.is_empty() {
        if rest.len() < 5 {
            return Err("truncated operation header".into());
        }
        let kind = rest[0];
        let len = u32::from_le_bytes([rest[1], rest[2], rest[3], rest[4]]) as usize;
        rest = &rest[5..];
        if len > rest.len() {
            return Err("operation runs past the end of the frame".into());
        }
        out.push((kind, &rest[..len]));
        rest = &rest[len..];
    }
    Ok(out)
}

/// Images referenced from baked SVG by relative path. Only regular files
/// inside the pack's folder are ever read; each is read once.
struct ImageCache {
    dir: PathBuf,
    loaded: Mutex<HashMap<String, Option<usvg::ImageKind>>>,
}

impl ImageCache {
    fn get(&self, href: &str) -> Option<usvg::ImageKind> {
        let mut loaded = self.loaded.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(hit) = loaded.get(href) {
            return hit.clone();
        }
        let image = self.load(href);
        loaded.insert(href.to_owned(), image.clone());
        image
    }

    fn load(&self, href: &str) -> Option<usvg::ImageKind> {
        let rel = Path::new(href);
        if href.contains("://") || rel.is_absolute() {
            return None;
        }
        if !rel
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return None;
        }
        let root = self.dir.canonicalize().ok()?;
        let real = root.join(rel).canonicalize().ok()?;
        if !real.starts_with(&root) || !real.is_file() {
            return None;
        }
        if std::fs::metadata(&real).ok()?.len() > 64 * 1024 * 1024 {
            return None;
        }
        let bytes = Arc::new(std::fs::read(&real).ok()?);
        match bytes.as_slice() {
            [0x89, b'P', b'N', b'G', ..] => Some(usvg::ImageKind::PNG(bytes)),
            [0xff, 0xd8, 0xff, ..] => Some(usvg::ImageKind::JPEG(bytes)),
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => {
                Some(usvg::ImageKind::WEBP(bytes))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_are_split_and_bounds_checked() {
        let mut blob = vec![OP_SVG, 3, 0, 0, 0, b'a', b'b', b'c'];
        blob.extend_from_slice(&[OP_SHADER, 2, 0, 0, 0, b'{', b'}']);
        let list = ops(&blob).unwrap();
        assert_eq!(list, vec![(OP_SVG, &b"abc"[..]), (OP_SHADER, &b"{}"[..])]);
        assert!(ops(&blob[..blob.len() - 1]).is_err(), "payload cut short");
        assert!(ops(&[OP_SVG, 1, 0]).is_err(), "header cut short");
        assert!(ops(&[]).unwrap().is_empty());
    }

    #[test]
    fn images_outside_the_pack_folder_are_never_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.png"), [0x89, b'P', b'N', b'G', 0, 0]).unwrap();
        let cache = ImageCache {
            dir: dir.path().to_path_buf(),
            loaded: Mutex::new(HashMap::new()),
        };
        assert!(cache.get("ok.png").is_some());
        for bad in ["/etc/hostname", "../ok.png", "file:///etc/hostname", "nope.png"] {
            assert!(cache.get(bad).is_none(), "{bad} must not load");
        }
    }
}
