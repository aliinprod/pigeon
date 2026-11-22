//! Path guard: maps a path requested by a remote peer onto the shared root.
//!
//! Every request goes through [`resolve`]. It rejects absolute paths (Unix and
//! Windows style), `..` components, drive prefixes, and anything whose
//! canonicalized path does not stay under the canonicalized root (this covers
//! symlinks that point outside the shared folder).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Resolves `requested` (a `/`-separated path relative to the share root) to a
/// canonical path that is guaranteed to be inside `root`.
///
/// An empty string or `.` resolves to the root itself. The target must exist.
pub fn resolve(root: &Path, requested: &str) -> Result<PathBuf> {
    if requested.contains('\0') {
        bail!("path contains a NUL byte");
    }
    // Reject absolute paths in any style, regardless of the host OS.
    if requested.starts_with('/') || requested.starts_with('\\') {
        bail!("absolute paths are not allowed: {requested}");
    }
    let bytes = requested.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        bail!("drive paths are not allowed: {requested}");
    }
    if Path::new(requested).is_absolute() || Path::new(requested).has_root() {
        bail!("absolute paths are not allowed: {requested}");
    }

    // Split on both separators so `a\..\..\x` is caught on Unix too.
    let mut relative = PathBuf::new();
    for part in requested.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => bail!("`..` is not allowed: {requested}"),
            p if p.contains(':') => bail!("`:` is not allowed in path components: {requested}"),
            p => relative.push(p),
        }
    }

    let root = root
        .canonicalize()
        // No absolute path here: this message is sent to the remote peer (SEC-10).
        .context("the shared folder is not available")?;
    let full = root
        .join(&relative)
        .canonicalize()
        .with_context(|| format!("not found: {requested}"))?;
    if !full.starts_with(&root) {
        bail!("path escapes the shared folder: {requested}");
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("share/a/b")).unwrap();
        fs::write(dir.path().join("share/a/b/file.txt"), b"hi").unwrap();
        fs::write(dir.path().join("secret.txt"), b"secret").unwrap();
        dir
    }

    #[test]
    fn normal_nested_file() {
        let dir = setup();
        let root = dir.path().join("share");
        let p = resolve(&root, "a/b/file.txt").unwrap();
        assert_eq!(fs::read(p).unwrap(), b"hi");
        // Backslash separators work too.
        assert!(resolve(&root, "a\\b\\file.txt").is_ok());
        // Empty and "." are the root.
        assert_eq!(resolve(&root, "").unwrap(), root.canonicalize().unwrap());
        assert_eq!(resolve(&root, ".").unwrap(), root.canonicalize().unwrap());
    }

    #[test]
    fn rejects_parent_dir() {
        let dir = setup();
        let root = dir.path().join("share");
        assert!(resolve(&root, "../etc").is_err());
        assert!(resolve(&root, "../secret.txt").is_err());
        assert!(resolve(&root, "a/../../x").is_err());
        assert!(resolve(&root, "a\\..\\..\\secret.txt").is_err());
        // Even a `..` that would stay inside is rejected: keep it simple.
        assert!(resolve(&root, "a/../a/b/file.txt").is_err());
    }

    #[test]
    fn rejects_absolute_paths() {
        let dir = setup();
        let root = dir.path().join("share");
        assert!(resolve(&root, "/etc/passwd").is_err());
        assert!(resolve(&root, "\\Windows\\System32").is_err());
        assert!(resolve(&root, "C:\\Windows\\System32").is_err());
        assert!(resolve(&root, "C:/Windows").is_err());
        assert!(resolve(&root, "c:secret.txt").is_err());
        assert!(resolve(&root, "\\\\server\\share\\x").is_err());
        let abs = dir.path().join("secret.txt");
        assert!(resolve(&root, abs.to_str().unwrap()).is_err());
    }

    #[test]
    fn rejects_missing() {
        let dir = setup();
        let root = dir.path().join("share");
        assert!(resolve(&root, "nope.txt").is_err());
    }

    #[test]
    fn rejects_symlink_escape() {
        let dir = setup();
        let root = dir.path().join("share");
        let target = dir.path().join("secret.txt");
        let link = root.join("link.txt");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&target, &link);
        if made.is_err() {
            eprintln!("skipping: cannot create symlinks here");
            return;
        }
        assert!(resolve(&root, "link.txt").is_err());

        // A symlink to a directory outside the root is rejected too.
        let outside_dir = dir.path().join("outside");
        fs::create_dir(&outside_dir).unwrap();
        fs::write(outside_dir.join("x.txt"), b"x").unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside_dir, root.join("dirlink"));
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside_dir, root.join("dirlink"));
        if made.is_ok() {
            assert!(resolve(&root, "dirlink/x.txt").is_err());
        }
    }
}

