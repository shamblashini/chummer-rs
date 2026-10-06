//! Colours as Chummer saves them: `<notesColor>` is
//! `ColorTranslator.ToHtml` text (a known colour's name such as
//! "Chocolate", else `#RRGGBB`), and `<colour>` is `Color.ToArgb` as a
//! signed integer.

/// `GlobalSettings.DefaultHasNotesColor` as `ColorTranslator.ToHtml`
/// writes it.
pub const DEFAULT_NOTES_COLOR: &str = "Chocolate";

/// `SystemColors.Control` (`0xFFF0F0F0`): a contact's `<colour>` when it
/// was never set.
pub const DEFAULT_CONTACT_COLOUR: i32 = -986896;

/// The .NET Framework `KnownColor` web colours, by name.
const NAMED: &[(&str, u32)] = &[
    ("AliceBlue", 0xF0F8FF), ("AntiqueWhite", 0xFAEBD7), ("Aqua", 0x00FFFF), ("Aquamarine", 0x7FFFD4),
    ("Azure", 0xF0FFFF), ("Beige", 0xF5F5DC), ("Bisque", 0xFFE4C4), ("Black", 0x000000),
    ("BlanchedAlmond", 0xFFEBCD), ("Blue", 0x0000FF), ("BlueViolet", 0x8A2BE2), ("Brown", 0xA52A2A),
    ("BurlyWood", 0xDEB887), ("CadetBlue", 0x5F9EA0), ("Chartreuse", 0x7FFF00), ("Chocolate", 0xD2691E),
    ("Coral", 0xFF7F50), ("CornflowerBlue", 0x6495ED), ("Cornsilk", 0xFFF8DC), ("Crimson", 0xDC143C),
    ("Cyan", 0x00FFFF), ("DarkBlue", 0x00008B), ("DarkCyan", 0x008B8B), ("DarkGoldenrod", 0xB8860B),
    ("DarkGray", 0xA9A9A9), ("DarkGreen", 0x006400), ("DarkKhaki", 0xBDB76B), ("DarkMagenta", 0x8B008B),
    ("DarkOliveGreen", 0x556B2F), ("DarkOrange", 0xFF8C00), ("DarkOrchid", 0x9932CC), ("DarkRed", 0x8B0000),
    ("DarkSalmon", 0xE9967A), ("DarkSeaGreen", 0x8FBC8F), ("DarkSlateBlue", 0x483D8B), ("DarkSlateGray", 0x2F4F4F),
    ("DarkTurquoise", 0x00CED1), ("DarkViolet", 0x9400D3), ("DeepPink", 0xFF1493), ("DeepSkyBlue", 0x00BFFF),
    ("DimGray", 0x696969), ("DodgerBlue", 0x1E90FF), ("Firebrick", 0xB22222), ("FloralWhite", 0xFFFAF0),
    ("ForestGreen", 0x228B22), ("Fuchsia", 0xFF00FF), ("Gainsboro", 0xDCDCDC), ("GhostWhite", 0xF8F8FF),
    ("Gold", 0xFFD700), ("Goldenrod", 0xDAA520), ("Gray", 0x808080), ("Green", 0x008000),
    ("GreenYellow", 0xADFF2F), ("Honeydew", 0xF0FFF0), ("HotPink", 0xFF69B4), ("IndianRed", 0xCD5C5C),
    ("Indigo", 0x4B0082), ("Ivory", 0xFFFFF0), ("Khaki", 0xF0E68C), ("Lavender", 0xE6E6FA),
    ("LavenderBlush", 0xFFF0F5), ("LawnGreen", 0x7CFC00), ("LemonChiffon", 0xFFFACD), ("LightBlue", 0xADD8E6),
    ("LightCoral", 0xF08080), ("LightCyan", 0xE0FFFF), ("LightGoldenrodYellow", 0xFAFAD2), ("LightGray", 0xD3D3D3),
    ("LightGreen", 0x90EE90), ("LightPink", 0xFFB6C1), ("LightSalmon", 0xFFA07A), ("LightSeaGreen", 0x20B2AA),
    ("LightSkyBlue", 0x87CEFA), ("LightSlateGray", 0x778899), ("LightSteelBlue", 0xB0C4DE), ("LightYellow", 0xFFFFE0),
    ("Lime", 0x00FF00), ("LimeGreen", 0x32CD32), ("Linen", 0xFAF0E6), ("Magenta", 0xFF00FF),
    ("Maroon", 0x800000), ("MediumAquamarine", 0x66CDAA), ("MediumBlue", 0x0000CD), ("MediumOrchid", 0xBA55D3),
    ("MediumPurple", 0x9370DB), ("MediumSeaGreen", 0x3CB371), ("MediumSlateBlue", 0x7B68EE), ("MediumSpringGreen", 0x00FA9A),
    ("MediumTurquoise", 0x48D1CC), ("MediumVioletRed", 0xC71585), ("MidnightBlue", 0x191970), ("MintCream", 0xF5FFFA),
    ("MistyRose", 0xFFE4E1), ("Moccasin", 0xFFE4B5), ("NavajoWhite", 0xFFDEAD), ("Navy", 0x000080),
    ("OldLace", 0xFDF5E6), ("Olive", 0x808000), ("OliveDrab", 0x6B8E23), ("Orange", 0xFFA500),
    ("OrangeRed", 0xFF4500), ("Orchid", 0xDA70D6), ("PaleGoldenrod", 0xEEE8AA), ("PaleGreen", 0x98FB98),
    ("PaleTurquoise", 0xAFEEEE), ("PaleVioletRed", 0xDB7093), ("PapayaWhip", 0xFFEFD5), ("PeachPuff", 0xFFDAB9),
    ("Peru", 0xCD853F), ("Pink", 0xFFC0CB), ("Plum", 0xDDA0DD), ("PowderBlue", 0xB0E0E6),
    ("Purple", 0x800080), ("Red", 0xFF0000), ("RosyBrown", 0xBC8F8F),
    ("RoyalBlue", 0x4169E1), ("SaddleBrown", 0x8B4513), ("Salmon", 0xFA8072), ("SandyBrown", 0xF4A460),
    ("SeaGreen", 0x2E8B57), ("SeaShell", 0xFFF5EE), ("Sienna", 0xA0522D), ("Silver", 0xC0C0C0),
    ("SkyBlue", 0x87CEEB), ("SlateBlue", 0x6A5ACD), ("SlateGray", 0x708090), ("Snow", 0xFFFAFA),
    ("SpringGreen", 0x00FF7F), ("SteelBlue", 0x4682B4), ("Tan", 0xD2B48C), ("Teal", 0x008080),
    ("Thistle", 0xD8BFD8), ("Tomato", 0xFF6347), ("Turquoise", 0x40E0D0), ("Violet", 0xEE82EE),
    ("Wheat", 0xF5DEB3), ("White", 0xFFFFFF), ("WhiteSmoke", 0xF5F5F5), ("Yellow", 0xFFFF00),
    ("YellowGreen", 0x9ACD32),
];

