//! Brackets and their lookalikes (M5c.2 item 3, review S-3): a closed table, so no
//! normalisation crate. A bracket is every character of Unicode 16's open and close punctuation
//! (Ps, Pe) apart from the two low-9 quotation marks, every punctuation or symbol named as a
//! bracket or parenthesis (pieces, ornaments, tags), and every character whose compatibility
//! form (NFKC) holds `(`, `)`, `[` or `]` (fullwidth, small, superscript, subscript, vertical,
//! parenthesised letters and numbers). ASCII `{` and `}` are not in it: they are no lookalike of
//! the output's structure. Generated once from Python 3.14's `unicodedata` (Unicode 16.0).

/// Inclusive code point ranges, sorted.
const BRACKETS: &[(u32, u32)] = &[
    (0x28, 0x29),
    (0x5B, 0x5B),
    (0x5D, 0x5D),
    (0xF3A, 0xF3D),
    (0x169B, 0x169C),
    (0x2045, 0x2046),
    (0x207D, 0x207E),
    (0x208D, 0x208E),
    (0x2308, 0x230B),
    (0x2329, 0x232A),
    (0x239B, 0x23AD),
    (0x23B0, 0x23B1),
    (0x23B4, 0x23B6),
    (0x23DC, 0x23E1),
    (0x2474, 0x2487),
    (0x249C, 0x24B5),
    (0x2768, 0x2775),
    (0x27C5, 0x27C6),
    (0x27E6, 0x27EF),
    (0x2983, 0x2998),
    (0x29D8, 0x29DB),
    (0x29FC, 0x29FD),
    (0x2E02, 0x2E05),
    (0x2E09, 0x2E0A),
    (0x2E0C, 0x2E0D),
    (0x2E1C, 0x2E1D),
    (0x2E22, 0x2E29),
    (0x2E42, 0x2E42),
    (0x2E55, 0x2E5C),
    (0x3008, 0x3011),
    (0x3014, 0x301B),
    (0x301D, 0x301F),
    (0x3200, 0x321E),
    (0x3220, 0x3243),
    (0xFD3E, 0xFD3F),
    (0xFE17, 0xFE18),
    (0xFE35, 0xFE44),
    (0xFE47, 0xFE48),
    (0xFE59, 0xFE5E),
    (0xFF08, 0xFF09),
    (0xFF3B, 0xFF3B),
    (0xFF3D, 0xFF3D),
    (0xFF5B, 0xFF5B),
    (0xFF5D, 0xFF5D),
    (0xFF5F, 0xFF60),
    (0xFF62, 0xFF63),
    (0x1D115, 0x1D115),
    (0x1D156, 0x1D156),
    (0x1DA8B, 0x1DA8B),
    (0x1F110, 0x1F12A),
    (0x1F240, 0x1F248),
    (0x1FBCC, 0x1FBCC),
    (0xE0028, 0xE0029),
    (0xE005B, 0xE005B),
    (0xE005D, 0xE005D),
    (0xE007B, 0xE007B),
    (0xE007D, 0xE007D),
];

/// Whether `c` is a bracket or a lookalike of one.
pub fn is_bracket(c: char) -> bool {
    let c = u32::from(c);
    BRACKETS
        .binary_search_by(|&(start, end)| {
            if end < c {
                std::cmp::Ordering::Less
            } else if start > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// A square bracket or a lookalike of one (NFKC `[` or `]`, or named a left or right square
/// bracket) as the parenthesis that replaces it: `(` for a left one, `)` for a right one.
pub fn square(c: char) -> Option<char> {
    match c {
        '\u{5B}' | '\u{2045}' | '\u{23A1}' | '\u{23A2}' | '\u{23A3}' | '\u{27E6}' | '\u{298B}'
        | '\u{298D}' | '\u{298F}' | '\u{2E55}' | '\u{2E57}' | '\u{301A}' | '\u{FE47}'
        | '\u{FF3B}' | '\u{1FBCC}' => Some('('),
        '\u{5D}' | '\u{2046}' | '\u{23A4}' | '\u{23A5}' | '\u{23A6}' | '\u{27E7}' | '\u{298C}'
        | '\u{298E}' | '\u{2990}' | '\u{2E56}' | '\u{2E58}' | '\u{301B}' | '\u{FE48}'
        | '\u{FF3D}' => Some(')'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookalikes_are_brackets() {
        for c in "()[]（）［］｛｝⁽⁾₍₎﹙﹚❨❩❪❫⟦⟧【】〔〕「」⑴⒜︵︶⸨⸩".chars()
        {
            assert!(is_bracket(c), "{c} U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn letters_and_braces_are_not() {
        for c in "aZ09 -_{}<>«»„‚\"'é🎩—→".chars() {
            assert!(!is_bracket(c), "{c} U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn square_ones_become_parentheses() {
        assert_eq!(square('['), Some('('));
        assert_eq!(square(']'), Some(')'));
        assert_eq!(square('［'), Some('('));
        assert_eq!(square('］'), Some(')'));
        assert_eq!(square('⟦'), Some('('));
        assert_eq!(square('〛'), Some(')'));
        assert_eq!(square('（'), None);
        assert_eq!(square('a'), None);
    }

    #[test]
    fn every_square_one_is_a_bracket() {
        for cp in 0..=0x10FFFF {
            if let Some(c) = char::from_u32(cp) {
                assert!(square(c).is_none() || is_bracket(c), "U+{cp:04X}");
            }
        }
    }
}
