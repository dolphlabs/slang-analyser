//! Position math and semantic-token encoding (pure, unit-tested).

use slang_syntax::{Token, TokenKind};

/// Byte offsets where each line starts (line 0 starts at 0).
pub fn line_starts(src: &str) -> Vec<usize> {
    let mut out = vec![0];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            out.push(i + 1);
        }
    }
    out
}

/// Byte offset -> (line, UTF-16 character). Lines are 0-based for LSP.
pub fn offset_to_position(starts: &[usize], src: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(src.len());
    let line = starts.partition_point(|&s| s <= offset).saturating_sub(1);
    let character = src[starts[line]..offset].encode_utf16().count() as u32;
    (line as u32, character)
}

/// (line, UTF-16 character) -> byte offset, clamped into the file.
pub fn position_to_offset(starts: &[usize], src: &str, line: u32, character: u32) -> usize {
    let line = (line as usize).min(starts.len().saturating_sub(1));
    let base = starts[line];
    let end = if line + 1 < starts.len() {
        starts[line + 1].saturating_sub(1) // exclude '\n'
    } else {
        src.len()
    };
    let mut units = 0;
    for (i, c) in src[base..end].char_indices() {
        let w = c.encode_utf16(&mut [0; 2]).len() as u32;
        if units + w > character {
            return base + i;
        }
        units += w;
    }
    base + (end - base)
}

/// Semantic token legend index, in the server's legend order:
/// keyword, type, function, variable, string, number, comment,
/// operator, method, enumMember.
pub fn token_class(kind: TokenKind, text: Option<&str>, prev_is_dot: bool) -> Option<u32> {
    if kind.is_keyword() {
        return Some(0);
    }
    if kind.is_type_name() {
        return Some(1);
    }
    match kind {
        TokenKind::Int | TokenKind::Float => Some(5),
        TokenKind::String | TokenKind::Bytes => Some(4),
        TokenKind::Ident => {
            if prev_is_dot {
                return Some(8);
            }
            if text.map(|t| t.starts_with(|c: char| c.is_ascii_uppercase())).unwrap_or(false) {
                return Some(9);
            }
            Some(3)
        }
        TokenKind::Lifetime => Some(1),
        TokenKind::Eof => None,
        _ => Some(7),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemTok {
    pub line: u32,
    pub start: u32,
    pub length: u32,
    pub kind: u32,
}

/// Encode tokens + comments into delta-ordered semantic tokens.
pub fn encode_semantic(
    src: &str,
    toks: &[Token],
    comments: &[slang_syntax::Comment],
    is_call: &dyn Fn(usize) -> bool,
) -> Vec<SemTok> {
    let starts = line_starts(src);
    let mut out = Vec::new();
    let mut prev_is_dot = false;
    for (i, t) in toks.iter().enumerate() {
        let class = match t.kind {
            TokenKind::Ident if is_call(i) => Some(2),
            _ => token_class(t.kind, t.text.as_deref(), prev_is_dot),
        };
        prev_is_dot = t.kind == TokenKind::Dot;
        let Some(kind) = class else { continue };
        let (line, start) = offset_to_position(&starts, src, t.span.offset);
        let slice = &src[t.span.offset..(t.span.offset + t.span.len).min(src.len())];
        let length = slice.encode_utf16().count() as u32;
        if length == 0 {
            continue;
        }
        out.push(SemTok { line, start, length, kind });
    }
    for c in comments {
        let (line, start) = offset_to_position(&starts, src, c.offset);
        let end = src[c.offset..].find('\n').map(|i| c.offset + i).unwrap_or(src.len());
        let length = (end - c.offset) as u32;
        out.push(SemTok { line, start, length, kind: 6 });
    }
    out.sort_by_key(|t| (t.line, t.start));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_roundtrip_positions() {
        let src = "let n = 10;\nprintln(n);\n";
        let starts = line_starts(src);
        let off = src.find("(n)").unwrap() + 1;
        let (line, ch) = offset_to_position(&starts, src, off);
        assert_eq!((line, ch), (1, 8));
        assert_eq!(position_to_offset(&starts, src, line, ch), off);
    }

    #[test]
    fn should_classify_tokens() {
        assert_eq!(token_class(TokenKind::KwLet, Some("let"), false), Some(0));
        assert_eq!(token_class(TokenKind::TyInt, Some("int"), false), Some(1));
        assert_eq!(token_class(TokenKind::Int, None, false), Some(5));
        assert_eq!(token_class(TokenKind::Ident, Some("Paid"), false), Some(9));
        assert_eq!(token_class(TokenKind::Ident, Some("sum"), true), Some(8));
        assert_eq!(token_class(TokenKind::Eof, None, false), None);
    }
}