/// Extra adversarial cases added by Proof (verifier).
#[cfg(test)]
mod proof_tests {
    use super::*;
    use std::fs;

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("share/a")).unwrap();
        fs::write(dir.path().join("share/a/file.txt"), b"hi").unwrap();
        fs::write(dir.path().join("secret.txt"), b"secret").unwrap();
        dir
    }

    #[test]
    fn odd_components_never_escape() {
        let dir = setup();
        let root = dir.path().join("share");
        let canon_root = root.canonicalize().unwrap();
        for p in [
            "...",
            "..../secret.txt",
            ".. ",
            ".. /secret.txt",
            "..%2fsecret.txt",
            "%2e%2e/secret.txt",
            "%2e%2e%5csecret.txt",
            "a/./../../secret.txt",
            "a//..//..//secret.txt",
            "\u{2025}/secret.txt",
            "\u{ff0e}\u{ff0e}/secret.txt",
            "a/file.txt::$DATA",
            "a/file.txt:stream",
            r"\\?\C:\Windows",
            r"\\.\C:\Windows",
            r"\\?\UNC\server\share",
            "a/C:secret.txt",
            "//server/share",
            "CON",
            "NUL",
            "a/NUL.txt",
            "COM1",
            "secret.txt",
        ] {
            match resolve(&root, p) {
                Err(_) => {}
                Ok(full) => assert!(
                    full.starts_with(&canon_root),
                    "{p:?} resolved outside the root: {}",
                    full.display()
                ),
            }
        }
    }

    #[test]
    fn long_paths_inside_root_work() {
        let dir = setup();
        let root = dir.path().join("share");
        // > 260 chars total, to exercise Windows MAX_PATH handling.
        let mut rel = String::new();
        for i in 0..12 {
            if !rel.is_empty() {
                rel.push('/');
            }
            rel.push_str(&format!("{i:02}-{}", "d".repeat(24)));
        }
        let mut full = root.clone();
        for part in rel.split('/') {
            full.push(part);
        }
        assert!(full.as_os_str().len() > 260);
        if fs::create_dir_all(&full).is_err() {
            eprintln!("skipping: long paths not supported by this filesystem");
            return;
        }
        fs::write(full.join("deep.txt"), b"deep").unwrap();
        let p = resolve(&root, &format!("{rel}/deep.txt")).unwrap();
        assert_eq!(fs::read(p).unwrap(), b"deep");
    }

    /// Junctions need no admin rights on Windows, so they are the realistic
    /// way a share root ends up pointing outside itself there.
    #[cfg(windows)]
    #[test]
    fn rejects_junction_escape() {
        let dir = setup();
        let root = dir.path().join("share");
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("x.txt"), b"x").unwrap();
        let link = root.join("junc");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(status.status.success(), "mklink /J failed: {status:?}");
        assert!(link.join("x.txt").exists());
        assert!(resolve(&root, "junc/x.txt").is_err());
        assert!(resolve(&root, "junc").is_err());
    }
}
