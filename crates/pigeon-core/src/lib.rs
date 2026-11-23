//! Pigeon core: protocol types, path guard and the shared transfer code used by
//! both the agent and the CLI.

// `unsafe` is only allowed where it is unavoidable (the Windows Credential
// Manager FFI in `keystore`), and each use is justified in place.
#![deny(unsafe_code)]

pub mod agent;
pub mod client;
pub mod config;
pub mod guard;
pub mod keystore;
pub mod proto;
pub mod ticket;

pub use ticket::Ticket;

/// ALPN identifying the Pigeon protocol on an iroh connection.
pub const ALPN: &[u8] = b"pigeon/0";

/// Placeholder printed instead of secrets in `Debug` output.
pub(crate) const REDACTED: &str = "<redacted>";

/// Short, human-comparable name for a device: the first 8 base32 characters
/// of its endpoint ID (PAIRING.md section 4, step 8).
pub fn fingerprint(id: &iroh::EndpointId) -> String {
    data_encoding::BASE32_NOPAD.encode(id.as_bytes())[..8].to_ascii_lowercase()
}

/// Hex-encodes a SHA-256 digest.
pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// Makes a string that came from the other side (file names, agent error
/// messages) safe to print in a terminal: control characters (ESC, CR, BEL,
/// ...) and Unicode bidi / invisible formatting characters are shown as
/// `\u{..}` escapes instead of being interpreted. A file name on the work
/// computer must never be able to rewrite the home user's terminal (SEC-12).
pub fn display_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        // ZWNJ / ZWJ (U+200C, U+200D) are deliberately kept: they are part of
        // normal Persian / Indic words and emoji sequences, and escaping them
        // would make listed names impossible to copy back into `get`.
        let bidi_or_invisible = matches!(
            c,
            '\u{061C}'
                | '\u{200B}'
                | '\u{200E}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        );
        if c.is_control() || bidi_or_invisible {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Generates a fresh random 32-byte access token (Day-1 auth).
pub fn new_token() -> anyhow::Result<[u8; 32]> {
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).map_err(|e| anyhow::anyhow!("no OS randomness: {e}"))?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_safe_neutralises_terminal_escapes() {
        assert_eq!(display_safe("report.pdf"), "report.pdf");
        assert_eq!(display_safe("Résumé ü.txt"), "Résumé ü.txt");
        let evil = "a\u{1b}]0;pwned\u{7}\u{1b}[2Jb\r\n";
        let shown = display_safe(evil);
        assert!(!shown.chars().any(|c| c.is_control()), "{shown:?}");
        assert_eq!(shown, r"a\u{1b}]0;pwned\u{7}\u{1b}[2Jb\u{d}\u{a}");
        // A right-to-left override can make `x<RLO>fdp.exe` look like `xexe.pdf`.
        assert_eq!(display_safe("x\u{202E}fdp.exe"), r"x\u{202e}fdp.exe");
        assert_eq!(display_safe("a\u{061C}b"), r"a\u{61c}b");
    }

    #[test]
    fn display_safe_keeps_normal_names_of_any_script() {
        for name in [
            "Hisobot_oʻzbekcha_gʻalaba.xlsx", // Uzbek Latin (U+02BB)
            "Отчёт за ноябрь.pdf",            // Cyrillic
            "Ҳисобот Ўзбекистон.txt",         // Uzbek Cyrillic
            "Cafe\u{301} NFD.txt",            // macOS NFD combining accent
            "تقرير.pdf",                      // Arabic letters (RTL, no controls)
            "می\u{200C}خواهم.txt",            // Persian with ZWNJ
            "fam \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}.png", // emoji ZWJ
            "日本語 ファイル.txt",
            "  two  spaces .txt",
        ] {
            assert_eq!(display_safe(name), name, "normal name was mangled");
        }
    }
}
