/// Token kinds for `.sl`, mirroring `src/lexer.h` TokenType.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Eof,
    Ident,
    Int,
    Float,
    String,
    Bytes,
    // keywords
    KwLet,
    KwFn,
    KwIf,
    KwElse,
    KwWhile,
    KwReturn,
    KwTrue,
    KwFalse,
    KwPub,
    KwImport,
    KwGuard,
    KwFor,
    KwIn,
    KwAs,
    KwStruct,
    KwEnum,
    KwGc,
    KwOwn,
    KwMut,
    KwImpl,
    KwExtern,
    KwLink,
    KwSpawn,
    KwBreak,
    KwContinue,
    KwUnsafe,
    KwSelect,
    KwSwitch,
    KwCase,
    KwDefault,
    // type names
    TyInt,
    TyFloat,
    TyStr,
    TyBool,
    TyBytes,
    TyI8,
    TyI16,
    TyI32,
    TyI64,
    TyU8,
    TyU16,
    TyU32,
    TyU64,
    TyF32,
    TyMap,
    TyOpt,
    TyResult,
    TyDuration,
    TyRawptr,
    TyArena,
    TyWire,
    TyUntil,
    TyFault,
    TyPeer,
    TyTrip,
    TyLink,
    TyChan,
    TyJoin,
    // operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    BangEq,
    Lt,
    Gt,
    Lte,
    Gte,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
    AndAnd,
    OrOr,
    QuestionQuestion,
    Bang,
    Assign,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Colon,
    LBracket,
    RBracket,
    Dot,
    DotDot,
    DotDotEq,
    Arrow,
    Lifetime,
}

