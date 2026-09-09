//! Filename decoding for archive formats that store raw bytes rather than
//! Unicode.
//!
//! ZIP and tar predate Unicode. ZIP only guarantees UTF-8 when general-purpose
//! bit 11 is set; without it the name is whatever code page the creating
//! machine used — CP936 (GBK) on Chinese Windows, CP932 (Shift_JIS) on
//! Japanese, CP949 on Korean, CP950 (Big5) on Traditional Chinese. tar stores
//! bytes with no encoding field at all. Reading those as CP437 (what the `zip`
//! crate does) or as lossy UTF-8 (what tar does) turns `中文文件.txt` into
//! `ÖÐÎÄÎÄ¼þ.txt`, in the listing and on disk.
//!
//! So: a name that is valid UTF-8 is taken as UTF-8, and anything else is
//! decoded with a legacy encoding worked out from the archive itself.
//!
//! Two things make that guess reliable:
//!
//! * **Detection is per archive, not per entry.** Every name in one archive
//!   came from one machine, so all of them are fed to the detector before any
//!   is decoded, and the answer is then locked in — entries of the same archive
//!   can never disagree with each other.
//! * **The machine's own code page breaks ties.** chardetng is tuned for web
//!   pages; on a sample as short as a single filename it sometimes lands on a
//!   single-byte Western encoding. When the guess is not a CJK encoding but
//!   this machine's code page is one, and the bytes are valid in it, we take
//!   the code page — an archive with legacy names is usually being opened on
//!   the same kind of system that wrote it.
//!
//! The trade-off in that last rule: a genuinely Western CP1252 name opened on a
//! Chinese machine can be read as GBK. That is the same bargain 7-Zip and
//! Windows make, and it is the far rarer direction.

use chardetng::EncodingDetector;
use encoding_rs::Encoding;

/// Decodes archive-internal filenames, learning the archive's legacy encoding
/// as it goes.
pub struct NameDecoder {
    detector: EncodingDetector,
    /// Whether any non-UTF-8 name has been offered to the detector.
    saw_legacy: bool,
    /// The encoding chosen on the first legacy name we had to decode.
    locked: Option<&'static Encoding>,
    /// This machine's code page, used when detection comes back non-CJK.
    fallback: Option<&'static Encoding>,
}

impl Default for NameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl NameDecoder {
    pub fn new() -> Self {
        Self::with_fallback(system_code_page())
    }

    /// Same, with the machine code page supplied explicitly. Tests use this so
    /// they don't depend on the locale of whoever runs them.
    pub fn with_fallback(fallback: Option<&'static Encoding>) -> Self {
        Self {
            detector: EncodingDetector::new(),
            saw_legacy: false,
            locked: None,
            fallback: fallback.filter(|e| is_cjk(e)),
        }
    }

    /// Offer a raw name to improve the guess. Formats that can cheaply see all
    /// their names up front (ZIP reads the whole central directory on open)
    /// should sample every one before decoding any; streaming formats (tar) can
    /// only sample each name just before decoding it.
    ///
    /// UTF-8 names say nothing about the legacy encoding, and samples arriving
    /// after the encoding is locked cannot change it, so both are ignored.
    pub fn sample(&mut self, raw: &[u8]) {
        if self.locked.is_some() || std::str::from_utf8(raw).is_ok() {
            return;
        }
        self.detector.feed(raw, false);
        self.saw_legacy = true;
    }

    /// Decode one raw name.
    pub fn decode(&mut self, raw: &[u8]) -> String {
        if let Ok(s) = std::str::from_utf8(raw) {
            return s.to_string();
        }
        let encoding = match self.locked {
            Some(e) => e,
            None => {
                // Make sure the name being decoded is part of the sample, even
                // if the caller never offered it.
                if !self.saw_legacy {
                    self.detector.feed(raw, false);
                    self.saw_legacy = true;
                }
                let e = self.choose(raw);
                self.locked = Some(e);
                e
            }
        };
        encoding.decode(raw).0.into_owned()
    }

    /// Pick the encoding for this archive: the detector's answer if it is a CJK
    /// one, otherwise this machine's code page when the bytes fit it cleanly.
    fn choose(&self, raw: &[u8]) -> &'static Encoding {
        // `allow_utf8: false` — we only get here for bytes that are not UTF-8.
        let guessed = self.detector.guess(None, false);
        if is_cjk(guessed) {
            return guessed;
        }
        match self.fallback {
            Some(cp) if decodes_cleanly(cp, raw) => cp,
            _ => guessed,
        }
    }

    /// The legacy encoding this archive was decoded with, once one has been
    /// chosen. `None` means every name so far was valid UTF-8.
    #[cfg(test)]
    pub fn detected(&self) -> Option<&'static str> {
        self.locked.map(|e| e.name())
    }
}

/// The multi-byte East Asian encodings. A guess outside this set, for bytes we
/// already know are not UTF-8, is the case where the code-page fallback earns
/// its keep.
fn is_cjk(e: &'static Encoding) -> bool {
    matches!(
        e.name(),
        "GBK" | "gb18030" | "Big5" | "Shift_JIS" | "EUC-JP" | "EUC-KR" | "ISO-2022-JP"
    )
}

/// Whether `raw` decodes under `encoding` without a single replacement
/// character — i.e. the bytes really are valid in that code page.
fn decodes_cleanly(encoding: &'static Encoding, raw: &[u8]) -> bool {
    let (_, had_errors) = encoding.decode_with_bom_removal(raw);
    !had_errors
}

/// This machine's non-Unicode code page, if it is one of the East Asian ones.
#[cfg(windows)]
fn system_code_page() -> Option<&'static Encoding> {
    // GetACP is the code page Windows itself uses for the "ANSI" APIs, which is
    // exactly the one that named the entries in a locally-made ZIP.
    let acp = unsafe { windows_sys::Win32::Globalization::GetACP() };
    code_page_encoding(acp)
}

