use crate::types::FontFamily;

pub const BUNDLED: &[(FontFamily, &[u8])] = &[
    (FontFamily::Excalifont, include_bytes!("../fonts/Excalifont.ttf")),
    (FontFamily::Nunito, include_bytes!("../fonts/Nunito.ttf")),
    (FontFamily::ComicShanns, include_bytes!("../fonts/ComicShanns.ttf")),
    (FontFamily::Lilita, include_bytes!("../fonts/Lilita.ttf")),
    (FontFamily::Assistant, include_bytes!("../fonts/Assistant.ttf")),
    (FontFamily::Virgil, include_bytes!("../fonts/Virgil.ttf")),
];

impl FontFamily {
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