impl TokenKind {
    /// Human name, mirroring `token_type_name` in `src/lexer.c`.
    pub fn name(self) -> &'static str {
        match self {
            TokenKind::Eof => "end of file",
            TokenKind::Ident => "identifier",
            TokenKind::Int => "integer",
            TokenKind::Float => "float",
            TokenKind::String => "string",
            TokenKind::Bytes => "byte string",
            TokenKind::KwLet => "'let'",
            TokenKind::KwFn => "'fn'",
            TokenKind::KwIf => "'if'",
            TokenKind::KwElse => "'else'",
            TokenKind::KwWhile => "'while'",
            TokenKind::KwReturn => "'return'",
            TokenKind::KwTrue => "'true'",
            TokenKind::KwFalse => "'false'",
            TokenKind::KwPub => "'pub'",
            TokenKind::KwImport => "'import'",
            TokenKind::KwGuard => "'guard'",
            TokenKind::KwFor => "'for'",
            TokenKind::KwIn => "'in'",
            TokenKind::KwAs => "'as'",
            TokenKind::KwStruct => "'struct'",
            TokenKind::KwEnum => "'enum'",
            TokenKind::KwGc => "'gc'",
            TokenKind::KwOwn => "'own'",
            TokenKind::KwMut => "'mut'",
            TokenKind::KwImpl => "'impl'",
            TokenKind::KwExtern => "'extern'",
            TokenKind::KwLink => "'link'",
            TokenKind::KwSpawn => "'spawn'",
            TokenKind::KwBreak => "'break'",
            TokenKind::KwContinue => "'continue'",
            TokenKind::KwUnsafe => "'unsafe'",
            TokenKind::KwSelect => "'select'",
            TokenKind::KwSwitch => "'switch'",
            TokenKind::KwCase => "'case'",
            TokenKind::KwDefault => "'default'",
            TokenKind::TyInt => "'int'",
            TokenKind::TyFloat => "'float'",
            TokenKind::TyStr => "'str'",
            TokenKind::TyBool => "'bool'",
            TokenKind::TyBytes => "'bytes'",
            TokenKind::TyI8 => "'i8'",
            TokenKind::TyI16 => "'i16'",
            TokenKind::TyI32 => "'i32'",
            TokenKind::TyI64 => "'i64'",
            TokenKind::TyU8 => "'u8'",
            TokenKind::TyU16 => "'u16'",
            TokenKind::TyU32 => "'u32'",
            TokenKind::TyU64 => "'u64'",
            TokenKind::TyF32 => "'f32'",
            TokenKind::TyMap => "'map'",
            TokenKind::TyOpt => "'opt'",
            TokenKind::TyResult => "'result'",
            TokenKind::TyDuration => "'duration'",
            TokenKind::TyRawptr => "'rawptr'",
            TokenKind::TyArena => "'arena'",
            TokenKind::TyWire => "'wire'",
            TokenKind::TyUntil => "'until'",
            TokenKind::TyFault => "'fault'",
            TokenKind::TyPeer => "'peer'",
            TokenKind::TyTrip => "'trip'",
            TokenKind::TyLink => "'link'",
            TokenKind::TyChan => "'chan'",
            TokenKind::TyJoin => "'join'",
            TokenKind::Plus => "'+'",
            TokenKind::Minus => "'-'",
            TokenKind::Star => "'*'",
            TokenKind::Slash => "'/'",
            TokenKind::Percent => "'%'",
            TokenKind::EqEq => "'=='",
            TokenKind::BangEq => "'!='",
            TokenKind::Lt => "'<'",
            TokenKind::Gt => "'>'",
            TokenKind::Lte => "'<='",
            TokenKind::Gte => "'>='",
            TokenKind::Amp => "'&'",
            TokenKind::Pipe => "'|'",
            TokenKind::Caret => "'^'",
            TokenKind::Tilde => "'~'",
            TokenKind::Shl => "'<<'",
            TokenKind::Shr => "'>>'",
            TokenKind::PlusEq => "'+='",
            TokenKind::MinusEq => "'-='",
            TokenKind::StarEq => "'*='",
            TokenKind::SlashEq => "'/='",
            TokenKind::PercentEq => "'%='",
            TokenKind::AmpEq => "'&='",
            TokenKind::PipeEq => "'|='",
            TokenKind::CaretEq => "'^='",
            TokenKind::ShlEq => "'<<='",
            TokenKind::ShrEq => "'>>='",
            TokenKind::AndAnd => "'&&'",
            TokenKind::OrOr => "'||'",
            TokenKind::QuestionQuestion => "'??'",
            TokenKind::Bang => "'!'",
            TokenKind::Assign => "'='",
            TokenKind::LParen => "'('",
            TokenKind::RParen => "')'",
            TokenKind::LBrace => "'{'",
            TokenKind::RBrace => "'}'",
            TokenKind::Comma => "','",
            TokenKind::Semi => "';'",
            TokenKind::Colon => "':'",
            TokenKind::LBracket => "'['",
            TokenKind::RBracket => "']'",
            TokenKind::Dot => "'.'",
            TokenKind::DotDot => "'..'",
            TokenKind::DotDotEq => "'..='",
            TokenKind::Arrow => "'->'",
            TokenKind::Lifetime => "lifetime",
        }
    }

    pub fn is_keyword(self) -> bool {
        matches!(
            self,
            TokenKind::KwLet
                | TokenKind::KwFn
                | TokenKind::KwIf
                | TokenKind::KwElse
                | TokenKind::KwWhile
                | TokenKind::KwReturn
                | TokenKind::KwTrue
                | TokenKind::KwFalse
                | TokenKind::KwPub
                | TokenKind::KwImport
                | TokenKind::KwGuard
                | TokenKind::KwFor
                | TokenKind::KwIn
                | TokenKind::KwAs
                | TokenKind::KwStruct
                | TokenKind::KwEnum
                | TokenKind::KwGc
                | TokenKind::KwOwn
                | TokenKind::KwMut
                | TokenKind::KwImpl
                | TokenKind::KwExtern
                | TokenKind::KwLink
                | TokenKind::KwSpawn
                | TokenKind::KwBreak
                | TokenKind::KwContinue
                | TokenKind::KwUnsafe
                | TokenKind::KwSelect
                | TokenKind::KwSwitch
                | TokenKind::KwCase
                | TokenKind::KwDefault
        )
    }

    pub fn is_type_name(self) -> bool {
        matches!(
            self,
            TokenKind::TyInt
                | TokenKind::TyFloat
                | TokenKind::TyStr
                | TokenKind::TyBool
                | TokenKind::TyBytes
                | TokenKind::TyI8
                | TokenKind::TyI16
                | TokenKind::TyI32
                | TokenKind::TyI64
                | TokenKind::TyU8
                | TokenKind::TyU16
                | TokenKind::TyU32
                | TokenKind::TyU64
                | TokenKind::TyF32
                | TokenKind::TyMap
                | TokenKind::TyOpt
                | TokenKind::TyResult
                | TokenKind::TyDuration
                | TokenKind::TyRawptr
                | TokenKind::TyArena
                | TokenKind::TyWire
                | TokenKind::TyUntil
                | TokenKind::TyFault
                | TokenKind::TyPeer
                | TokenKind::TyTrip
                | TokenKind::TyLink
                | TokenKind::TyChan
                | TokenKind::TyJoin
        )
    }
}

