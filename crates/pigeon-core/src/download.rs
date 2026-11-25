//! What happens to a fetched file on the home computer: its local name, the
//! `.part` file, the final no-overwrite move, the "downloaded from elsewhere"
//! mark, and whether it is safe to open with a double click (W-17..W-20).
//!
//! Everything here is also re-exported from [`crate::client`].

use std::{
    io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

/// Path of the partial download for `out`.
pub fn part_path(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// `out`, then `name (1).ext`, `name (2).ext`, ... next to it.
fn candidates(out: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = out
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    std::iter::once(out.to_path_buf())
        .chain((1u32..).map(move |n| out.with_file_name(format!("{stem} ({n}){ext}"))))
}

/// Returns `out` if neither it nor its `.part` exists yet, otherwise the first
/// free `name (1).ext`, `name (2).ext`, ... next to it. A name whose `.part`
/// exists counts as taken, so a new download never joins someone else's
/// `.part` (W-18). Never overwrites.
pub fn free_path(out: &Path) -> PathBuf {
    candidates(out)
        .find(|p| !exists(p) && !exists(&part_path(p)))
        .expect("ran out of u32 file names")
}

/// Where `pigeon get` should write `out`, and how: if `out` is free and an
/// earlier run left `out.part` that no other download is writing, resume it;
/// otherwise start fresh at [`free_path`].
pub fn choose_out(out: &Path) -> (PathBuf, crate::client::Mode) {
    use crate::client::Mode;
    if !exists(out) && exists(&part_path(out)) && !part_in_use(out) {
        (out.to_path_buf(), Mode::Resume)
    } else {
        (free_path(out), Mode::Fresh)
    }
}

/// True while another download (in this process or another one) holds the
/// lock on `out.part`. A download locks its `.part` from open to close.
pub fn part_in_use(out: &Path) -> bool {
    let Ok(file) = std::fs::File::open(part_path(out)) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// Like `Path::exists`, but a dangling symlink also counts as taken.
pub(crate) fn exists(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}

/// Opens `part` for a download and takes an exclusive lock on it, so two
/// downloads never write one `.part` (W-18). `fresh`: the file must not
/// exist yet (`create_new`). Otherwise an existing `.part` is reused (resume).
pub(crate) fn open_part(part: &Path, fresh: bool) -> Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true);
    if fresh {
        opts.create_new(true);
    } else {
        opts.create(true);
    }
    // Fetched work files are readable by the home user only (SEC-14). On
    // Windows the file inherits the ACL of the folder it is saved in.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let file = match opts.open(part) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => bail!(
            "{} already exists (another download, or one that was cut off); \
             not starting over it",
            part.display()
        ),
        Err(e) => return Err(e).with_context(|| format!("could not open {}", part.display())),
    };
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => bail!(
            "another download is writing {} right now; wait for it or get the file again \
             under another name",
            part.display()
        ),
        // Some file systems (some network shares) have no locks. The
        // no-overwrite move and the SHA-256 check still hold there.
        Err(std::fs::TryLockError::Error(e)) if e.kind() == io::ErrorKind::Unsupported => {}
        Err(std::fs::TryLockError::Error(e)) => {
            return Err(e).with_context(|| format!("could not lock {}", part.display()));
        }
    }
    Ok(file)
}

/// Moves the finished `part` to `out`, or to the next free `name (n).ext` if
/// `out` (or that name's `.part`) appeared in the meantime. Never replaces an
/// existing file (W-18). Returns where the file landed.
pub(crate) fn finish_part(part: &Path, out: &Path) -> Result<PathBuf> {
    for (i, to) in candidates(out).enumerate().take(10_000) {
        // Our own `.part` is `part_path(out)`; for the other names a `.part`
        // means another download wants that name.
        if exists(&to) || (i > 0 && exists(&part_path(&to))) {
            continue;
        }
        match rename_no_replace(part, &to) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("could not move {} to {}", part.display(), to.display())
                });
            }
        }
    }
    bail!("no free file name next to {}", out.display())
}

