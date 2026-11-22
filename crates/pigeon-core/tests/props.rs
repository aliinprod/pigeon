//! Property tests (stable Rust, no nightly needed) for the three parsers that
//! see untrusted input: the frame decoder, the ticket decoder and the path
//! guard. Random inputs must never panic, never allocate past the frame limit
//! and never resolve outside the share root (SEC-03, SEC-05, SEC-06).
//!
//! Coverage-guided fuzzing (cargo-fuzz) needs nightly and is a later step.

use std::path::Path;

use pigeon_core::{
    Ticket, guard,
    proto::{Hello, MAX_HELLO_LEN, Request, Response, read_frame, read_frame_max, write_frame},
};
use proptest::prelude::*;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Arbitrary bytes on the wire: decoding may fail, but must not panic.
    #[test]
    fn frame_decoder_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        rt().block_on(async {
            let _ = read_frame_max::<_, Hello>(&mut bytes.as_slice(), MAX_HELLO_LEN).await;
            let _ = read_frame::<_, Request>(&mut bytes.as_slice()).await;
            let _ = read_frame::<_, Response>(&mut bytes.as_slice()).await;
        });
    }

    /// A valid length prefix followed by arbitrary body bytes (reaches deeper
    /// into the postcard decoder than fully random input).
    #[test]
    fn frame_body_fuzz_never_panics(body in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut wire = (body.len() as u32).to_be_bytes().to_vec();
        wire.extend_from_slice(&body);
        rt().block_on(async {
            let _ = read_frame_max::<_, Hello>(&mut wire.as_slice(), MAX_HELLO_LEN).await;
            let _ = read_frame::<_, Request>(&mut wire.as_slice()).await;
            let _ = read_frame::<_, Response>(&mut wire.as_slice()).await;
        });
    }

    /// Any length prefix above the limit is refused from the prefix alone.
    #[test]
    fn oversized_prefix_is_refused(len in (MAX_HELLO_LEN + 1)..=u32::MAX) {
        let wire = len.to_be_bytes();
        let res = rt().block_on(read_frame_max::<_, Hello>(&mut wire.as_slice(), MAX_HELLO_LEN));
        let err = format!("{:#}", res.unwrap_err());
        prop_assert!(err.contains("too large"), "{}", err);
    }

    /// Requests survive an encode/decode round trip unchanged.
    #[test]
    fn request_roundtrip(path in ".{0,200}", offset in any::<u64>(), get in any::<bool>()) {
        let req = if get { Request::Get { path, offset } } else { Request::List { path } };
        let back: Request = rt().block_on(async {
            let mut buf = Vec::new();
            write_frame(&mut buf, &req).await.unwrap();
            read_frame(&mut buf.as_slice()).await.unwrap()
        });
        prop_assert_eq!(req, back);
    }

    /// Ticket parsing of arbitrary text never panics.
    #[test]
    fn ticket_parse_never_panics(s in ".{0,400}") {
        let _ = s.parse::<Ticket>();
        let _ = format!("pigeon{s}").parse::<Ticket>();
    }

    /// Well-formed base32 with random content never panics.
    #[test]
    fn ticket_base32_fuzz_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let text = format!(
            "pigeon{}",
            data_encoding::BASE32_NOPAD.encode(&bytes).to_ascii_lowercase()
        );
        let _ = text.parse::<Ticket>();
    }

    /// Any token survives Display -> FromStr.
    #[test]
    fn ticket_roundtrip(token in any::<[u8; 32]>(), seed in any::<[u8; 32]>()) {
        let key = iroh::SecretKey::from_bytes(&seed);
        let t = Ticket { addr: iroh::EndpointAddr::new(key.public()), token };
        let back: Ticket = t.to_string().parse().unwrap();
        prop_assert_eq!(t, back);
    }
}

/// Path pieces chosen to hit the guard's edge cases, mixed with random text.
fn path_piece() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("..".to_string()),
        Just(".".to_string()),
        Just(String::new()),
        Just("a".to_string()),
        Just("a/b".to_string()),
        Just("file.txt".to_string()),
        Just("link".to_string()),
        Just("dirlink".to_string()),
        Just("up".to_string()),
        Just("C:".to_string()),
        Just(r"\\?\".to_string()),
        Just("~".to_string()),
        Just("%2e%2e".to_string()),
        Just("\u{ff0e}\u{ff0e}".to_string()),
        Just("secret.txt".to_string()),
        Just("outside".to_string()),
        "[a-z.]{1,4}",
        ".{0,8}",
    ]
}

fn path_strategy() -> impl Strategy<Value = String> {
    (
        proptest::collection::vec(path_piece(), 0..8),
        proptest::collection::vec(prop_oneof![Just('/'), Just('\\')], 8),
        any::<bool>(),
    )
        .prop_map(|(pieces, seps, lead)| {
            let mut s = String::new();
            if lead {
                s.push('/');
            }
            for (i, p) in pieces.iter().enumerate() {
                if i > 0 {
                    s.push(seps[i % seps.len()]);
                }
                s.push_str(p);
            }
            s
        })
}

/// A share with nested files, files outside it, and (where the OS allows)
/// symlinks pointing both inside and outside.
fn share() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("share");
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("a/b/file.txt"), b"in").unwrap();
    std::fs::write(root.join("file.txt"), b"in").unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"out").unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("file.txt"), b"out").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(dir.path().join("secret.txt"), root.join("link"));
        let _ = symlink(&outside, root.join("dirlink"));
        let _ = symlink(root.join("a"), root.join("a/b/up"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let _ = symlink_file(dir.path().join("secret.txt"), root.join("link"));
        let _ = symlink_dir(&outside, root.join("dirlink"));
        let _ = symlink_dir(root.join("a"), root.join("a/b/up"));
    }
    (dir, root)
}

fn assert_inside(root: &Path, canon_root: &Path, p: &str) -> Result<(), TestCaseError> {
    if let Ok(full) = guard::resolve(root, p) {
        prop_assert!(
            full.starts_with(canon_root),
            "{:?} resolved outside the root: {}",
            p,
            full.display()
        );
        // Nothing that resolves may be one of the outside files.
        if let Ok(bytes) = std::fs::read(&full) {
            prop_assert_ne!(bytes, b"out".to_vec(), "{:?} read an outside file", p);
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The guard never returns a path outside the canonical root, for any mix
    /// of `..`, separators, drive prefixes, symlinks and random text.
    #[test]
    fn guard_never_escapes_root(p in path_strategy()) {
        let (_dir, root) = share();
        let canon_root = root.canonicalize().unwrap();
        assert_inside(&root, &canon_root, &p)?;
    }

    /// Fully random strings: never panic, never escape.
    #[test]
    fn guard_random_text_never_escapes(p in ".{0,64}") {
        let (_dir, root) = share();
        let canon_root = root.canonicalize().unwrap();
        assert_inside(&root, &canon_root, &p)?;
    }
}
