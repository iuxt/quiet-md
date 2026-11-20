use crate::types::FontFamily;

pub const BUNDLED: &[(FontFamily, &[u8])] = &[
    (FontFamily::Excalifont, include_bytes!("../fonts/Excalifont.ttf")),
    (FontFamily::Nunito, include_bytes!("../fonts/Nunito.ttf")),
    (FontFamily::ComicShanns, include_bytes!("../fonts/ComicShanns.ttf")),
    (FontFamily::Lilita, include_bytes!("../fonts/Lilita.ttf")),
    (FontFamily::Assistant, include_bytes!("../fonts/Assistant.ttf")),
    (FontFamily::Virgil, include_bytes!("../fonts/Virgil.ttf")),
];

/// Keep a variation selector or combining mark with its base glyph when switching faces.
pub fn han_runs(text: &str) -> impl Iterator<Item = (&str, bool)> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut clusters = text.grapheme_indices(true).map(|(i, cluster)| (i, matches!(cluster.chars().next().unwrap() as u32,
        0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF |
        0xF900..=0xFAFF | 0xFF01..=0xFF60 | 0x20000..=0x2FA1F | 0x30000..=0x3347F))).peekable();
    std::iter::from_fn(move || {
        let (start, han) = clusters.next()?;
        while clusters.next_if(|&(_, next)| next == han).is_some() {}
        Some((&text[start..clusters.peek().map_or(text.len(), |&(i, _)| i)], han))
    })
}

impl FontFamily {
    pub fn han_families(self) -> &'static [&'static str] {
        match self {
            Self::Excalifont => &["Hannotate SC", "Kaiti SC", "KaiTi", "LXGW WenKai", "Songti SC", "Noto Serif CJK SC"],
            Self::Virgil => &["Kaiti SC", "KaiTi", "LXGW WenKai", "Hannotate SC", "Songti SC", "Noto Serif CJK SC"],
            Self::Xiaolai => &["Xiaolai SC", "Hannotate SC", "KaiTi", "LXGW WenKai"],
            Self::ComicShanns | Self::Cascadia => &["STFangsong", "FangSong", "LXGW WenKai Mono", "Noto Sans Mono CJK SC", "Songti SC", "SimSun"],
            Self::Lilita => &["Weibei SC", "Songti SC", "SimSun", "Noto Serif CJK SC"],
            Self::Assistant => &["Heiti SC", "SimHei", "Noto Sans CJK SC"],
            _ => &["PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", "Hiragino Sans GB"],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ComicShanns => "Comic Shanns Regular",
            _ => self.label(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Virgil => "Virgil",
            Self::Helvetica => "Helvetica",
            Self::Cascadia => "Cascadia Code",
            Self::Assistant => "Assistant",
            Self::Excalifont => "Excalifont",
            Self::ComicShanns => "Comic Shanns",
            Self::Liberation => "Liberation Sans",
            Self::Nunito => "Nunito",
            Self::Lilita => "Lilita One",
            Self::Xiaolai => "Xiaolai SC",
        }
    }
}