/// Renames `from` to `to`; fails with `AlreadyExists` instead of replacing
/// `to`. `std::fs::rename` replaces the target on every OS.
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        win::move_no_replace(from, to)
    }
    #[cfg(not(windows))]
    {
        // A hard link is created atomically and fails if `to` exists.
        match std::fs::hard_link(from, to) {
            Ok(()) => {
                // If this fails, a full `.part` stays next to the file; it
                // only makes `free_path` skip that name.
                let _ = std::fs::remove_file(from);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
            // No hard links on this file system (FAT, exFAT, some network
            // shares): copy into a file that must be new, then drop `from`.
            Err(_) => {
                copy_no_replace(from, to)?;
                let _ = std::fs::remove_file(from);
                Ok(())
            }
        }
    }
}

#[cfg(not(windows))]
fn copy_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut dst = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)?;
    let copied = std::fs::File::open(from)
        .and_then(|mut src| io::copy(&mut src, &mut dst))
        .and_then(|_| dst.sync_all());
    if let Err(e) = copied {
        drop(dst);
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::{io, os::windows::ffi::OsStrExt, path::Path};

    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`: fails with
    /// ERROR_ALREADY_EXISTS (an `AlreadyExists` io error) if `to` exists.
    pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (wide(from), wide(to));
        // SAFETY: both buffers are NUL-terminated UTF-16 strings that outlive
        // the call; MoveFileExW only reads them.
        let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Marks `path` as downloaded from another computer, the way browsers do, so
/// the OS warns before running it (W-17):
/// - Windows: Mark of the Web, an NTFS stream `<file>:Zone.Identifier` with
///   `ZoneId=3` (Internet zone). SmartScreen and Office Protected View use it.
/// - macOS: the `com.apple.quarantine` extended attribute (Gatekeeper).
/// - Other systems: nothing to write.
///
/// Returns `Ok(true)` if a mark was written, `Ok(false)` where the OS has no
/// such mark. An error means the file system refused (e.g. FAT has no
/// streams); the file itself is fine.
pub fn mark_downloaded(path: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        std::fs::write(PathBuf::from(stream), b"[ZoneTransfer]\r\nZoneId=3\r\n")?;
        Ok(true)
    }
    #[cfg(target_os = "macos")]
    {
        mac::quarantine(path)?;
        Ok(true)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = path;
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac {
    use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::Path};

    /// Sets `com.apple.quarantine` to `flags;time;agent;` (hex seconds), the
    /// format browsers write. 0x0081 = "downloaded" as Chrome sets it.
    pub fn quarantine(path: &Path) -> io::Result<()> {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let value = format!("0081;{secs:08x};Pigeon;");
        let cpath = CString::new(path.as_os_str().as_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let name = c"com.apple.quarantine";
        // SAFETY: `cpath` and `name` are NUL-terminated C strings and `value`
        // is `value.len()` readable bytes; all outlive the call, which only
        // reads them. position 0 and options 0 are the defaults for a
        // non-resource-fork attribute.
        let rc = unsafe {
            libc::setxattr(
                cpath.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Unicode general category Cf ("format"), Unicode 16.0: invisible
/// characters that change how text around them is shown (bidi overrides and
/// isolates, zero-width spaces, BOM, tag characters, ...).
pub(crate) fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// ZWNJ / ZWJ: Cf, but part of normal Persian / Indic words and emoji.
pub(crate) fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// Characters that show as nothing (or as a line break) although they are not
/// Cf: line / paragraph separators, the combining grapheme joiner, Hangul and
/// Khmer fillers.
fn is_invisible(c: char) -> bool {
    (is_format_char(c) && !is_joiner(c))
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                | '\u{034F}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180D}' | '\u{180F}' | '\u{3164}' | '\u{FFA0}'
        )
}

/// Spaces other than U+0020 (no-break, en/em, ideographic, braille blank...).
fn is_odd_space(c: char) -> bool {
    matches!(
        c,
        '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{2800}' | '\u{3000}'
    )
}

/// Local file name for a remote `path`: its last component, made safe to
/// create on any OS and to show to the user. The agent never chooses the
/// local directory (SEC-15).
/// - `.`/`..` are refused (they would name a directory, or its parent).
/// - The characters Windows forbids (`<>:"|?*`, controls) become `_`, so a
///   Mac name like `a:b.txt` cannot turn into an NTFS stream or a
///   drive-relative path.
/// - Invisible and bidi characters are dropped (W-17): Unicode Cf (RLO
///   U+202E, RLM U+200F, isolates U+2066..2069, ZWSP U+200B, BOM U+FEFF, tags,
///   ...), line separators and fillers, so `invoice<RLO>fdp.exe` becomes
///   `invoicefdp.exe` instead of showing as `invoiceexe.pdf`. ZWNJ / ZWJ stay
///   only between two non-ASCII characters (Persian words, emoji), never next
///   to ASCII such as an extension.
/// - Other spaces become U+0020, runs of spaces become one, and leading
///   spaces go, so padding cannot push the real extension out of view.
/// - Trailing dots and spaces (which Windows drops) are removed.
/// - Windows device names get a leading `_` (W-20): `CON`, `PRN`, `AUX`,
///   `NUL`, `CONIN$`, `CONOUT$`, `COM0`-`COM9`, `COM¹`-`COM³`, `LPT0`-`LPT9`,
///   `LPT¹`-`LPT³`, case-insensitive, with any extension, and also with
///   spaces before the extension (`CON .txt`).
///
/// The result never changes under [`crate::display_safe`].
pub fn local_file_name(path: &str) -> Result<String> {
    let name = path
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .filter(|s| *s != "." && *s != "..")
        .context("cannot derive a file name from this path")?;
    let chars: Vec<char> = name.chars().collect();
    let shown = |c: Option<&char>| {
        c.is_some_and(|&c| {
            !c.is_ascii() && !is_invisible(c) && !c.is_whitespace() && !c.is_control()
        })
    };
    let mut safe = String::with_capacity(name.len());
    for (i, &c) in chars.iter().enumerate() {
        let c = if c.is_control() || "<>:\"|?*".contains(c) {
            '_'
        } else if is_joiner(c) {
            if shown(i.checked_sub(1).and_then(|j| chars.get(j))) && shown(chars.get(i + 1)) {
                c
            } else {
                continue;
            }
        } else if is_invisible(c) {
            continue;
        } else if is_odd_space(c) {
            ' '
        } else {
            c
        };
        if c == ' ' && (safe.is_empty() || safe.ends_with(' ')) {
            continue;
        }
        safe.push(c);
    }
    while safe.ends_with(['.', ' ']) {
        safe.pop();
    }
    if safe.is_empty() {
        bail!("cannot derive a file name from this path");
    }
    if is_reserved_on_windows(&safe) {
        safe.insert(0, '_');
    }
    Ok(safe)
}

/// Windows device names, which Windows reserves with any extension and
/// ignoring spaces before it (Microsoft, "Naming Files, Paths, and
/// Namespaces").
fn is_reserved_on_windows(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let stem = stem.to_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str()) {
        return true;
    }
    let mut it = stem.chars();
    let prefix: String = it.by_ref().take(3).collect();
    let rest: Vec<char> = it.collect();
    (prefix == "COM" || prefix == "LPT")
        && matches!(
            rest.as_slice(),
            ['0'..='9' | '\u{B9}' | '\u{B2}' | '\u{B3}']
        )
}