#[cfg(windows)]
fn code_page_encoding(acp: u32) -> Option<&'static Encoding> {
    Some(match acp {
        936 => encoding_rs::GBK,
        950 => encoding_rs::BIG5,
        932 => encoding_rs::SHIFT_JIS,
        949 => encoding_rs::EUC_KR,
        _ => return None,
    })
}

/// On Unix there is no ANSI code page; the language part of the locale is the
/// best available stand-in for "what kind of machine is this".
#[cfg(not(windows))]
fn system_code_page() -> Option<&'static Encoding> {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .unwrap_or_default();
    locale_encoding(&locale)
}

#[cfg(not(windows))]
fn locale_encoding(locale: &str) -> Option<&'static Encoding> {
    let l = locale.to_ascii_lowercase();
    Some(match () {
        _ if l.starts_with("zh_tw") || l.starts_with("zh_hk") || l.starts_with("zh_mo") => {
            encoding_rs::BIG5
        }
        _ if l.starts_with("zh") => encoding_rs::GBK,
        _ if l.starts_with("ja") => encoding_rs::SHIFT_JIS,
        _ if l.starts_with("ko") => encoding_rs::EUC_KR,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(encoding: &'static Encoding, s: &str) -> Vec<u8> {
        encoding.encode(s).0.into_owned()
    }

    /// A decoder that cannot fall back, so these tests exercise detection alone
    /// and give the same answer on every machine.
    fn detector_only() -> NameDecoder {
        NameDecoder::with_fallback(None)
    }

    #[test]
    fn utf8_names_pass_through_untouched() {
        let mut d = detector_only();
        assert_eq!(d.decode("中文/文件.txt".as_bytes()), "中文/文件.txt");
        assert_eq!(d.decode(b"plain.txt"), "plain.txt");
        assert_eq!(d.detected(), None, "no legacy encoding should be guessed");
    }

    #[test]
    fn detection_alone_handles_the_common_cjk_code_pages() {
        for (encoding, names) in [
            (encoding_rs::GBK, ["项目文档/说明.doc", "照片/春节合影.jpg"]),
            (encoding_rs::SHIFT_JIS, ["新しいフォルダ/資料.txt", "写真/家族旅行.jpg"]),
            (encoding_rs::BIG5, ["專案文件/說明.doc", "中文檔案.txt"]),
            (encoding_rs::EUC_KR, ["한글파일.txt", "사진/가족여행.jpg"]),
        ] {
            let raw: Vec<Vec<u8>> = names.iter().map(|n| enc(encoding, n)).collect();
            let mut d = detector_only();
            for r in &raw {
                d.sample(r);
            }
            for (r, expected) in raw.iter().zip(names) {
                assert_eq!(d.decode(r), expected, "{} misdecoded", encoding.name());
            }
        }
    }

    #[test]
    fn one_archive_locks_onto_one_encoding() {
        let names = ["项目文档/说明.doc", "照片/春节合影.jpg", "财务报表2024.xlsx"];
        let raw: Vec<Vec<u8>> = names.iter().map(|n| enc(encoding_rs::GBK, n)).collect();

        let mut d = detector_only();
        for r in &raw {
            d.sample(r);
        }
        for (r, expected) in raw.iter().zip(names) {
            assert_eq!(d.decode(r), expected);
        }
        assert_eq!(d.detected(), Some("GBK"));
    }

    #[test]
    fn mixed_utf8_and_legacy_names_both_survive() {
        let legacy = enc(encoding_rs::GBK, "旧的名字.txt");
        let mut d = detector_only();
        d.sample(&legacy);
        assert_eq!(d.decode("新しい.txt".as_bytes()), "新しい.txt");
        assert_eq!(d.decode(&legacy), "旧的名字.txt");
    }

    /// A single short name is too small a sample for the detector — this is
    /// exactly where the machine's own code page has to carry the decision.
    #[test]
    fn machine_code_page_rescues_a_sample_too_short_to_detect() {
        let raw = enc(encoding_rs::EUC_KR, "한글파일.txt");
        assert_ne!(
            detector_only().decode(&raw),
            "한글파일.txt",
            "sample got long enough to detect — pick a shorter one for this test"
        );

        let mut d = NameDecoder::with_fallback(Some(encoding_rs::EUC_KR));
        assert_eq!(d.decode(&raw), "한글파일.txt");
    }

    /// The fallback only applies when the bytes actually fit that code page, and
    /// never overrides a confident CJK detection.
    #[test]
    fn detection_wins_over_the_machine_code_page() {
        let raw = enc(encoding_rs::SHIFT_JIS, "新しいフォルダ/資料.txt");
        let mut d = NameDecoder::with_fallback(Some(encoding_rs::GBK));
        d.sample(&raw);
        assert_eq!(d.decode(&raw), "新しいフォルダ/資料.txt");
    }

    #[cfg(not(windows))]
    #[test]
    fn locales_map_to_code_pages() {
        assert_eq!(locale_encoding("zh_CN.UTF-8"), Some(encoding_rs::GBK));
        assert_eq!(locale_encoding("zh_TW.UTF-8"), Some(encoding_rs::BIG5));
        assert_eq!(locale_encoding("ja_JP.UTF-8"), Some(encoding_rs::SHIFT_JIS));
        assert_eq!(locale_encoding("ko_KR.UTF-8"), Some(encoding_rs::EUC_KR));
        assert_eq!(locale_encoding("en_US.UTF-8"), None);
        assert_eq!(locale_encoding(""), None);
    }
}
