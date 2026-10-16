use crate::ast::*;
use crate::lexer::{Span, Token};
use crate::token::TokenKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: error: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

fn span_between(start: Span, end: Span) -> Span {
    let s = start.offset.min(end.offset);
    let e = (start.offset + start.len).max(end.offset + end.len);
    Span {
        offset: s,
        len: e.saturating_sub(s),
        line: start.line,
    }
}

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    pub errors: Vec<ParseError>,
    /// Inside `[...]` the interior is index/slice syntax, not a general
    /// expression: a bare `..` there belongs to the slice (`xs[4..]`),
    /// so the binary-range arm stays off while this is set.
    no_range: bool,
}

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Self {
            toks,
            pos: 0,
            errors: Vec::new(),
            no_range: false,
        }
    }

    fn peek(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek_kind(&self) -> TokenKind {
        self.peek().kind
    }

    fn peek2_kind(&self) -> TokenKind {
        self.toks
            .get(self.pos + 1)
            .map(|t| t.kind)
            .unwrap_or(TokenKind::Eof)
    }

    fn at(&self, k: TokenKind) -> bool {
        self.peek_kind() == k
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos.min(self.toks.len() - 1)].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err_here(&mut self, msg: &str) {
        let t = self.peek().clone();
        self.errors.push(ParseError {
            line: t.span.line,
            offset: t.span.offset,
            message: msg.to_string(),
        });
    }

    fn expect(&mut self, k: TokenKind, what: &str) -> Option<()> {
        if self.at(k) {
            self.bump();
            Some(())
        } else {
            let got = self.peek().kind.name();
            let msg = format!("expected {what}, got {got}");
            self.err_here(&msg);
            None
        }
    }

    fn expect_ident(&mut self) -> Option<String> {
        if self.at(TokenKind::Ident) {
            return self.bump().text;
        }
        // Member names may be keywords (`x.map`): accept any keyword/type
        // spelling here, mirroring the C parser's recovery of keyword text.
        let t = self.peek().clone();
        if t.kind.is_keyword() || t.kind.is_type_name() {
            self.bump();
            return t.text;
        }
        self.err_here("expected identifier");
        None
    }

    fn sync_stmt(&mut self) {
        while !matches!(
            self.peek_kind(),
            TokenKind::Eof | TokenKind::Semi | TokenKind::RBrace
        ) {
            self.bump();
        }
        if self.at(TokenKind::Semi) {
            self.bump();
        }
    }

    pub fn parse_file(mut self) -> (File, Vec<ParseError>) {
        let mut items = Vec::new();
        let mut body = Vec::new();
        let start = self.peek().span;
        let mut end = start;
        while !self.at(TokenKind::Eof) {
            end = self.peek().span;
            let checkpoint = self.pos;
            match self.parse_top_level() {
                Some(Top::Item(i)) => items.push(i),
                Some(Top::Stmt(s)) => body.push(s),
                None => {
                    if self.pos == checkpoint {
                        self.bump();
                    } else {
                        self.sync_stmt();
                    }
                }
            }
        }
        let file_span = span_between(start, end);
        (
            File {
                items,
                body,
                span: file_span,
            },
            self.errors,
        )
    }

    fn parse_top_level(&mut self) -> Option<Top> {
        match self.peek_kind() {
            TokenKind::KwImport => Some(Top::Item(self.parse_import())),
            TokenKind::KwLink => Some(Top::Item(self.parse_link())),
            TokenKind::KwPub
            | TokenKind::KwFn
            | TokenKind::KwStruct
            | TokenKind::KwEnum
            | TokenKind::KwImpl
            | TokenKind::KwExtern
            | TokenKind::KwGc => Some(Top::Item(self.parse_item()?)),
            TokenKind::KwLet => {
                // Top-level `let` is a package global / main local.
                let s = self.bump().span; // let
                match self.parse_let_rest(s, false)? {
                    Stmt::Let {
                        name,
                        ty,
                        init,
                        span,
                    } => Some(Top::Item(Item::Let {
                        pub_: false,
                        name,
                        ty,
                        init,
                        span,
                    })),
                    other => Some(Top::Stmt(other)),
                }
            }
            _ => {
                // Other top-level statements belong to the main body.
                let stmt = self.parse_stmt()?;
                // Struct/enum/impl/fn nested in body are still items.
                match stmt {
                    Stmt::Expr { .. }
                    | Stmt::Let { .. }
                    | Stmt::Assign { .. }
                    | Stmt::If { .. }
                    | Stmt::While { .. }
                    | Stmt::ForRange { .. }
                    | Stmt::ForIn { .. }
                    | Stmt::Return { .. }
                    | Stmt::Break { .. }
                    | Stmt::Continue { .. }
                    | Stmt::GuardLet { .. }
                    | Stmt::Spawn { .. }
                    | Stmt::IfLet { .. }
                    | Stmt::Select { .. }
                    | Stmt::Switch { .. }
                    | Stmt::Unsafe { .. } => Some(Top::Stmt(stmt)),
                }
            }
        }
    }

    fn parse_import(&mut self) -> Item {
        let start = self.bump().span; // import
        let mut path = String::new();
        if self.at(TokenKind::String) {
            path = self.bump().text.unwrap_or_default();
            // Decoded strings keep interpolation markers; imports never
            // interpolate, so strip them back to raw path text.
            path = path.replace('\x01', "");
        } else {
            self.err_here("expected \"path\" after import");
        }
        let mut alias = None;
        if self.at(TokenKind::KwAs) {
            self.bump();
            alias = self.expect_ident();
        }
        let end = if self.at(TokenKind::Semi) {
            self.bump().span
        } else {
            self.peek().span
        };
        Item::Import {
            path,
            alias,
            span: span_between(start, end),
        }
    }

    fn parse_link(&mut self) -> Item {
        let start = self.bump().span; // link
        let mut lib = String::new();
        if self.at(TokenKind::String) {
            lib = self.bump().text.unwrap_or_default().replace('\x01', "");
        } else {
            self.err_here("expected \"lib\" after link");
        }
        let end = if self.at(TokenKind::Semi) {
            self.bump().span
        } else {
            self.peek().span
        };
        Item::Link {
            lib,
            span: span_between(start, end),
        }
    }

    fn parse_item(&mut self) -> Option<Item> {
        let mut pub_ = false;
        if self.at(TokenKind::KwPub) {
            self.bump();
            pub_ = true;
        }
        match self.peek_kind() {
            TokenKind::KwFn => self.parse_fn(pub_, false).map(Item::Fn),
            TokenKind::KwExtern => {
                self.bump();
                // Peek, don't consume: parse_fn bumps `fn` itself, and an
                // expect here would eat the function's name instead.
                if !self.at(TokenKind::KwFn) {
                    self.err_here("expected 'fn' after 'extern'");
                    return None;
                }
                self.parse_fn(pub_, true).map(Item::Fn)
            }
            TokenKind::KwStruct => self.parse_struct(pub_, false).map(Item::Struct),
            TokenKind::KwGc => {
                self.bump();
                // Same as above: parse_struct bumps `struct` itself.
                if !self.at(TokenKind::KwStruct) {
                    self.err_here("expected 'struct' after 'gc'");
                    return None;
                }
                self.parse_struct(pub_, true).map(Item::Struct)
            }
            TokenKind::KwEnum => self.parse_enum(pub_).map(Item::Enum),
            TokenKind::KwImpl => self.parse_impl().map(Item::Impl),
            TokenKind::KwLet => {
                let s = self.bump().span;
                match self.parse_let_rest(s, pub_)? {
                    Stmt::Let {
                        name,
                        ty,
                        init,
                        span,
                    } => Some(Item::Let {
                        pub_,
                        name,
                        ty,
                        init,
                        span,
                    }),
                    _ => None,
                }
            }
            _ => {
                self.err_here("expected fn, struct, enum, impl, or let");
                None
            }
        }
    }

    fn parse_tparams(&mut self) -> Vec<String> {
        // `[T, U]` immediately after a name with no space requirement;
        // only called where generic params are legal.
        if !self.at(TokenKind::LBracket) {
            return Vec::new();
        }
        // Heuristic: `fn f[T]` vs array type — caller decides; here we
        // optimistically parse `[Ident (, Ident)*]`.
        let save = self.pos;
        self.bump();
        let mut out = Vec::new();
        loop {
            match self.expect_ident() {
                Some(n) => out.push(n),
                None => {
                    self.pos = save;
                    return Vec::new();
                }
            }
            if self.at(TokenKind::Comma) {
                self.bump();
                continue;
            }
            break;
        }
        if !self.at(TokenKind::RBracket) {
            self.pos = save;
            return Vec::new();
        }
        self.bump();
        out
    }

    fn parse_fn(&mut self, pub_: bool, extern_: bool) -> Option<FnDecl> {
        let start = self.bump().span; // fn
        let name = self.expect_ident()?;
        let tparams = self.parse_tparams();
        self.expect(TokenKind::LParen, "'('");
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            let pstart = self.peek().span;
            let pname = match self.expect_ident() {
                Some(n) => n,
                None => {
                    self.sync_stmt();
                    break;
                }
            };
            self.expect(TokenKind::Colon, "':'");
            let ty = self.parse_type()?;
            let pend = ty.span;
            params.push(Param {
                name: pname,
                ty,
                span: span_between(pstart, pend),
            });
            if self.at(TokenKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokenKind::RParen, "')'");
        let mut ret = None;
        if self.at(TokenKind::Arrow) {
            self.bump();
            ret = Some(self.parse_type()?);
        }
        if extern_ {
            let end = if self.at(TokenKind::Semi) {
                self.bump().span
            } else {
                self.expect(TokenKind::Semi, "';'");
                self.peek().span
            };
            return Some(FnDecl {
                pub_,
                extern_: true,
                name,
                tparams,
                params,
                ret,
                body: None,
                span: span_between(start, end),
            });
        }
        let body = self.parse_block()?;
        let span = span_between(start, body.span);
        Some(FnDecl {
            pub_,
            extern_: false,
            name,
            tparams,
            params,
            ret,
            body: Some(body),
            span,
        })
    }

    fn parse_struct(&mut self, pub_: bool, gc: bool) -> Option<StructDecl> {
        let start = self.peek().span;
        self.bump(); // struct
        let name = self.expect_ident()?;
        let tparams = self.parse_tparams();
        self.expect(TokenKind::LBrace, "'{'");
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let fstart = self.peek().span;
            let fname = match self.expect_ident() {
                Some(n) => n,
                None => {
                    self.sync_stmt();
                    continue;
                }
            };
            self.expect(TokenKind::Colon, "':'");
            let ty = match self.parse_type() {
                Some(t) => t,
                None => {
                    self.sync_stmt();
                    continue;
                }
            };
            let fend = ty.span;
            fields.push(Field {
                name: fname,
                ty,
                span: span_between(fstart, fend),
            });
            if self.at(TokenKind::Comma) {
                self.bump();
            }
        }
        let end = self.bump().span; // }
        Some(StructDecl {
            pub_,
            gc,
            name,
            tparams,
            fields,
            span: span_between(start, end),
        })
    }

    fn parse_enum(&mut self, pub_: bool) -> Option<EnumDecl> {
        let start = self.bump().span; // enum
        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace, "'{'");
        let mut variants = Vec::new();
        let mut next_val: i64 = 0;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let vstart = self.peek().span;
            let vname = match self.expect_ident() {
                Some(n) => n,
                None => {
                    self.sync_stmt();
                    continue;
                }
            };
            let mut value = None;
            if self.at(TokenKind::Assign) {
                self.bump();
                let neg = if self.at(TokenKind::Minus) {
                    self.bump();
                    true
                } else {
                    false
                };
                if self.at(TokenKind::Int) {
                    let t = self.bump();
                    let v = if neg { -t.int_val } else { t.int_val };
                    value = Some(v);
                    next_val = v + 1;
                } else {
                    self.err_here("expected integer enum value");
                }
            } else {
                value = Some(next_val);
                next_val += 1;
            }
            // Keep explicit-None only when `= N` was written; default
            // numbering still records the resolved ordinal.
            let vend = self.peek().span;
            variants.push(EnumVariant {
                name: vname,
                value,
                span: span_between(vstart, vend),
            });
            if self.at(TokenKind::Comma) {
                self.bump();
            }
        }
        let end = self.bump().span;
        Some(EnumDecl {
            pub_,
            name,
            variants,
            span: span_between(start, end),
        })
    }

    fn parse_impl(&mut self) -> Option<ImplDecl> {
        let start = self.bump().span; // impl
        let type_name = self.expect_ident()?;
        let tparams = self.parse_tparams();
        self.expect(TokenKind::LBrace, "'{'");
        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let mut pub_ = false;
            if self.at(TokenKind::KwPub) {
                self.bump();
                pub_ = true;
            }
            if !self.at(TokenKind::KwFn) {
                self.err_here("expected fn in impl block");
                self.sync_stmt();
                continue;
            }
            if let Some(f) = self.parse_fn(pub_, false) {
                methods.push(f);
            } else {
                self.sync_stmt();
            }
        }
        let end = self.bump().span;
        Some(ImplDecl {
            type_name,
            tparams,
            methods,
            span: span_between(start, end),
        })
    }

    // ---- types ----

    fn parse_type(&mut self) -> Option<Type> {
        // Prefix: `&T`, `&mut T`, `own T`, `gc T`, `*T`, `*mut T`.
        if self.at(TokenKind::Amp) {
            let start = self.bump().span;
            let mut_ = if self.at(TokenKind::KwMut) {
                self.bump();
                true
            } else {
                false
            };
            let inner = self.parse_type()?;
            let span = span_between(start, inner.span);
            return Some(Type {
                span,
                kind: if mut_ {
                    TypeKind::RefMut(Box::new(inner))
                } else {
                    TypeKind::Ref(Box::new(inner))
                },
            });
        }
        if self.at(TokenKind::KwOwn) || self.at(TokenKind::KwGc) {
            let gc = self.peek_kind() == TokenKind::KwGc;
            let start = self.bump().span;
            let inner = self.parse_type()?;
            let span = span_between(start, inner.span);
            return Some(Type {
                span,
                kind: if gc {
                    TypeKind::Gc(Box::new(inner))
                } else {
                    TypeKind::Own(Box::new(inner))
                },
            });
        }
        if self.at(TokenKind::Star) {
            let start = self.bump().span;
            let inner = self.parse_type()?;
            let span = span_between(start, inner.span);
            return Some(Type {
                span,
                kind: TypeKind::Ptr(Box::new(inner)),
            });
        }
        // `[T]` array type.
        if self.at(TokenKind::LBracket) {
            let start = self.bump().span;
            let inner = self.parse_type()?;
            self.expect(TokenKind::RBracket, "']'")?;
            let end = self.peek().span;
            return Some(Type {
                kind: TypeKind::Array(Box::new(inner)),
                span: span_between(start, end),
            });
        }
        // `fn(A, B) -> R` type.
        if self.at(TokenKind::KwFn) {
            let start = self.bump().span;
            self.expect(TokenKind::LParen, "'('");
            let mut params = Vec::new();
            while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
                params.push(self.parse_type()?);
                if self.at(TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
            self.expect(TokenKind::RParen, "')'");
            let mut ret = None;
            if self.at(TokenKind::Arrow) {
                self.bump();
                ret = Some(Box::new(self.parse_type()?));
            }
            let end = ret.as_ref().map(|t| t.span).unwrap_or(start);
            return Some(Type {
                kind: TypeKind::Fn { params, ret },
                span: span_between(start, end),
            });
        }
        // Named / generic: `int`, `pkg.Point`, `Box[int]`, `map[K]V`, ...
        let start = self.peek().span;
        let base = self.type_base_name()?;
        // Generic arguments `Name[A, B]`.
        if self.at(TokenKind::LBracket) {
            self.bump();
            let mut args = Vec::new();
            while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
                args.push(self.parse_type()?);
                if self.at(TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
            self.expect(TokenKind::RBracket, "']'");
            let end = self.peek().span;
            let span = span_between(start, end);
            return Some(match base.as_str() {
                "map" if args.len() == 1 => {
                    // `map[K]V`: the value type follows `]` directly.
                    let key = args.pop().unwrap();
                    let val = self.parse_type()?;
                    let span = span_between(start, val.span);
                    Type {
                        kind: TypeKind::Map {
                            key: Box::new(key),
                            val: Box::new(val),
                        },
                        span,
                    }
                }
                "opt" if args.len() == 1 => Type {
                    kind: TypeKind::Opt(Box::new(args.into_iter().next().unwrap())),
                    span,
                },
                "result" if args.len() == 2 => {
                    let err = args.pop().unwrap();
                    let ok = args.pop().unwrap();
                    Type {
                        kind: TypeKind::Result {
                            ok: Box::new(ok),
                            err: Box::new(err),
                        },
                        span,
                    }
                }
                "chan" if args.len() == 1 => Type {
                    kind: TypeKind::Chan(Box::new(args.into_iter().next().unwrap())),
                    span,
                },
                "join" if args.len() == 1 => Type {
                    kind: TypeKind::Join(Box::new(args.into_iter().next().unwrap())),
                    span,
                },
                _ => {
                    // `Box[int]`, `Pair[K, V]`: keep canonical spelling.
                    let inner = args
                        .into_iter()
                        .map(|t| type_to_string(&t))
                        .collect::<Vec<_>>()
                        .join(", ");
                    Type {
                        kind: TypeKind::Named(format!("{base}[{inner}]")),
                        span,
                    }
                }
            });
        }
        // `map[K]V`: value type follows directly without brackets.
        if base == "map" {
            // `map` without `[...]` is incomplete; report and keep going.
            self.err_here("expected 'map[K]V'");
            return Some(Type {
                kind: TypeKind::Named(base),
                span: span_between(start, self.peek().span),
            });
        }
        let end = self.toks[self.pos.saturating_sub(1).min(self.toks.len() - 1)].span;
        Some(Type {
            kind: TypeKind::Named(base),
            span: span_between(start, end),
        })
    }

    fn type_base_name(&mut self) -> Option<String> {
        // Dotted `pkg.Type` plus builtin keyword spellings.
        let t = self.bump();
        let mut name = match t.kind {
            TokenKind::Ident => t.text.unwrap_or_default(),
            k if k.is_type_name() => k.name().trim_matches('\'').to_string(),
            // `link` is both the `link "lib";` directive and a type name;
            // the lexer emits KwLink for both and the position decides.
            TokenKind::KwLink => "link".to_string(),
            TokenKind::KwFn
            | TokenKind::KwStruct
            | TokenKind::KwEnum
            | TokenKind::KwMut
            | TokenKind::KwGc
            | TokenKind::KwOwn => t.text.unwrap_or_default(),
            _ => {
                self.err_here("expected type");
                return None;
            }
        };
        while self.at(TokenKind::Dot) && self.peek2_kind() == TokenKind::Ident {
            self.bump(); // .
            let part = self.bump().text.unwrap_or_default();
            name.push('.');
            name.push_str(&part);
        }
        Some(name)
    }

    // ---- blocks & statements ----

    fn parse_block(&mut self) -> Option<Block> {
        let start = self.peek().span;
        self.expect(TokenKind::LBrace, "'{'")?;
        let mut stmts = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            match self.parse_stmt() {
                Some(s) => stmts.push(s),
                None => self.sync_stmt(),
            }
        }
        let end = self.bump().span; // }
        Some(Block {
            stmts,
            span: span_between(start, end),
        })
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        let start = self.peek().span;
        match self.peek_kind() {
            TokenKind::KwLet => {
                self.bump();
                self.parse_let_rest(start, false)
            }
            TokenKind::KwGuard => self.parse_guard(start),
            TokenKind::KwIf => self.parse_if(start),
            TokenKind::KwWhile => {
                self.bump();
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                let span = span_between(start, body.span);
                Some(Stmt::While { cond, body, span })
            }
            TokenKind::KwFor => self.parse_for(start),
            TokenKind::KwReturn => {
                self.bump();
                let value = if matches!(
                    self.peek_kind(),
                    TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
                ) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                let end = if self.at(TokenKind::Semi) {
                    self.bump().span
                } else {
                    value.as_ref().map(|e| e.span).unwrap_or(start)
                };
                Some(Stmt::Return {
                    value,
                    span: span_between(start, end),
                })
            }
            TokenKind::KwBreak => {
                self.bump();
                let end = if self.at(TokenKind::Semi) {
                    self.bump().span
                } else {
                    start
                };
                Some(Stmt::Break {
                    span: span_between(start, end),
                })
            }
            TokenKind::KwContinue => {
                self.bump();
                let end = if self.at(TokenKind::Semi) {
                    self.bump().span
                } else {
                    start
                };
                Some(Stmt::Continue {
                    span: span_between(start, end),
                })
            }
            TokenKind::KwSpawn => {
                self.bump();
                let call = self.parse_expr()?;
                let end = if self.at(TokenKind::Semi) {
                    self.bump().span
                } else {
                    call.span
                };
                Some(Stmt::Spawn {
                    call,
                    span: span_between(start, end),
                })
            }
            TokenKind::KwSelect => self.parse_select(start),
            TokenKind::KwSwitch => self.parse_switch_stmt(start),
            TokenKind::KwUnsafe => {
                self.bump();
                let body = self.parse_block()?;
                let span = span_between(start, body.span);
                Some(Stmt::Unsafe { body, span })
            }
            _ => self.parse_expr_or_assign(),
        }
    }

    fn parse_let_rest(&mut self, start: Span, _pub: bool) -> Option<Stmt> {
        let name = self.expect_ident()?;
        let mut ty = None;
        if self.at(TokenKind::Colon) {
            self.bump();
            ty = Some(self.parse_type()?);
        }
        self.expect(TokenKind::Assign, "'='")?;
        let init = self.parse_expr()?;
        let end = if self.at(TokenKind::Semi) {
            self.bump().span
        } else {
            init.span
        };
        Some(Stmt::Let {
            name,
            ty,
            init,
            span: span_between(start, end),
        })
    }

    fn parse_guard(&mut self, start: Span) -> Option<Stmt> {
        self.bump(); // guard
                     // `guard cond else { }` or `guard let x = e [else let e2 = ...] { }`.
        if self.at(TokenKind::KwLet) {
            self.bump();
            let name = self.expect_ident()?;
            self.expect(TokenKind::Assign, "'='")?;
            let expr = self.parse_expr()?;
            let mut err_name = None;
            let mut err_expr = None;
            if !self.at(TokenKind::KwElse) {
                self.err_here("expected 'else'");
                return None;
            }
            self.bump(); // else
            if self.at(TokenKind::KwLet) {
                self.bump();
                err_name = self.expect_ident();
                self.expect(TokenKind::Assign, "'='")?;
                err_expr = Some(self.parse_expr()?);
            }
            let body = self.parse_block()?;
            let span = span_between(start, body.span);
            return Some(Stmt::GuardLet {
                name,
                expr,
                err_name,
                err_expr,
                body,
                span,
            });
        }
        let cond = self.parse_expr()?;
        self.expect(TokenKind::KwElse, "'else'")?;
        let body = self.parse_block()?;
        let span = span_between(start, body.span);
        // Represent bare-guard as an If with from_guard semantics kept
        // in the condition position; consumers check `else_` presence.
        Some(Stmt::If {
            cond,
            then: body,
            else_: None,
            span,
        })
    }

    fn parse_if(&mut self, start: Span) -> Option<Stmt> {
        self.bump(); // if
                     // `if let x = e { } [else ...]`
        if self.at(TokenKind::KwLet) {
            self.bump();
            let name = self.expect_ident()?;
            self.expect(TokenKind::Assign, "'='")?;
            let expr = self.parse_expr()?;
            let then = self.parse_block()?;
            let mut err_name = None;
            let mut err_expr = None;
            let mut else_ = None;
            if self.at(TokenKind::KwElse) {
                self.bump();
                if self.at(TokenKind::KwLet) {
                    self.bump();
                    err_name = self.expect_ident();
                    self.expect(TokenKind::Assign, "'='")?;
                    err_expr = Some(self.parse_expr()?);
                    else_ = Some(self.parse_block()?);
                } else if self.at(TokenKind::KwIf) {
                    let s = self.peek().span;
                    let nested = self.parse_if(s)?;
                    else_ = Some(Block {
                        stmts: vec![nested],
                        span: s,
                    });
                } else {
                    else_ = Some(self.parse_block()?);
                }
            }
            let span = span_between(start, then.span);
            return Some(Stmt::IfLet {
                name,
                expr,
                then,
                err_name,
                err_expr,
                else_,
                span,
            });
        }
        let cond = self.parse_expr()?;
        // `if cond { }` used as guard-else shape is handled in parse_guard.
        let then = self.parse_block()?;
        let mut else_ = None;
        if self.at(TokenKind::KwElse) {
            self.bump();
            if self.at(TokenKind::KwIf) {
                let s = self.peek().span;
                let nested = self.parse_if(s)?;
                else_ = Some(Else::If(Box::new(nested)));
            } else {
                else_ = Some(Else::Block(self.parse_block()?));
            }
        }
        let end = match &else_ {
            Some(Else::Block(b)) => b.span,
            Some(Else::If(s)) => stmt_span(s),
            None => then.span,
        };
        Some(Stmt::If {
            cond,
            then,
            else_,
            span: span_between(start, end),
        })
    }

    fn parse_for(&mut self, start: Span) -> Option<Stmt> {
        self.bump(); // for
        let name = self.expect_ident()?;
        let mut name2 = None;
        if self.at(TokenKind::Comma) {
            self.bump();
            name2 = self.expect_ident();
        }
        self.expect(TokenKind::KwIn, "'in'")?;
        let iter = self.parse_expr()?;
        let body = self.parse_block()?;
        let span = span_between(start, body.span);
        // `for i in a..b` / `a..=b` is a range loop; anything else iterates.
        if let ExprKind::Binary { op, lhs, rhs } = &iter.kind {
            if op == ".." || op == "..=" {
                if name2.is_some() {
                    self.err_here("range loop takes one variable");
                }
                return Some(Stmt::ForRange {
                    name,
                    start: (**lhs).clone(),
                    end: (**rhs).clone(),
                    inclusive: op == "..=",
                    body,
                    span,
                });
            }
        }
        Some(Stmt::ForIn {
            name,
            name2,
            iter,
            body,
            span,
        })
    }

    fn parse_select(&mut self, start: Span) -> Option<Stmt> {
        self.bump(); // select
        self.expect(TokenKind::LBrace, "'{'")?;
        let mut cases = Vec::new();
        let mut default = None;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let cstart = self.peek().span;
            if self.at(TokenKind::KwDefault) {
                self.bump();
                default = Some(self.parse_block()?);
                continue;
            }
            if self.expect(TokenKind::KwCase, "'case'").is_none() {
                self.sync_stmt();
                continue;
            }
            // `case let v = chan_recv(ch)` or `case chan_send(ch, v)`.
            if self.at(TokenKind::KwLet) {
                self.bump();
                let bind = self.expect_ident();
                self.expect(TokenKind::Assign, "'='")?;
                let ch = self.parse_expr()?;
                let body = self.parse_block()?;
                let span = span_between(cstart, body.span);
                cases.push(SelectCase {
                    is_send: false,
                    bind,
                    ch,
                    val: None,
                    body,
                    span,
                });
            } else {
                let ch = self.parse_expr()?;
                let body = self.parse_block()?;
                let span = span_between(cstart, body.span);
                // A send arm parses as a call; keep channel + value split
                // best-effort: `chan_send(ch, v)` -> ch=ch, val=v.
                let (ch_e, val) = match &ch.kind {
                    ExprKind::Call {
                        name: Some(n),
                        args,
                        ..
                    } if n.ends_with("chan_send") && args.len() == 2 => {
                        (args[0].clone(), Some(args[1].clone()))
                    }
                    _ => (ch, None),
                };
                cases.push(SelectCase {
                    is_send: true,
                    bind: None,
                    ch: ch_e,
                    val,
                    body,
                    span,
                });
            }
        }
        let end = self.bump().span;
        Some(Stmt::Select {
            cases,
            default,
            span: span_between(start, end),
        })
    }

    fn parse_switch_stmt(&mut self, start: Span) -> Option<Stmt> {
        self.bump(); // switch
        let scrut = self.parse_expr()?;
        self.expect(TokenKind::LBrace, "'{'")?;
        let mut cases = Vec::new();
        let mut default = None;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let cstart = self.peek().span;
            if self.at(TokenKind::KwDefault) {
                self.bump();
                default = Some(self.parse_block()?);
                continue;
            }
            if self.expect(TokenKind::KwCase, "'case'").is_none() {
                self.sync_stmt();
                continue;
            }
            let mut vals = Vec::new();
            loop {
                vals.push(self.parse_expr()?);
                if self.at(TokenKind::Comma) {
                    self.bump();
                    continue;
                }
                break;
            }
            let body = self.parse_block()?;
            let span = span_between(cstart, body.span);
            cases.push(SwitchCase {
                vals,
                body: SwitchBody::Block(body),
                span,
            });
        }
        let end = self.bump().span;
        Some(Stmt::Switch {
            scrut,
            cases,
            default,
            span: span_between(start, end),
        })
    }

    fn parse_expr_or_assign(&mut self) -> Option<Stmt> {
        let start = self.peek().span;
        let expr = self.parse_expr()?;
        // Compound assignment desugars in the real parser to `x = x OP v`.
        let op = match self.peek_kind() {
            TokenKind::Assign => "=",
            TokenKind::PlusEq => "+=",
            TokenKind::MinusEq => "-=",
            TokenKind::StarEq => "*=",
            TokenKind::SlashEq => "/=",
            TokenKind::PercentEq => "%=",
            TokenKind::AmpEq => "&=",
            TokenKind::PipeEq => "|=",
            TokenKind::CaretEq => "^=",
            TokenKind::ShlEq => "<<=",
            TokenKind::ShrEq => ">>=",
            _ => {
                let end = if self.at(TokenKind::Semi) {
                    self.bump().span
                } else {
                    expr.span
                };
                return Some(Stmt::Expr {
                    expr,
                    span: span_between(start, end),
                });
            }
        }
        .to_string();
        self.bump();
        let value = self.parse_expr()?;
        let end = if self.at(TokenKind::Semi) {
            self.bump().span
        } else {
            value.span
        };
        Some(Stmt::Assign {
            target: expr,
            op,
            value,
            span: span_between(start, end),
        })
    }

    // ---- expressions (C precedence) ----

    fn parse_expr(&mut self) -> Option<Expr> {
        self.parse_coalesce()
    }

    fn parse_coalesce(&mut self) -> Option<Expr> {
        let mut lhs = self.parse_or()?;
        while self.at(TokenKind::QuestionQuestion) {
            self.bump();
            let rhs = self.parse_or()?;
            let span = span_between(lhs.span, rhs.span);
            lhs = Expr {
                span,
                kind: ExprKind::Binary {
                    op: "??".to_string(),
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        Some(lhs)
    }

    fn parse_or(&mut self) -> Option<Expr> {
        self.parse_bin(|p| p.parse_and(), &[TokenKind::OrOr])
    }

    fn parse_and(&mut self) -> Option<Expr> {
        self.parse_bin(|p| p.parse_bitor(), &[TokenKind::AndAnd])
    }

    fn parse_bitor(&mut self) -> Option<Expr> {
        self.parse_bin(|p| p.parse_bitxor(), &[TokenKind::Pipe])
    }

    fn parse_bitxor(&mut self) -> Option<Expr> {
        self.parse_bin(|p| p.parse_bitand(), &[TokenKind::Caret])
    }

    fn parse_bitand(&mut self) -> Option<Expr> {
        // Note: `&x` borrow is prefix-only, so infix `&` is unambiguous.
        self.parse_bin(|p| p.parse_eq(), &[TokenKind::Amp])
    }

    fn parse_eq(&mut self) -> Option<Expr> {
        self.parse_bin(|p| p.parse_cmp(), &[TokenKind::EqEq, TokenKind::BangEq])
    }

    fn parse_cmp(&mut self) -> Option<Expr> {
        self.parse_bin(
            |p| p.parse_shift(),
            &[TokenKind::Lt, TokenKind::Gt, TokenKind::Lte, TokenKind::Gte],
        )
    }

    fn parse_shift(&mut self) -> Option<Expr> {
        self.parse_bin(|p| p.parse_add(), &[TokenKind::Shl, TokenKind::Shr])
    }

    fn parse_add(&mut self) -> Option<Expr> {
        self.parse_bin(|p| p.parse_mul(), &[TokenKind::Plus, TokenKind::Minus])
    }

    fn parse_mul(&mut self) -> Option<Expr> {
        self.parse_bin(
            |p| p.parse_unary(),
            &[TokenKind::Star, TokenKind::Slash, TokenKind::Percent],
        )
    }

    fn parse_bin(
        &mut self,
        mut sub: impl FnMut(&mut Self) -> Option<Expr>,
        kinds: &[TokenKind],
    ) -> Option<Expr> {
        let mut lhs = sub(self)?;
        // Ranges `..` / `..=` bind loosest of the arithmetic group: they
        // are parsed here so `for i in 0..n` and slices share one shape.
        loop {
            let op =
                if !self.no_range && kinds.contains(&TokenKind::Plus) && self.at(TokenKind::DotDot)
                {
                    ".."
                } else if !self.no_range
                    && kinds.contains(&TokenKind::Plus)
                    && self.at(TokenKind::DotDotEq)
                {
                    "..="
                } else if kinds.iter().any(|k| *k == self.peek_kind()) {
                    op_text(self.peek_kind())
                } else {
                    break;
                };
            self.bump();
            let rhs = sub(self)?;
            let span = span_between(lhs.span, rhs.span);
            lhs = Expr {
                span,
                kind: ExprKind::Binary {
                    op: op.to_string(),
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
            // `..` only participates once per level; other ops left-assoc.
            if op == ".." || op == "..=" {
                break;
            }
        }
        Some(lhs)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        if matches!(
            self.peek_kind(),
            TokenKind::Bang | TokenKind::Minus | TokenKind::Tilde
        ) {
            let start = self.bump();
            let operand = self.parse_unary()?;
            let span = span_between(start.span, operand.span);
            return Some(Expr {
                span,
                kind: ExprKind::Unary {
                    op: op_text(start.kind).to_string(),
                    operand: Box::new(operand),
                },
            });
        }
        // Borrow expressions and dereference are prefix-only.
        // Mirrors the T_AMP and T_STAR arms of slang parse_unary.
        if self.at(TokenKind::Amp) || self.at(TokenKind::Star) {
            let start = self.bump();
            let mut op = op_text(start.kind).to_string();
            if start.kind == TokenKind::Amp && self.at(TokenKind::KwMut) {
                self.bump();
                op = "&mut".to_string();
            }
            let operand = self.parse_unary()?;
            let span = span_between(start.span, operand.span);
            return Some(Expr {
                span,
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
            });
        }
        self.parse_postfix()
    }

    /// Parse an index/slice interior: like `parse_expr`, but a top-level
    /// `..` belongs to the slice (`xs[4..]`), not to a range expression.
    /// Restores the flag even when parsing fails.
    fn parse_index_expr(&mut self) -> Option<Expr> {
        let saved = self.no_range;
        self.no_range = true;
        let out = self.parse_expr();
        self.no_range = saved;
        out
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut base = self.parse_primary()?;
        loop {
            // `as` casts.
            if self.at(TokenKind::KwAs) {
                self.bump();
                let ty = self.parse_type()?;
                let span = span_between(base.span, ty.span);
                base = Expr {
                    span,
                    kind: ExprKind::Cast {
                        operand: Box::new(base),
                        ty,
                    },
                };
                continue;
            }
            // Calls: `f(x)` or `recv.method(x)` / `pkg.func(x)`.
            if self.at(TokenKind::LParen) {
                self.bump();
                let mut args = Vec::new();
                while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
                    args.push(self.parse_expr()?);
                    if self.at(TokenKind::Comma) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::RParen, "')'")?;
                let end = self.toks[self.pos.saturating_sub(1).min(self.toks.len() - 1)].span;
                let span = span_between(base.span, end);
                base = match base.kind {
                    ExprKind::Ident(name) => Expr {
                        span,
                        kind: ExprKind::Call {
                            name: Some(name),
                            callee: None,
                            args,
                        },
                    },
                    ExprKind::Field { base: recv, name } => {
                        // Bare `p.m(...)` keeps dotted-call form so name
                        // resolution via split_dotted is untouched.
                        if let ExprKind::Ident(recv_name) = &recv.kind {
                            Expr {
                                span,
                                kind: ExprKind::Call {
                                    name: Some(format!("{recv_name}.{name}")),
                                    callee: None,
                                    args,
                                },
                            }
                        } else {
                            Expr {
                                span,
                                kind: ExprKind::MethodCall { recv, name, args },
                            }
                        }
                    }
                    _ => Expr {
                        span,
                        kind: ExprKind::Call {
                            name: None,
                            callee: Some(Box::new(base)),
                            args,
                        },
                    },
                };
                continue;
            }
            // Index / slice: `xs[i]`, `xs[a..b]`, `b[..n]`, `b[n..]`.
            if self.at(TokenKind::LBracket) {
                self.bump();
                // Slice with omitted start: `[..end]`.
                if self.at(TokenKind::DotDot) || self.at(TokenKind::DotDotEq) {
                    let inclusive = self.peek_kind() == TokenKind::DotDotEq;
                    self.bump();
                    let end_e = if self.at(TokenKind::RBracket) {
                        None
                    } else {
                        Some(Box::new(self.parse_index_expr()?))
                    };
                    self.expect(TokenKind::RBracket, "']'")?;
                    let end = self.toks[self.pos.saturating_sub(1).min(self.toks.len() - 1)].span;
                    let span = span_between(base.span, end);
                    base = Expr {
                        span,
                        kind: ExprKind::Slice {
                            base: Box::new(base),
                            start: None,
                            end: end_e,
                            inclusive,
                        },
                    };
                    continue;
                }
                let idx = self.parse_index_expr()?;
                if self.at(TokenKind::DotDot) || self.at(TokenKind::DotDotEq) {
                    let inclusive = self.peek_kind() == TokenKind::DotDotEq;
                    self.bump();
                    let end_e = if self.at(TokenKind::RBracket) {
                        None
                    } else {
                        Some(Box::new(self.parse_index_expr()?))
                    };
                    self.expect(TokenKind::RBracket, "']'")?;
                    let end = self.toks[self.pos.saturating_sub(1).min(self.toks.len() - 1)].span;
                    let span = span_between(base.span, end);
                    base = Expr {
                        span,
                        kind: ExprKind::Slice {
                            base: Box::new(base),
                            start: Some(Box::new(idx)),
                            end: end_e,
                            inclusive,
                        },
                    };
                    continue;
                }
                self.expect(TokenKind::RBracket, "']'")?;
                let end = self.toks[self.pos.saturating_sub(1).min(self.toks.len() - 1)].span;
                let span = span_between(base.span, end);
                base = Expr {
                    span,
                    kind: ExprKind::Index {
                        base: Box::new(base),
                        index: Box::new(idx),
                    },
                };
                continue;
            }
            if self.at(TokenKind::Dot) {
                self.bump();
                let name = self.expect_ident()?;
                // `Type.Variant` enum references stay Ident-shaped so the
                // enum pass can rewrite them; only continue as Field.
                let end = self.toks[self.pos.saturating_sub(1).min(self.toks.len() - 1)].span;
                let span = span_between(base.span, end);
                base = Expr {
                    span,
                    kind: ExprKind::Field {
                        base: Box::new(base),
                        name,
                    },
                };
                continue;
            }
            break;
        }
        Some(base)
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        let t = self.peek().clone();
        match t.kind {
            TokenKind::Int => {
                self.bump();
                Some(Expr {
                    span: t.span,
                    kind: ExprKind::Int {
                        value: t.int_val,
                        big_u64: t.big_u64,
                    },
                })
            }
            TokenKind::Float => {
                self.bump();
                Some(Expr {
                    span: t.span,
                    kind: ExprKind::Float(t.float_val),
                })
            }
            TokenKind::String => {
                self.bump();
                Some(Expr {
                    span: t.span,
                    kind: ExprKind::String(t.text.unwrap_or_default()),
                })
            }
            TokenKind::Bytes => {
                self.bump();
                Some(Expr {
                    span: t.span,
                    kind: ExprKind::Bytes(t.bytes.unwrap_or_default()),
                })
            }
            TokenKind::KwTrue | TokenKind::KwFalse => {
                self.bump();
                Some(Expr {
                    span: t.span,
                    kind: ExprKind::Bool(t.kind == TokenKind::KwTrue),
                })
            }
            TokenKind::KwSwitch => self.parse_switch_expr(),
            TokenKind::KwSpawn => {
                let start = self.bump().span;
                let call = self.parse_expr()?;
                let span = span_between(start, call.span);
                Some(Expr {
                    span,
                    kind: ExprKind::Spawn(Box::new(call)),
                })
            }
            TokenKind::LParen => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen, "')'")?;
                Some(e)
            }
            TokenKind::LBracket => {
                let start = self.bump().span;
                let mut elems = Vec::new();
                while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
                    elems.push(self.parse_expr()?);
                    if self.at(TokenKind::Comma) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                let end = self.peek().span;
                self.expect(TokenKind::RBracket, "']'")?;
                Some(Expr {
                    span: span_between(start, end),
                    kind: ExprKind::List(elems),
                })
            }
            TokenKind::LBrace => self.parse_map_lit(),
            TokenKind::Ident => {
                // Struct-literal type reference: `Name[.Member]*[[args]]`
                // followed by `{ f: ... }`. Scan ahead first so plain
                // idents, field access, and indexing are untouched when
                // the struct-literal opening is absent (mirrors
                // generic_literal_ahead + the 'ident {' test in slang's
                // parser.c).
                if self.struct_tyname_ahead() {
                    let span = t.span;
                    let mut full = self.bump().text.unwrap_or_default();
                    while self.at(TokenKind::Dot) {
                        self.bump();
                        // Member shape mirrors the scanner above: Ident,
                        // keyword, or type name (e.g. `httpc.Request`).
                        let mk = self.peek_kind();
                        if mk == TokenKind::Ident || mk.is_keyword() || mk.is_type_name() {
                            full.push('.');
                            full.push_str(&self.bump().text.unwrap_or_default());
                        } else {
                            break;
                        }
                    }
                    if self.at(TokenKind::LBracket) {
                        full.push_str(&self.parse_type_arg_list()?);
                    }
                    return self.parse_struct_lit_named(full, span);
                }
                self.bump();
                Some(Expr {
                    span: t.span,
                    kind: ExprKind::Ident(t.text.unwrap_or_default()),
                })
            }
            k if k.is_type_name() => {
                // `Status.Paid`, `Box[int] { ... }` start from a type name.
                let name = t.text.clone().unwrap_or_default();
                self.bump();
                let mut full = name;
                while self.at(TokenKind::Dot) && self.peek2_kind() == TokenKind::Ident {
                    self.bump();
                    full.push('.');
                    full.push_str(&self.bump().text.unwrap_or_default());
                }
                if self.at(TokenKind::LBracket) {
                    // Generic struct literal `Box[int] { v: 1 }`.
                    self.bump();
                    let mut args = Vec::new();
                    while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
                        args.push(type_to_string(&self.parse_type()?));
                        if self.at(TokenKind::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    self.expect(TokenKind::RBracket, "']'")?;
                    full.push('[');
                    full.push_str(&args.join(", "));
                    full.push(']');
                }
                if self.at(TokenKind::LBrace) {
                    return self.parse_struct_lit_named(full, t.span);
                }
                Some(Expr {
                    span: t.span,
                    kind: ExprKind::Ident(full),
                })
            }
            _ => {
                self.err_here(&format!("expected expression, got {}", t.kind.name()));
                None
            }
        }
    }

    /// True when the tokens at the cursor form a struct-literal type
    /// reference: `Name[.Member]*[[type args]] { field:`.
    /// Mirrors generic_literal_ahead + the 'ident {' test in slang's
    /// parser.c: brackets must match (nesting `[]` and `()`), and the
    /// brace must open `field:`. A block after an index or condition
    /// never satisfies this.
    fn struct_tyname_ahead(&self) -> bool {
        let toks = &self.toks;
        let mut i = self.pos + 1; // token after the leading Ident
        // dotted members: `.Name`
        loop {
            if toks.get(i).map(|t| t.kind) != Some(TokenKind::Dot) {
                break;
            }
            match toks.get(i + 1).map(|t| t.kind) {
                Some(TokenKind::Ident) => {}
                Some(k) if k.is_keyword() || k.is_type_name() => {}
                _ => return false,
            }
            i += 2;
        }
        // optional `[type args]`: scan to the matching `]`.
        if toks.get(i).map(|t| t.kind) == Some(TokenKind::LBracket) {
            let mut depth = 0u32;
            let mut j = i;
            loop {
                let Some(t) = toks.get(j) else {
                    return false;
                };
                match t.kind {
                    TokenKind::LBracket | TokenKind::LParen => depth += 1,
                    TokenKind::RParen => depth = depth.saturating_sub(1),
                    TokenKind::RBracket => {
                        if depth <= 1 {
                            break;
                        }
                        depth -= 1;
                    }
                    TokenKind::Eof => return false,
                    _ => {}
                }
                j += 1;
            }
            i = j + 1; // token after the matching `]`
        }
        // struct-literal opening: `{` Ident `:`.
        matches!(toks.get(i).map(|t| t.kind), Some(TokenKind::LBrace))
            && matches!(
                toks.get(i + 1).map(|t| t.kind),
                Some(TokenKind::Ident)
            )
            && matches!(
                toks.get(i + 2).map(|t| t.kind),
                Some(TokenKind::Colon)
            )
    }

    /// Parse `[A, B]` type arguments, returning them as `[A, B]` text.
    /// The caller must have established the brackets via
    /// struct_tyname_ahead.
    fn parse_type_arg_list(&mut self) -> Option<String> {
        self.expect(TokenKind::LBracket, "'['")?;
        let mut args = Vec::new();
        while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
            args.push(type_to_string(&self.parse_type()?));
            if self.at(TokenKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokenKind::RBracket, "']'")?;
        Some(format!("[{}]", args.join(", ")))
    }

    fn parse_map_lit(&mut self) -> Option<Expr> {
        let start = self.bump().span; // {
        let mut pairs = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let k = self.parse_expr()?;
            self.expect(TokenKind::Colon, "':'")?;
            let v = self.parse_expr()?;
            pairs.push((k, v));
            if self.at(TokenKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        let end = self.peek().span;
        self.expect(TokenKind::RBrace, "'}'")?;
        Some(Expr {
            span: span_between(start, end),
            kind: ExprKind::MapLit(pairs),
        })
    }

    fn parse_struct_lit_named(&mut self, tyname: String, nspan: Span) -> Option<Expr> {
        self.expect(TokenKind::LBrace, "'{'")?;
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let fname = match self.expect_ident() {
                Some(n) => n,
                None => {
                    self.sync_stmt();
                    continue;
                }
            };
            // Struct literals are `field: value` (see `EX_STRUCTLIT` in
            // `src/ast.h`); anything else is not a struct literal.
            if self.at(TokenKind::Colon) {
                self.bump();
            } else {
                self.err_here("expected ':' in struct literal");
                self.sync_stmt();
                continue;
            }
            let v = match self.parse_expr() {
                Some(e) => e,
                None => {
                    self.sync_stmt();
                    continue;
                }
            };
            fields.push((fname, v));
            if self.at(TokenKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        let end = self.peek().span;
        self.expect(TokenKind::RBrace, "'}'")?;
        let span = span_between(nspan, end);
        Some(Expr {
            span,
            kind: ExprKind::StructLit { tyname, fields },
        })
    }

    fn parse_switch_expr(&mut self) -> Option<Expr> {
        let start = self.bump().span; // switch
        let scrut = self.parse_expr()?;
        self.expect(TokenKind::LBrace, "'{'")?;
        let mut cases = Vec::new();
        let mut default = None;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let cstart = self.peek().span;
            if self.at(TokenKind::KwDefault) {
                self.bump();
                let body = self.parse_block()?;
                // Expression-switch default is a block yielding one value
                // only in statement position; here take its single expr.
                let expr = if body.stmts.len() == 1 {
                    if let Stmt::Expr { expr, .. } = body.stmts.into_iter().next().unwrap() {
                        expr
                    } else {
                        self.err_here("expected value in switch arm");
                        return None;
                    }
                } else {
                    // `default { "other" }` without `;` parses the block's
                    // expression directly when possible.
                    self.err_here("expected value in switch arm");
                    return None;
                };
                default = Some(Box::new(SwitchBody::Expr(expr)));
                continue;
            }
            if self.expect(TokenKind::KwCase, "'case'").is_none() {
                self.sync_stmt();
                continue;
            }
            let mut vals = Vec::new();
            loop {
                vals.push(self.parse_expr()?);
                if self.at(TokenKind::Comma) {
                    self.bump();
                    continue;
                }
                break;
            }
            // Arm value is either `{ expr }` or a bare expression.
            let arm = if self.at(TokenKind::LBrace) {
                // Peek: `{ "ok" }` single-expr block.
                let b = self.parse_block()?;
                if b.stmts.len() == 1 {
                    if let Stmt::Expr { expr, .. } = b.stmts.into_iter().next().unwrap() {
                        SwitchBody::Expr(expr)
                    } else {
                        SwitchBody::Block(Block {
                            stmts: vec![],
                            span: b.span,
                        })
                    }
                } else {
                    SwitchBody::Block(b)
                }
            } else {
                SwitchBody::Expr(self.parse_expr()?)
            };
            let span = span_between(cstart, switch_body_span(&arm));
            cases.push(SwitchCase {
                vals,
                body: arm,
                span,
            });
        }
        let end = self.bump().span;
        Some(Expr {
            span: span_between(start, end),
            kind: ExprKind::Switch {
                scrut: Box::new(scrut),
                cases,
                default,
            },
        })
    }
}

enum Top {
    Item(Item),
    Stmt(Stmt),
}

fn op_text(k: TokenKind) -> &'static str {
    match k {
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::Percent => "%",
        TokenKind::EqEq => "==",
        TokenKind::BangEq => "!=",
        TokenKind::Lt => "<",
        TokenKind::Gt => ">",
        TokenKind::Lte => "<=",
        TokenKind::Gte => ">=",
        TokenKind::Amp => "&",
        TokenKind::Pipe => "|",
        TokenKind::Caret => "^",
        TokenKind::Shl => "<<",
        TokenKind::Shr => ">>",
        TokenKind::AndAnd => "&&",
        TokenKind::OrOr => "||",
        TokenKind::Bang => "!",
        TokenKind::Tilde => "~",
        _ => "?",
    }
}

fn stmt_span(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::ForRange { span, .. }
        | Stmt::ForIn { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Expr { span, .. }
        | Stmt::GuardLet { span, .. }
        | Stmt::Spawn { span, .. }
        | Stmt::IfLet { span, .. }
        | Stmt::Select { span, .. }
        | Stmt::Switch { span, .. }
        | Stmt::Unsafe { span, .. } => *span,
    }
}

fn switch_body_span(b: &SwitchBody) -> Span {
    match b {
        SwitchBody::Block(block) => block.span,
        SwitchBody::Expr(e) => e.span,
    }
}

fn type_to_string(t: &Type) -> String {
    match &t.kind {
        TypeKind::Named(n) => n.clone(),
        TypeKind::Array(i) => format!("[{}]", type_to_string(i)),
        TypeKind::Map { key, val } => {
            format!("map[{}]{}", type_to_string(key), type_to_string(val))
        }
        TypeKind::Opt(i) => format!("opt[{}]", type_to_string(i)),
        TypeKind::Result { ok, err } => {
            format!("result[{}, {}]", type_to_string(ok), type_to_string(err))
        }
        TypeKind::Chan(i) => format!("chan[{}]", type_to_string(i)),
        TypeKind::Join(i) => format!("join[{}]", type_to_string(i)),
        TypeKind::Ref(i) => format!("&{}", type_to_string(i)),
        TypeKind::RefMut(i) => format!("&mut {}", type_to_string(i)),
        TypeKind::Own(i) => format!("own {}", type_to_string(i)),
        TypeKind::Gc(i) => format!("gc {}", type_to_string(i)),
        TypeKind::Ptr(i) => format!("*{}", type_to_string(i)),
        TypeKind::Fn { params, ret } => {
            let ps = params
                .iter()
                .map(type_to_string)
                .collect::<Vec<_>>()
                .join(", ");
            match ret {
                Some(r) => format!("fn({ps}) -> {}", type_to_string(r)),
                None => format!("fn({ps})"),
            }
        }
    }
}

/// Parse tokens into a file AST, collecting errors for IDE recovery.
/// Never panics on malformed input; errors are returned alongside.
pub fn parse(tokens: Vec<Token>) -> (File, Vec<ParseError>) {
    Parser::new(tokens).parse_file()
}