/// Extensions that run code, open a link or mount a disk image when opened
/// with a double click, on Windows, macOS or Linux.
const RISKY_EXTENSIONS: &[&str] = &[
    // Windows programs and installers
    "exe",
    "com",
    "scr",
    "pif",
    "msi",
    "msp",
    "mst",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "cpl",
    "msc",
    "dll",
    "sys",
    "drv",
    "ocx",
    "gadget",
    "application",
    "appref-ms",
    "xll",
    "diagcab",
    // Windows scripts
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "psd1",
    "ps1xml",
    "ps2",
    "psc1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "wsc",
    "ws",
    "hta",
    "chm",
    "reg",
    "inf",
    "scf",
    // shortcuts and links
    "lnk",
    "url",
    "website",
    "settingcontent-ms",
    "library-ms",
    "search-ms",
    "searchconnector-ms",
    "webloc",
    "inetloc",
    "fileloc",
    "desktop",
    // disk images (mounting can drop the downloaded mark)
    "iso",
    "img",
    "vhd",
    "vhdx",
    "dmg",
    // macOS apps, installers and scripts
    "app",
    "command",
    "tool",
    "workflow",
    "terminal",
    "pkg",
    "mpkg",
    "scpt",
    "applescript",
    // cross-platform runtimes and Unix scripts
    "jar",
    "py",
    "pyw",
    "pyz",
    "pl",
    "rb",
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "fish",
    "appimage",
    "run",
    "deb",
    "rpm",
];