/// `ColorTranslator.FromHtml` for the forms Chummer writes: `#RRGGBB`,
/// `#RGB` and colour names (any case; "LightGrey" is LightGray). `None`
/// for anything else (Chummer would throw).
pub fn parse(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return match hex.len() {
            6 => Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]),
            3 => {
                let d = |n: u32| ((n & 0xF) * 17) as u8;
                Some([d(v >> 8), d(v >> 4), d(v)])
            }
            _ => None,
        };
    }
    let name = if s.eq_ignore_ascii_case("LightGrey") { "LightGray" } else { s };
    NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, v)| [(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// `ColorTranslator.ToHtml` of a colour made from RGB values (what the
/// notes dialog's colour picker returns): `#RRGGBB`, upper-case. .NET only
/// writes a name for a colour that was itself made from a name.
pub fn to_html(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// A `<colour>` (`Color.ToArgb`) as ARGB bytes `[a, r, g, b]`.
pub fn from_argb(v: i32) -> [u8; 4] {
    (v as u32).to_be_bytes()
}

/// Whether a contact's `<colour>` is worth showing: set, and not the
/// default control colour (or `Color.Empty`, 0).
pub fn is_custom_contact_colour(v: i32) -> bool {
    v != DEFAULT_CONTACT_COLOUR && v != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_hex() {
        assert_eq!(parse("Chocolate"), Some([0xD2, 0x69, 0x1E]));
        assert_eq!(parse("chocolate"), Some([0xD2, 0x69, 0x1E]));
        assert_eq!(parse("LightGrey"), parse("LightGray"));
        assert_eq!(parse("#003FFF"), Some([0, 0x3F, 0xFF]));
        assert_eq!(parse("#0af"), Some([0, 0xAA, 0xFF]));
        assert_eq!(parse("Black"), Some([0, 0, 0]));
        assert_eq!(parse("#12345"), None);
        assert_eq!(parse("NotAColour"), None);
    }

    #[test]
    fn writes_like_color_translator() {
        assert_eq!(to_html([0xD2, 0x69, 0x1E]), "#D2691E");
        assert_eq!(to_html([0, 0x3f, 0xff]), "#003FFF");
        assert_eq!(parse(&to_html([1, 2, 3])), Some([1, 2, 3]));
    }

    #[test]
    fn contact_colour() {
        assert_eq!(from_argb(DEFAULT_CONTACT_COLOUR), [0xFF, 0xF0, 0xF0, 0xF0]);
        assert_eq!(from_argb(-65536), [0xFF, 0xFF, 0, 0]);
        assert!(!is_custom_contact_colour(DEFAULT_CONTACT_COLOUR));
        assert!(!is_custom_contact_colour(0));
        assert!(is_custom_contact_colour(-65536));
    }
}
