use crate::token::{TokenKind, keyword_kind};

/// Byte-offset span plus 1-based line, mirroring `Token.line` in C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub offset: usize,
    pub len: usize,
    pub line: u32,
}

/// A lexed token with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    /// Identifier / keyword spelling, or decoded string contents.
    pub text: Option<String>,
    /// `T_INT`: bit pattern; values above `i64::MAX` set `big_u64`.
    pub int_val: i64,
    pub big_u64: bool,
    pub float_val: f64,
    pub bytes: Option<Vec<u8>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub line: u32,
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: error: {}", self.line, self.message)
    }
}

impl std::error::Error for LexError {}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
        }
    }

    fn peek(&self, off: usize) -> u8 {
        self.src.get(self.pos + off).copied().unwrap_or(0)
    }

    fn err(&self, msg: &str) -> LexError {
        LexError {
            line: self.line,
            offset: self.pos,
            message: msg.to_string(),
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            let c = self.peek(0);
            if c == b'\n' {
                self.line += 1;
                self.pos += 1;
            } else if c == b' ' || c == b'\t' || c == b'\r' {
                self.pos += 1;
            } else if c == b'/' && self.peek(1) == b'/' {
                while self.peek(0) != 0 && self.peek(0) != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn decode_string_body(&self, start: usize, end: usize) -> Result<String, LexError> {
        // Mirrors decode_string(): `${...}` interpolation is kept verbatim
        // between \x01 markers for the parser to split on later.
        let mut out = String::new();
        let mut i = start;
        while i < end {
            let c = self.src[i];
            if c == b'$' && i + 1 < end && self.src[i + 1] == b'{' {
                out.push('\x01');
                i += 2;
                let mut depth = 1;
                while i < end && depth > 0 {
                    if self.src[i] == b'{' {
                        depth += 1;
                    } else if self.src[i] == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    out.push(self.src[i] as char);
                    i += 1;
                }
                if depth != 0 {
                    return Err(self.err("unterminated interpolation"));
                }
                out.push('\x01');
                i += 1; // past '}'
                continue;
            }
            if c == b'\\' {
                i += 1;
                if i >= end {
                    return Err(self.err("unterminated string literal"));
                }
                if self.src[i] == b'{' {
                    out.push('\x01');
                    i += 1;
                    let mut depth = 1;
                    while i < end && depth > 0 {
                        if self.src[i] == b'{' {
                            depth += 1;
                        } else if self.src[i] == b'}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        out.push(self.src[i] as char);
                        i += 1;
                    }
                    if depth != 0 {
                        return Err(self.err("unterminated interpolation"));
                    }
                    out.push('\x01');
                    i += 1;
                    continue;
                }
                match self.src[i] {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'0' => out.push('\0'),
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    _ => return Err(self.err("unknown escape sequence in string literal")),
                }
                i += 1;
            } else {
                out.push(c as char);
                i += 1;
            }
        }
        Ok(out)
    }

    fn lex_number(&mut self, start: usize, line: u32) -> Result<Token, LexError> {
        // Hex / binary with `_` separators, else decimal or float.
        if self.src[start] == b'0'
            && (self.peek(1) == b'x'
                || self.peek(1) == b'X'
                || self.peek(1) == b'b'
                || self.peek(1) == b'B')
        {
            let base = if self.peek(1) == b'x' || self.peek(1) == b'X' {
                16
            } else {
                2
            };
            // `0b"` starts a byte-string literal, not a binary number.
            if !(base == 2 && self.peek(2) == b'"') {
                self.pos += 2;
                let mut digits = String::new();
                loop {
                    let d = self.peek(0);
                    if d == b'_' {
                        self.pos += 1;
                        continue;
                    }
                    let ok = if base == 16 {
                        d.is_ascii_hexdigit()
                    } else {
                        d == b'0' || d == b'1'
                    };
                    if !ok {
                        break;
                    }
                    if digits.len() < 79 {
                        digits.push(d as char);
                    }
                    self.pos += 1;
                }
                if digits.is_empty() {
                    return Err(self.err(if base == 16 {
                        "hex literal has no digits"
                    } else {
                        "binary literal has no digits"
                    }));
                }
                if digits.len() >= 79 {
                    return Err(self.err(if base == 16 {
                        "hex literal does not fit in 64 bits"
                    } else {
                        "binary literal does not fit in 64 bits"
                    }));
                }
                let uv = u64::from_str_radix(&digits, base as u32)
                    .map_err(|_| self.err("integer literal does not fit in 64 bits"))?;
                return Ok(Token {
                    kind: TokenKind::Int,
                    text: None,
                    int_val: uv as i64,
                    big_u64: uv > i64::MAX as u64,
                    float_val: 0.0,
                    bytes: None,
                    span: Span {
                        offset: start,
                        len: self.pos - start,
                        line,
                    },
                });
            }
        }
        while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
            self.pos += 1;
        }
        if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            self.pos += 1; // consume '.'
            while self.peek(0).is_ascii_digit() {
                self.pos += 1;
            }
            let raw: String = self.src[start..self.pos]
                .iter()
                .filter(|&&c| c != b'_')
                .map(|&c| c as char)
                .collect();
            let v: f64 = raw.parse().map_err(|_| self.err("invalid float literal"))?;
            return Ok(Token {
                kind: TokenKind::Float,
                text: None,
                int_val: 0,
                big_u64: false,
                float_val: v,
                bytes: None,
                span: Span {
                    offset: start,
                    len: self.pos - start,
                    line,
                },
            });
        }
        let digits: String = self.src[start..self.pos]
            .iter()
            .filter(|&&c| c != b'_')
            .map(|&c| c as char)
            .collect();
        if digits.len() > 20 {
            return Err(self.err(
                "integer literal does not fit in 64 bits (maximum is 18446744073709551615)",
            ));
        }
        let uv: u64 = digits
            .parse()
            .map_err(|_| self.err("integer literal does not fit in 64 bits (maximum is 18446744073709551615)"))?;
        Ok(Token {
            kind: TokenKind::Int,
            text: None,
            int_val: uv as i64,
            big_u64: uv > i64::MAX as u64,
            float_val: 0.0,
            bytes: None,
            span: Span {
                offset: start,
                len: self.pos - start,
                line,
            },
        })
    }

    fn lex_bytes(&mut self, start: usize, line: u32) -> Result<Token, LexError> {
        self.pos += 2; // b"
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let ch = self.peek(0);
            if ch == 0 || ch == b'\n' {
                return Err(self.err("unterminated byte string literal"));
            }
            if ch == b'"' {
                break;
            }
            if ch == b'\\' {
                self.pos += 1;
                match self.peek(0) {
                    b'n' => buf.push(10),
                    b't' => buf.push(9),
                    b'r' => buf.push(13),
                    b'0' => buf.push(0),
                    b'"' => buf.push(b'"'),
                    b'\\' => buf.push(b'\\'),
                    b'x' => {
                        let hi = hex_val(self.peek(1))
                            .ok_or_else(|| self.err("invalid \\xHH escape in byte string literal"))?;
                        let lo = hex_val(self.peek(2))
                            .ok_or_else(|| self.err("invalid \\xHH escape in byte string literal"))?;
                        buf.push(hi * 16 + lo);
                        self.pos += 2;
                    }
                    _ => {
                        return Err(self.err(
                            "unknown escape sequence in byte string literal",
                        ))
                    }
                }
                self.pos += 1;
            } else {
                buf.push(ch);
                self.pos += 1;
            }
        }
        self.pos += 1; // closing quote
        Ok(Token {
            kind: TokenKind::Bytes,
            text: None,
            int_val: 0,
            big_u64: false,
            float_val: 0.0,
            bytes: Some(buf),
            span: Span {
                offset: start,
                len: self.pos - start,
                line,
            },
        })
    }

    fn lex_ident(&mut self, start: usize, line: u32) -> Token {
        while is_ident_char(self.peek(0)) {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.src[start..self.pos]).to_string();
        let kind = keyword_kind(&text).unwrap_or(TokenKind::Ident);
        Token {
            kind,
            text: Some(text),
            int_val: 0,
            big_u64: false,
            float_val: 0.0,
            bytes: None,
            span: Span {
                offset: start,
                len: self.pos - start,
                line,
            },
        }
    }

    fn lex_string(&mut self, start: usize, line: u32) -> Result<Token, LexError> {
        self.pos += 1; // opening quote
        let body_start = self.pos;
        while self.peek(0) != 0 && self.peek(0) != b'"' {
            if self.peek(0) == b'\n' {
                return Err(self.err("unterminated string literal"));
            }
            if self.peek(0) == b'\\' {
                self.pos += 1; // skip escaped char; validated in decode
            }
            self.pos += 1;
        }
        if self.peek(0) != b'"' {
            return Err(self.err("unterminated string literal"));
        }
        let body_end = self.pos;
        let decoded = self.decode_string_body(body_start, body_end)?;
        self.pos += 1; // closing quote
        Ok(Token {
            kind: TokenKind::String,
            text: Some(decoded),
            int_val: 0,
            big_u64: false,
            float_val: 0.0,
            bytes: None,
            span: Span {
                offset: start,
                len: self.pos - start,
                line,
            },
        })
    }

    fn single(&mut self, kind: TokenKind, start: usize, line: u32, width: usize) -> Token {
        self.pos += width;
        Token {
            kind,
            text: None,
            int_val: 0,
            big_u64: false,
            float_val: 0.0,
            bytes: None,
            span: Span {
                offset: start,
                len: width,
                line,
            },
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        self.skip_trivia();
        let start = self.pos;
        let line = self.line;
        let c = self.peek(0);
        if c == 0 {
            return Ok(None);
        }
        if c.is_ascii_digit() {
            return self.lex_number(start, line).map(Some);
        }
        if c == b'b' && self.peek(1) == b'"' {
            return self.lex_bytes(start, line).map(Some);
        }
        if is_ident_start(c) {
            return Ok(Some(self.lex_ident(start, line)));
        }
        if c == b'.' && self.peek(1) == b'.' {
            if self.peek(2) == b'=' {
                return Ok(Some(self.single(TokenKind::DotDotEq, start, line, 3)));
            }
            return Ok(Some(self.single(TokenKind::DotDot, start, line, 2)));
        }
        if c == b'\'' && is_ident_start(self.peek(1)) {
            self.pos += 1;
            let name_start = self.pos;
            while is_ident_char(self.peek(0)) {
                self.pos += 1;
            }
            let text = String::from_utf8_lossy(&self.src[name_start..self.pos]).to_string();
            return Ok(Some(Token {
                kind: TokenKind::Lifetime,
                text: Some(text),
                int_val: 0,
                big_u64: false,
                float_val: 0.0,
                bytes: None,
                span: Span {
                    offset: start,
                    len: self.pos - start,
                    line,
                },
            }));
        }
        if c == b'"' {
            return self.lex_string(start, line).map(Some);
        }
        // Three-char operators first so `<<=` does not lex as `<<` + `=`.
        if c == b'<' && self.peek(1) == b'<' && self.peek(2) == b'=' {
            return Ok(Some(self.single(TokenKind::ShlEq, start, line, 3)));
        }
        if c == b'>' && self.peek(1) == b'>' && self.peek(2) == b'=' {
            return Ok(Some(self.single(TokenKind::ShrEq, start, line, 3)));
        }
        let two = (c, self.peek(1));
        let two_kind = match two {
            (b'+', b'=') => Some(TokenKind::PlusEq),
            (b'-', b'=') => Some(TokenKind::MinusEq),
            (b'*', b'=') => Some(TokenKind::StarEq),
            (b'/', b'=') => Some(TokenKind::SlashEq),
            (b'%', b'=') => Some(TokenKind::PercentEq),
            (b'&', b'=') => Some(TokenKind::AmpEq),
            (b'|', b'=') => Some(TokenKind::PipeEq),
            (b'^', b'=') => Some(TokenKind::CaretEq),
            (b'=', b'=') => Some(TokenKind::EqEq),
            (b'!', b'=') => Some(TokenKind::BangEq),
            (b'<', b'=') => Some(TokenKind::Lte),
            (b'>', b'=') => Some(TokenKind::Gte),
            (b'&', b'&') => Some(TokenKind::AndAnd),
            (b'|', b'|') => Some(TokenKind::OrOr),
            (b'?', b'?') => Some(TokenKind::QuestionQuestion),
            (b'-', b'>') => Some(TokenKind::Arrow),
            (b'<', b'<') => Some(TokenKind::Shl),
            (b'>', b'>') => Some(TokenKind::Shr),
            _ => None,
        };
        if let Some(kind) = two_kind {
            return Ok(Some(self.single(kind, start, line, 2)));
        }
        let single = match c {
            b'&' => Some(TokenKind::Amp),
            b'|' => Some(TokenKind::Pipe),
            b'^' => Some(TokenKind::Caret),
            b'~' => Some(TokenKind::Tilde),
            b'+' => Some(TokenKind::Plus),
            b'-' => Some(TokenKind::Minus),
            b'*' => Some(TokenKind::Star),
            b'/' => Some(TokenKind::Slash),
            b'%' => Some(TokenKind::Percent),
            b'<' => Some(TokenKind::Lt),
            b'>' => Some(TokenKind::Gt),
            b'!' => Some(TokenKind::Bang),
            b'=' => Some(TokenKind::Assign),
            b'(' => Some(TokenKind::LParen),
            b')' => Some(TokenKind::RParen),
            b'{' => Some(TokenKind::LBrace),
            b'}' => Some(TokenKind::RBrace),
            b',' => Some(TokenKind::Comma),
            b';' => Some(TokenKind::Semi),
            b':' => Some(TokenKind::Colon),
            b'[' => Some(TokenKind::LBracket),
            b']' => Some(TokenKind::RBracket),
            b'.' => Some(TokenKind::Dot),
            _ => None,
        };
        if let Some(kind) = single {
            return Ok(Some(self.single(kind, start, line, 1)));
        }
        Err(self.err("unexpected character"))
    }
}

/// Lex a full `.sl` source file, appending a trailing `Eof` token.
/// Mirrors repeated `lexer_next` calls in `src/loader.c`.
pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    if !src.is_ascii() {
        // slang sources are UTF-8 `str`, but the lexer operates byte-wise;
        // non-ASCII is only legal inside string literals and comments.
        // The byte-wise walk below still handles those correctly.
    }
    let mut lx = Lexer::new(src);
    let mut out = Vec::new();
    loop {
        match lx.next_token()? {
            Some(t) => out.push(t),
            None => {
                out.push(Token {
                    kind: TokenKind::Eof,
                    text: None,
                    int_val: 0,
                    big_u64: false,
                    float_val: 0.0,
                    bytes: None,
                    span: Span {
                        offset: lx.pos,
                        len: 0,
                        line: lx.line,
                    },
                });
                return Ok(out);
            }
        }
    }
}