/// Map identifier text to a keyword / type-name kind, if any.
/// Mirrors the `KW(...)` table in `src/lexer.c`, plus the extra
/// `TokenType` names (`wire`, `until`, `fault`, `peer`, `trip`, `link`)
/// that `token_type_name` knows about.
pub fn keyword_kind(text: &str) -> Option<TokenKind> {
    Some(match text {
        "let" => TokenKind::KwLet,
        "fn" => TokenKind::KwFn,
        "if" => TokenKind::KwIf,
        "else" => TokenKind::KwElse,
        "while" => TokenKind::KwWhile,
        "return" => TokenKind::KwReturn,
        "true" => TokenKind::KwTrue,
        "false" => TokenKind::KwFalse,
        "pub" => TokenKind::KwPub,
        "import" => TokenKind::KwImport,
        "guard" => TokenKind::KwGuard,
        "for" => TokenKind::KwFor,
        "in" => TokenKind::KwIn,
        "as" => TokenKind::KwAs,
        "struct" => TokenKind::KwStruct,
        "enum" => TokenKind::KwEnum,
        "gc" => TokenKind::KwGc,
        "own" => TokenKind::KwOwn,
        "mut" => TokenKind::KwMut,
        "impl" => TokenKind::KwImpl,
        "extern" => TokenKind::KwExtern,
        "link" => TokenKind::KwLink,
        "spawn" => TokenKind::KwSpawn,
        "break" => TokenKind::KwBreak,
        "continue" => TokenKind::KwContinue,
        "unsafe" => TokenKind::KwUnsafe,
        "select" => TokenKind::KwSelect,
        "switch" => TokenKind::KwSwitch,
        "case" => TokenKind::KwCase,
        "default" => TokenKind::KwDefault,
        "int" => TokenKind::TyInt,
        "float" => TokenKind::TyFloat,
        "str" => TokenKind::TyStr,
        "bool" => TokenKind::TyBool,
        "bytes" => TokenKind::TyBytes,
        "i8" => TokenKind::TyI8,
        "i16" => TokenKind::TyI16,
        "i32" => TokenKind::TyI32,
        "i64" => TokenKind::TyI64,
        "u8" => TokenKind::TyU8,
        "u16" => TokenKind::TyU16,
        "u32" => TokenKind::TyU32,
        "u64" => TokenKind::TyU64,
        "f32" => TokenKind::TyF32,
        "map" => TokenKind::TyMap,
        "opt" => TokenKind::TyOpt,
        "result" => TokenKind::TyResult,
        "duration" => TokenKind::TyDuration,
        "rawptr" => TokenKind::TyRawptr,
        "arena" => TokenKind::TyArena,
        "wire" => TokenKind::TyWire,
        "until" => TokenKind::TyUntil,
        "fault" => TokenKind::TyFault,
        "peer" => TokenKind::TyPeer,
        "trip" => TokenKind::TyTrip,
        "chan" => TokenKind::TyChan,
        "join" => TokenKind::TyJoin,
        // `link` is both a directive keyword and a type name depending on
        // position; the lexer emits KwLink and the parser disambiguates.
        _ => return None,
    })
}