/// True if opening a file with this name (double click, "Open") could run
/// code: programs, scripts, shortcuts, installers, disk images (W-17). The
/// app offers "Show in folder" instead of "Open" for these.
///
/// The name is judged the way the OS will see it: invisible / bidi characters
/// are ignored, case does not matter, trailing dots and spaces are dropped
/// (`a.exe.`, `a.exe  `), an NTFS stream suffix is cut (`a.exe::$DATA`), and
/// only the last extension counts, so `report.pdf.exe` and
/// `invoice<RLO>fdp.exe` are risky while `setup.exe.txt` is not. Both the raw
/// name and its [`local_file_name`] are checked.
pub fn is_risky_to_open(name: &str) -> bool {
    let raw = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let raw = raw.split(':').next().unwrap_or(raw);
    let local = local_file_name(name).ok();
    [Some(raw), local.as_deref()]
        .into_iter()
        .flatten()
        .any(risky_extension)
}

fn risky_extension(name: &str) -> bool {
    let clean: String = name
        .chars()
        .filter(|&c| !is_invisible(c) && !is_joiner(c) && !c.is_control())
        .collect::<String>()
        .to_lowercase();
    let clean = clean.trim_end_matches(|c: char| c == '.' || c.is_whitespace() || is_odd_space(c));
    match clean.rsplit_once('.') {
        Some((_, ext)) => RISKY_EXTENSIONS.contains(&ext.trim()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_safe;

    #[test]
    fn free_path_keeps_a_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("report.pdf");
        assert_eq!(free_path(&out), out);
    }

    #[test]
    fn free_path_numbers_taken_names() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("report.pdf");
        std::fs::write(&out, b"1").unwrap();
        assert_eq!(free_path(&out), dir.path().join("report (1).pdf"));
        std::fs::write(dir.path().join("report (1).pdf"), b"2").unwrap();
        assert_eq!(free_path(&out), dir.path().join("report (2).pdf"));
    }

    #[test]
    fn free_path_without_extension_and_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("Makefile");
        std::fs::write(&plain, b"").unwrap();
        assert_eq!(free_path(&plain), dir.path().join("Makefile (1)"));
        let dot = dir.path().join(".env");
        std::fs::write(&dot, b"").unwrap();
        assert_eq!(free_path(&dot), dir.path().join(".env (1)"));
    }

    #[test]
    fn free_path_treats_a_directory_as_taken() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("photos");
        std::fs::create_dir(&out).unwrap();
        assert_eq!(free_path(&out), dir.path().join("photos (1)"));
    }

    /// W-18: a name whose `.part` exists is taken, so a new download never
    /// appends to another download's `.part`.
    #[test]
    fn free_path_treats_a_part_file_as_taken() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.bin");
        std::fs::write(part_path(&out), b"someone else's bytes").unwrap();
        assert_eq!(free_path(&out), dir.path().join("a (1).bin"));
    }

    #[test]
    fn choose_out_resumes_only_a_free_unlocked_part() {
        use crate::client::Mode;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.bin");
        assert_eq!(choose_out(&out), (out.clone(), Mode::Fresh));

        std::fs::write(part_path(&out), b"half").unwrap();
        assert_eq!(choose_out(&out), (out.clone(), Mode::Resume));

        // Another download holds the lock: go to the next name.
        let held = open_part(&part_path(&out), false).unwrap();
        assert!(part_in_use(&out));
        assert_eq!(
            choose_out(&out),
            (dir.path().join("a (1).bin"), Mode::Fresh)
        );
        drop(held);
        assert!(!part_in_use(&out));

        // The finished file is there: never resume into it.
        std::fs::write(&out, b"done").unwrap();
        assert_eq!(
            choose_out(&out),
            (dir.path().join("a (1).bin"), Mode::Fresh)
        );
    }

    #[test]
    fn open_part_fresh_refuses_an_existing_part_and_lock_excludes() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("x.part");
        let first = open_part(&part, true).unwrap();
        let err = open_part(&part, true).unwrap_err();
        assert!(format!("{err:#}").contains("already exists"), "{err:#}");
        let err = open_part(&part, false).unwrap_err();
        assert!(format!("{err:#}").contains("another download"), "{err:#}");
        drop(first);
        open_part(&part, false).unwrap();
    }

    #[test]
    fn finish_part_never_replaces_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.txt");
        let part = part_path(&out);
        std::fs::write(&part, b"new").unwrap();
        std::fs::write(&out, b"precious").unwrap();
        // `a (1).txt` has a `.part` of another download: skip it too.
        std::fs::write(part_path(&dir.path().join("a (1).txt")), b"other").unwrap();
        let landed = finish_part(&part, &out).unwrap();
        assert_eq!(landed, dir.path().join("a (2).txt"));
        assert_eq!(std::fs::read(&out).unwrap(), b"precious");
        assert_eq!(std::fs::read(&landed).unwrap(), b"new");
        assert!(!part.exists());
    }

    #[test]
    fn rename_no_replace_refuses_an_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let err = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{err}");
        assert_eq!(std::fs::read(&b).unwrap(), b"b");
        assert_eq!(std::fs::read(&a).unwrap(), b"a");
        let c = dir.path().join("c");
        rename_no_replace(&a, &c).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(&c).unwrap(), b"a");
    }

    #[test]
    fn local_file_name_is_safe_on_every_os() {
        assert_eq!(local_file_name("Reports/q4.pdf").unwrap(), "q4.pdf");
        assert_eq!(local_file_name("a\\b\\c.txt").unwrap(), "c.txt");
        assert_eq!(local_file_name("dir/").unwrap(), "dir");
        assert_eq!(local_file_name("a:b.txt").unwrap(), "a_b.txt");
        assert_eq!(local_file_name("x\u{1b}y?.txt").unwrap(), "x_y_.txt");
        assert_eq!(local_file_name("name. .").unwrap(), "name");
        assert_eq!(local_file_name("CON.txt").unwrap(), "_CON.txt");
        assert_eq!(local_file_name("com1").unwrap(), "_com1");
        assert_eq!(local_file_name("COM10").unwrap(), "COM10");
        for bad in ["", "/", "..", "a/..", ".", "...", "  "] {
            assert!(local_file_name(bad).is_err(), "{bad:?}");
        }
    }

    /// W-17: bidi overrides, isolates, marks, zero-width spaces and BOMs
    /// cannot disguise a name.
    #[test]
    fn local_file_name_drops_bidi_and_invisible_characters() {
        let cases = [
            ("invoice\u{202E}fdp.exe", "invoicefdp.exe"), // RLO
            ("invoice\u{202D}x.exe", "invoicex.exe"),     // LRO
            ("a\u{2066}b\u{2069}.txt", "ab.txt"),         // LRI .. PDI
            ("a\u{2067}\u{2068}b.txt", "ab.txt"),         // RLI, FSI
            ("x\u{200F}y\u{200E}.pdf", "xy.pdf"),         // RLM, LRM
            ("\u{FEFF}report.pdf", "report.pdf"),         // BOM
            ("pay\u{200B}roll.xlsx", "payroll.xlsx"),     // ZWSP
            ("a\u{2060}b\u{00AD}c.doc", "abc.doc"),       // word joiner, soft hyphen
            ("a\u{061C}b.txt", "ab.txt"),                 // Arabic letter mark
            ("tag\u{E0041}\u{E007F}.txt", "tag.txt"),     // tag characters
            ("report.pdf\u{2028}.exe", "report.pdf.exe"), // line separator
            ("a\u{3164}.exe", "a.exe"),                   // Hangul filler
            ("a\u{200D}.exe", "a.exe"),                   // ZWJ next to ASCII
            ("pdf\u{200C}.exe", "pdf.exe"),               // ZWNJ next to ASCII
            ("report.pdf          .exe", "report.pdf .exe"),
            ("a\u{3000}\u{00A0}b.txt", "a b.txt"),
            ("  lead.txt", "lead.txt"),
        ];
        for (remote, local) in cases {
            assert_eq!(local_file_name(remote).unwrap(), local, "{remote:?}");
        }
        for empty in ["\u{FEFF}", "\u{202E}\u{2066}", "\u{200B}.", "\u{2028}"] {
            assert!(local_file_name(empty).is_err(), "{empty:?}");
        }
    }

    #[test]
    fn local_file_name_keeps_normal_names_of_any_script() {
        for name in [
            "Hisobot_o\u{2BB}zbekcha.xlsx",
            "\u{41e}\u{442}\u{447}\u{451}\u{442}.pdf",
            "Cafe\u{301} NFD.txt",
            "\u{62a}\u{642}\u{631}\u{64a}\u{631}.pdf",
            "\u{645}\u{6cc}\u{200C}\u{62e}\u{648}\u{627}\u{647}\u{645}.txt", // Persian ZWNJ
            "fam \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}.png",           // emoji ZWJ
            "\u{65e5}\u{672c}\u{8a9e} \u{30d5}\u{30a1}\u{30a4}\u{30eb}.txt",
            "heart \u{2764}\u{FE0F}.png", // variation selector
        ] {
            assert_eq!(local_file_name(name).unwrap(), name);
        }
    }

    /// W-20: every Windows device name gets a `_`.
    #[test]
    fn local_file_name_escapes_all_windows_device_names() {
        let reserved = [
            "CON",
            "con.txt",
            "PRN.log",
            "aux",
            "NUL.tar.gz",
            "CONIN$",
            "conout$.txt",
            "COM0",
            "com9.x",
            "LPT1",
            "lpt0",
            "COM\u{B9}.txt",
            "COM\u{B2}",
            "LPT\u{B3}.log",
            "lpt\u{B9}",
            "CON .txt",
            "nul   .txt",
            "AUX\u{3000}.txt",
            "CON..txt",
        ];
        for name in reserved {
            let local = local_file_name(name).unwrap();
            assert!(local.starts_with('_'), "{name:?} -> {local:?}");
        }
        for name in [
            "COM10",
            "LPT10",
            "console.txt",
            "CONIN",
            "COM",
            "COMA",
            "LPT\u{B4}",
            "nullable",
            "a CON.txt",
            "COM\u{B9}0",
        ] {
            let local = local_file_name(name).unwrap();
            assert!(!local.starts_with('_'), "{name:?} -> {local:?}");
        }
        assert_eq!(local_file_name("CON .txt").unwrap(), "_CON .txt");
        assert_eq!(local_file_name("COM\u{B9}.txt").unwrap(), "_COM\u{B9}.txt");
    }

    #[test]
    fn risky_names_are_refused_for_open() {
        for name in [
            "setup.exe",
            "SETUP.EXE",
            "report.pdf.exe",
            "invoice\u{202E}fdp.exe",
            "a.exe.",
            "a.exe   ",
            "a.exe\u{3000}",
            "a.exe::$DATA",
            "a.ex\u{200B}e",
            "Dir/run.bat",
            "x.cmd",
            "x.com",
            "x.scr",
            "x.ps1",
            "x.vbs",
            "x.js",
            "x.jse",
            "x.wsf",
            "Report.pdf.lnk",
            "x.url",
            "x.hta",
            "x.cpl",
            "x.msc",
            "x.msi",
            "x.jar",
            "Thing.app",
            "x.command",
            "x.pkg",
            "x.dmg",
            "x.sh",
            "x.iso",
            "x.scf",
            "evil:x.exe", // becomes evil_x.exe locally
        ] {
            assert!(is_risky_to_open(name), "{name:?} should be risky");
        }
        for name in [
            "report.pdf",
            "photo.jpg",
            "setup.exe.txt",
            "invoice\u{202E}exe.pdf", // really a .pdf, whatever it shows
            "notes",
            "exe",
            ".bashrc",
            "data.csv",
            "a.exe.zip",
        ] {
            assert!(!is_risky_to_open(name), "{name:?} should be fine");
        }
    }

    #[test]
    fn mark_downloaded_writes_the_os_mark() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("got.txt");
        std::fs::write(&file, b"x").unwrap();
        let marked = mark_downloaded(&file).unwrap();
        #[cfg(windows)]
        {
            assert!(marked);
            let mut stream = file.as_os_str().to_owned();
            stream.push(":Zone.Identifier");
            let text = std::fs::read_to_string(PathBuf::from(stream)).unwrap();
            assert_eq!(text, "[ZoneTransfer]\r\nZoneId=3\r\n");
        }
        #[cfg(target_os = "macos")]
        {
            assert!(marked);
            let out = std::process::Command::new("/usr/bin/xattr")
                .args(["-p", "com.apple.quarantine"])
                .arg(&file)
                .output()
                .unwrap();
            let value = String::from_utf8_lossy(&out.stdout);
            assert!(
                value.starts_with("0081;") && value.contains(";Pigeon;"),
                "{value:?}"
            );
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        assert!(!marked);
        // The file content is untouched.
        assert_eq!(std::fs::read(&file).unwrap(), b"x");
    }

    use proptest::strategy::Strategy;

    proptest::proptest! {
        /// Whatever the agent names a file, the local name shows as itself
        /// (no escapes needed), has no Cf/bidi character outside a joiner
        /// between two non-ASCII characters, and is never a device name.
        #[test]
        fn local_file_name_is_always_display_safe(
            s in proptest::collection::vec(
                proptest::prop_oneof![
                    proptest::char::range('\u{0}', '\u{7f}'),
                    proptest::sample::select(vec![
                        '\u{202E}', '\u{202D}', '\u{2066}', '\u{2069}', '\u{200B}', '\u{200C}',
                        '\u{200D}', '\u{200E}', '\u{200F}', '\u{FEFF}', '\u{2028}', '\u{00AD}',
                        '\u{3000}', '\u{00A0}', '\u{E0041}', '\u{1F600}', '\u{0627}', '\u{B9}',
                        '\u{3164}', '\u{061C}',
                    ]),
                    proptest::char::any(),
                ],
                0..24,
            ).prop_map(|v: Vec<char>| v.into_iter().collect::<String>())
        ) {
            let s: String = s;
            if let Ok(name) = local_file_name(&s) {
                proptest::prop_assert_eq!(display_safe(&name), name.clone());
                proptest::prop_assert!(!name.chars().any(|c| c.is_control()
                    || "<>:\"|?*/\\".contains(c)
                    || is_invisible(c)
                    || is_odd_space(c)));
                proptest::prop_assert!(!name.ends_with(['.', ' ']) && !name.starts_with(' '));
                proptest::prop_assert!(!is_reserved_on_windows(&name));
                proptest::prop_assert!(!name.contains("  "));
            }
        }
    }
}
