//! Asset resolution. Templates are untrusted: an asset path may only ever
//! name a regular file underneath the asset root.

use std::path::{Component, Path, PathBuf};

use nova_core::{Error, ErrorCode};

/// Resolve `src` under `root`, refusing anything that could escape it:
/// absolute paths, `..`, URLs, and symlinks whose target lies outside.
pub fn resolve_asset_path(root: &Path, src: &str) -> Result<PathBuf, String> {
    if src.is_empty() {
        return Err("asset path is empty".into());
    }
    if src.contains('\0') {
        return Err("asset path contains a NUL byte".into());
    }
    if src.contains("://") || src.starts_with("data:") {
        return Err(format!(
            "`{src}`: URLs are not allowed; assets must be local files under the asset root"
        ));
    }
    let rel = Path::new(src);
    if rel.is_absolute() || src.starts_with('\\') || src.starts_with('~') {
        return Err(format!(
            "`{src}`: absolute paths are not allowed; use a path relative to the asset root"
        ));
    }
    for comp in rel.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(format!("`{src}`: `..` is not allowed in asset paths")),
        }
    }
    let root_real = root
        .canonicalize()
        .map_err(|e| format!("asset root `{}` is not accessible: {e}", root.display()))?;
    let full = root_real.join(rel);
    let real = full.canonicalize().map_err(|e| format!("`{src}`: {e}"))?;
    // Catches symlinks that point out of the root.
    if !real.starts_with(&root_real) {
        return Err(format!("`{src}` resolves outside the asset root"));
    }
    if !real.is_file() {
        return Err(format!("`{src}` is not a regular file"));
    }
    Ok(real)
}

/// Read an asset, enforcing the per-file size limit before reading it.
pub fn read_asset(root: &Path, src: &str, max_bytes: u64) -> Result<(PathBuf, Vec<u8>), Error> {
    let path = resolve_asset_path(root, src).map_err(|m| Error::new(ErrorCode::Asset, m))?;
    let len = std::fs::metadata(&path)
        .map_err(|e| Error::new(ErrorCode::Asset, format!("`{src}`: {e}")))?
        .len();
    if len > max_bytes {
        return Err(Error::new(
            ErrorCode::Limit,
            format!("`{src}` is {len} bytes; the limit for one asset file is {max_bytes}"),
        ));
    }
    let bytes =
        std::fs::read(&path).map_err(|e| Error::new(ErrorCode::Asset, format!("`{src}`: {e}")))?;
    Ok((path, bytes))
}

/// FNV-1a 64. Stable across runs and Rust versions, which a cache key needs.
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn accepts_files_under_the_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("img")).unwrap();
        fs::write(dir.path().join("img/a.png"), b"x").unwrap();
        let p = resolve_asset_path(dir.path(), "img/a.png").unwrap();
        assert!(p.ends_with("img/a.png"));
        assert!(resolve_asset_path(dir.path(), "./img/a.png").is_ok());
    }

    #[test]
    fn rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.png"), b"x").unwrap();
        for bad in [
            "../a.png",
            "img/../../etc/passwd",
            "/etc/passwd",
            "~/secret",
            "https://example.com/a.png",
            "file:///etc/passwd",
            "data:image/png;base64,AAAA",
            "",
        ] {
            assert!(
                resolve_asset_path(dir.path(), bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_pointing_outside() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), b"top secret").unwrap();
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            root.path().join("innocent.png"),
        )
        .unwrap();
        let err = resolve_asset_path(root.path(), "innocent.png").unwrap_err();
        assert!(err.contains("outside the asset root"), "{err}");
    }

    #[test]
    fn rejects_directories_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(resolve_asset_path(dir.path(), "sub").is_err());
        assert!(resolve_asset_path(dir.path(), "nope.png").is_err());
    }

    #[test]
    fn size_limit_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("big.bin"), vec![0u8; 2048]).unwrap();
        let err = read_asset(dir.path(), "big.bin", 1024).unwrap_err();
        assert_eq!(err.code, ErrorCode::Limit);
        assert!(read_asset(dir.path(), "big.bin", 4096).is_ok());
    }

    #[test]
    fn hash_depends_on_content_only() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
        assert_eq!(content_hash(b""), 0xcbf2_9ce4_8422_2325);
    }
}
