//! Code completion.
//!
//! The cursor context comes from the raw text, not the AST: completion is
//! asked for mid-keystroke (`p.`, `let x = pk`), where the tree around the
//! cursor is broken but the text says exactly what is being typed. The
//! AST still answers *what is in scope* and *what type a receiver has*,
//! because the parser recovers everything before the cursor.
//!
//! Visibility mirrors slangc (checked against slang `ec4a417f`): another
//! package's functions, globals, and types are offered only when `pub`
//! (`core.c` `sig_find_pub`, `expr.c` global and call checks,
//! `core.c` / `enum.c` "not exported" errors). Methods are offered on any
//! value whose type has them, since slangc applies no export check to
//! method calls (`expr.c` receiver path). Builtins are `BUILTIN_NAMES` in
//! `core.c`, minus the internal `__` helpers.

use crate::native_sigs_generated::NATIVE_SIGS;
use crate::scope::Mode;
use crate::{generic_base, is_native_package, Db, FileData, FuncSym, StructSym};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Variable,
    Function,
    Method,
    Field,
    Struct,
    Enum,
    EnumMember,
    Constant,
    Module,
    Keyword,
    TypeName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    /// One-line signature or type shown beside the label.
    pub detail: String,
    pub docs: String,
    /// Relevance bucket, lower first: locals and members, then package
    /// items, imports, builtins, keywords.
    pub rank: u8,
}

const RANK_LOCAL: u8 = 0;
const RANK_PACKAGE: u8 = 1;
const RANK_IMPORT: u8 = 2;
const RANK_BUILTIN: u8 = 3;
const RANK_KEYWORD: u8 = 4;

/// Builtin functions callable without an import (`BUILTIN_NAMES`).
const BUILTINS: &[&str] = &[
    "print",
    "println",
    "len",
    "push",
    "pop",
    "to_str",
    "inspect",
    "to_bytes",
    "to_int",
    "to_float",
    "to_le",
    "to_be",
    "from_le",
    "from_be",
    "has",
    "del",
    "exit",
    "panic",
    "assert",
    "some",
    "none",
    "ok",
    "err",
    "err_of",
    "nullptr",
    "bytes_ptr",
    "wire_put",
    "wire_put_bytes",
    "make_chan",
    "chan_send",
    "chan_recv",
    "chan_close",
    "make_mutex",
    "mutex_lock",
    "mutex_unlock",
    "mutex_trylock",
    "join_wait",
    "arena_new",
    "until_of",
    "until_never",
    "until_hit",
    "fault_timeout",
    "fault_reset",
    "fault_closed",
    "fault_io",
    "fault_refused",
    "fault_kind",
    "fault_code",
    "fault_op",
    "peer_v4",
    "peer_port",
    "trip_new",
    "link_listen",
    "link_dial",
];

/// Keywords, as the lexer recognises them (`token.rs` `keyword`).
const KEYWORDS: &[&str] = &[
    "let", "fn", "if", "else", "while", "return", "true", "false", "pub", "import", "guard", "for",
    "in", "as", "struct", "enum", "gc", "own", "mut", "impl", "extern", "link", "spawn", "break",
    "continue", "unsafe", "select", "switch", "case", "default",
];

/// Builtin type names, as the lexer recognises them.
const TYPE_NAMES: &[&str] = &[
    "int", "float", "str", "bool", "bytes", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64",
    "f32", "map", "opt", "result", "duration", "rawptr", "arena", "chan", "join",
];

/// Keywords after which the next word declares a new name: nothing that
/// already exists is a useful suggestion there.
const DECLARING: &[&str] = &["let", "fn", "struct", "enum", "as", "for"];

#[derive(Debug, PartialEq, Eq)]
enum Context {
    /// After `recv.`: `receiver` is the dotted ident chain before the dot.
    Member { receiver: String },
    /// A bare name position.
    Scope,
    /// Inside a string or comment, naming a new binding, or after a
    /// receiver too complex to resolve from text (`f().`, `a[0].`).
    Nothing,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True when the cursor sits inside a string literal or a `//` comment.
/// Strings and comments never span lines (`lexer.rs`), so scanning the
/// cursor's own line is exact.
fn in_string_or_comment(line: &[u8]) -> bool {
    let mut in_str = false;
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        if in_str {
            match b {
                b'\\' => i += 1,
                b'"' => in_str = false,
                _ => {}
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'/' && line.get(i + 1) == Some(&b'/') {
            return true;
        }
        i += 1;
    }
    in_str
}

/// Classify the cursor position from the text before it.
fn context_at(src: &str, offset: usize) -> Context {
    let b = src.as_bytes();
    let line_start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    if in_string_or_comment(&b[line_start..offset]) {
        return Context::Nothing;
    }
    let mut start = offset;
    while start > 0 && is_ident_byte(b[start - 1]) {
        start -= 1;
    }
    if start > 0 && b[start - 1] == b'.' {
        let dot = start - 1;
        // `..` / `..=` is a range, not member access.
        if dot > 0 && b[dot - 1] == b'.' {
            return Context::Scope;
        }
        let mut head = dot;
        loop {
            let seg_end = head;
            while head > 0 && is_ident_byte(b[head - 1]) {
                head -= 1;
            }
            if head == seg_end {
                // Empty segment: `f().`, `a[0].`, `"s".`, or a lone dot.
                return Context::Nothing;
            }
            if head > 0 && b[head - 1] == b'.' && head > 1 && is_ident_byte(b[head - 2]) {
                head -= 1;
                continue;
            }
            break;
        }
        // A receiver that is itself the tail of a call or index
        // (`f().x.`) cannot be typed from text alone. A range before it
        // (`0..p.`) is fine: the receiver starts after the `..`.
        let after_range = head > 1 && b[head - 2] == b'.';
        if head > 0 && b[head - 1] == b'.' && !after_range {
            return Context::Nothing;
        }
        let receiver = &src[head..dot];
        if receiver.starts_with(|c: char| c.is_ascii_digit()) {
            // `1.` is a float literal being typed.
            return Context::Nothing;
        }
        return Context::Member {
            receiver: receiver.to_string(),
        };
    }
    // The word before the cursor's word: `let |` declares a name.
    let mut p = start;
    while p > 0 && matches!(b[p - 1], b' ' | b'\t') {
        p -= 1;
    }
    let word_end = p;
    while p > 0 && is_ident_byte(b[p - 1]) {
        p -= 1;
    }
    if word_end < start && DECLARING.contains(&&src[p..word_end]) {
        return Context::Nothing;
    }
    Context::Scope
}

fn fn_detail(f: &FuncSym, name: &str) -> String {
    let params = f
        .params
        .iter()
        .map(|(n, t)| format!("{n}: {t}"))
        .collect::<Vec<_>>()
        .join(", ");
    match &f.ret {
        Some(r) => format!("fn {name}({params}) -> {r}"),
        None => format!("fn {name}({params})"),
    }
}

fn type_item(s: &StructSym, rank: u8) -> Completion {
    Completion {
        label: s.name.clone(),
        kind: if s.is_enum {
            CompletionKind::Enum
        } else {
            CompletionKind::Struct
        },
        detail: format!("{} {}", if s.is_enum { "enum" } else { "struct" }, s.name),
        docs: s.docs.clone(),
        rank,
    }
}

fn variants(s: &StructSym) -> Vec<Completion> {
    s.fields
        .iter()
        .map(|(v, _, _)| Completion {
            label: v.clone(),
            kind: CompletionKind::EnumMember,
            detail: format!("{}.{v}", s.name),
            docs: s.docs.clone(),
            rank: RANK_LOCAL,
        })
        .collect()
}

/// Collects completions, keeping the first item per label so that inner
/// scopes shadow outer ones exactly as name lookup does.
#[derive(Default)]
struct Out {
    seen: HashSet<String>,
    items: Vec<Completion>,
}

impl Out {
    fn push(&mut self, c: Completion) {
        if self.seen.insert(c.label.clone()) {
            self.items.push(c);
        }
    }
}

impl Db {
    /// Completions at byte `offset` of `src`, the editor's live buffer.
    ///
    /// The cursor context is read from `src` rather than the indexed copy:
    /// a buffer that does not lex (an unclosed `"` mid-typing) is never
    /// indexed, and the index keeps the last good version. Scope and types
    /// still come from that last good parse.
    ///
    /// Every candidate valid at the cursor is returned; clients filter by
    /// the typed prefix themselves (fuzzy matching needs the full set).
    pub fn completions(&self, path: &Path, src: &str, offset: usize) -> Vec<Completion> {
        let Some(fd) = self.file(path) else {
            return Vec::new();
        };
        let mut offset = offset.min(src.len());
        while !src.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut out = Out::default();
        match context_at(src, offset) {
            Context::Nothing => {}
            Context::Member { receiver } => {
                self.member_completions(fd, &receiver, offset, &mut out)
            }
            Context::Scope => self.scope_completions(fd, offset, &mut out),
        }
        out.items
    }

    fn scope_completions(&self, fd: &FileData, offset: usize, out: &mut Out) {
        let package = fd.package.as_str();
        // Innermost first so shadowing bindings win the label.
        for b in self.bindings_at(fd, offset, Mode::Complete).iter().rev() {
            out.push(Completion {
                label: b.name.to_string(),
                kind: CompletionKind::Variable,
                detail: self.binding_type(fd, b),
                docs: String::new(),
                rank: RANK_LOCAL,
            });
        }
        let mut pkg_items: Vec<Completion> = Vec::new();
        for f in self.funcs.values().filter(|f| f.package == package) {
            pkg_items.push(Completion {
                label: f.name.clone(),
                kind: CompletionKind::Function,
                detail: fn_detail(f, &f.name),
                docs: f.docs.clone(),
                rank: RANK_PACKAGE,
            });
        }
        for s in self.structs.values().filter(|s| s.package == package) {
            pkg_items.push(type_item(s, RANK_PACKAGE));
        }
        // Entry-package top-level `let`s are `main()` locals: offered by
        // the scope walk where visible, never as globals inside functions.
        if !self.is_entry_file(fd) {
            for c in self.consts.values().filter(|c| c.package == package) {
                pkg_items.push(Completion {
                    label: c.name.clone(),
                    kind: CompletionKind::Constant,
                    detail: format!("let {}: {}", c.name, c.ty),
                    docs: c.docs.clone(),
                    rank: RANK_PACKAGE,
                });
            }
        }
        // HashMap order is arbitrary; sort so results are deterministic.
        pkg_items.sort_by(|a, b| a.label.cmp(&b.label));
        for c in pkg_items {
            out.push(c);
        }
        let mut imports: Vec<(&String, &String)> = self
            .imports
            .iter()
            .filter(|((pkg, _), _)| pkg == package)
            .map(|((_, bind), target)| (bind, target))
            .collect();
        imports.sort();
        for (bind, target) in imports {
            out.push(Completion {
                label: bind.clone(),
                kind: CompletionKind::Module,
                detail: format!("package {target}"),
                docs: String::new(),
                rank: RANK_IMPORT,
            });
        }
        for name in BUILTINS {
            out.push(Completion {
                label: (*name).to_string(),
                kind: CompletionKind::Function,
                detail: "builtin".to_string(),
                docs: String::new(),
                rank: RANK_BUILTIN,
            });
        }
        for kw in KEYWORDS {
            out.push(Completion {
                label: (*kw).to_string(),
                kind: CompletionKind::Keyword,
                detail: String::new(),
                docs: String::new(),
                rank: RANK_KEYWORD,
            });
        }
        for ty in TYPE_NAMES {
            out.push(Completion {
                label: (*ty).to_string(),
                kind: CompletionKind::TypeName,
                detail: String::new(),
                docs: String::new(),
                rank: RANK_KEYWORD,
            });
        }
    }

    fn member_completions(&self, fd: &FileData, receiver: &str, offset: usize, out: &mut Out) {
        let package = fd.package.as_str();
        let parts: Vec<&str> = receiver.split('.').collect();
        let head = parts[0];
        // A local binding shadows an import or type of the same name.
        let local = self.local_type(fd, head, offset);
        if local.is_none() {
            let import = self.imports.get(&(package.to_string(), head.to_string()));
            match (parts.len(), import) {
                (1, Some(target)) => return self.package_members(target, out),
                (2, Some(target)) => {
                    // `pkg.Enum.` lists the variants of an exported enum.
                    if let Some(s) = self.structs.get(&(target.clone(), parts[1].to_string())) {
                        if s.is_enum && s.exported {
                            for c in variants(s) {
                                out.push(c);
                            }
                        }
                    }
                    return;
                }
                _ => {}
            }
            if parts.len() == 1 {
                if let Some(s) = self.structs.get(&(package.to_string(), head.to_string())) {
                    if s.is_enum {
                        for c in variants(s) {
                            out.push(c);
                        }
                    }
                    return;
                }
            }
        }
        let ty = if parts.len() == 1 {
            local.map(|(t, _)| t).or_else(|| {
                self.consts
                    .get(&(package.to_string(), head.to_string()))
                    .map(|c| c.ty.clone())
            })
        } else {
            self.walk_chain(fd, package, &parts, offset, false)
                .map(|h| h.ty)
        };
        if let Some(ty) = ty {
            self.type_members(package, &ty, out);
        }
    }

    /// `pkg.` after an import: exported functions, globals, and types, or
    /// the signature table of a compiler-provided package.
    fn package_members(&self, target: &str, out: &mut Out) {
        let mut items: Vec<Completion> = Vec::new();
        for f in self
            .funcs
            .values()
            .filter(|f| f.package == target && f.exported)
        {
            items.push(Completion {
                label: f.name.clone(),
                kind: CompletionKind::Function,
                detail: fn_detail(f, &f.name),
                docs: f.docs.clone(),
                rank: RANK_LOCAL,
            });
        }
        for c in self
            .consts
            .values()
            .filter(|c| c.package == target && c.exported)
        {
            items.push(Completion {
                label: c.name.clone(),
                kind: CompletionKind::Constant,
                detail: format!("let {}: {}", c.name, c.ty),
                docs: c.docs.clone(),
                rank: RANK_LOCAL,
            });
        }
        for s in self
            .structs
            .values()
            .filter(|s| s.package == target && s.exported)
        {
            items.push(type_item(s, RANK_LOCAL));
        }
        if items.is_empty() && is_native_package(target) {
            for sig in NATIVE_SIGS.iter().filter(|s| s.pkg == target) {
                let mut detail = format!("fn {}({})", sig.name, sig.params.join(", "));
                if let Some(r) = sig.ret {
                    detail.push_str(&format!(" -> {r}"));
                }
                items.push(Completion {
                    label: sig.name.to_string(),
                    kind: CompletionKind::Function,
                    detail,
                    docs: String::new(),
                    rank: RANK_LOCAL,
                });
            }
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        for c in items {
            out.push(c);
        }
    }

    /// Locate the struct a value type names. `alias.Type` resolves through
    /// the import table. An unqualified name not declared in `package` is
    /// a type returned from another package (inference keeps the bare
    /// name); it is accepted only when exactly one package declares it,
    /// so an ambiguous name yields nothing rather than wrong members.
    fn find_struct(&self, package: &str, ty: &str) -> Option<&StructSym> {
        let base = generic_base(ty.trim());
        if let Some((alias, name)) = base.split_once('.') {
            let target = self
                .imports
                .get(&(package.to_string(), alias.to_string()))
                .map_or(alias, String::as_str);
            return self.structs.get(&(target.to_string(), name.to_string()));
        }
        if let Some(s) = self.structs.get(&(package.to_string(), base.to_string())) {
            return Some(s);
        }
        let mut found = self.structs.values().filter(|s| s.name == base);
        match (found.next(), found.next()) {
            (Some(s), None) => Some(s),
            _ => None,
        }
    }

    /// Fields and methods of a value's type.
    fn type_members(&self, package: &str, ty: &str, out: &mut Out) {
        let Some(s) = self.find_struct(package, ty) else {
            return;
        };
        if s.is_enum {
            // Variants are reached through the type (`Status.Paid`), not
            // through a value of it.
            return;
        }
        for (name, fty, _) in &s.fields {
            out.push(Completion {
                label: name.clone(),
                kind: CompletionKind::Field,
                detail: fty.clone(),
                docs: String::new(),
                rank: RANK_LOCAL,
            });
        }
        let mut methods: Vec<Completion> = self
            .methods
            .iter()
            .filter(|((pkg, owner, _), _)| *pkg == s.package && *owner == s.name)
            .map(|((_, _, m), f)| Completion {
                label: m.clone(),
                kind: CompletionKind::Method,
                detail: fn_detail(f, m),
                docs: f.docs.clone(),
                rank: RANK_LOCAL,
            })
            .collect();
        methods.sort_by(|a, b| a.label.cmp(&b.label));
        for c in methods {
            out.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_detect_member_context_after_dot() {
        let src = "let x = a.b.c";
        assert_eq!(
            context_at(src, src.len()),
            Context::Member {
                receiver: "a.b".to_string()
            }
        );
        let src = "p.";
        assert_eq!(
            context_at(src, src.len()),
            Context::Member {
                receiver: "p".to_string()
            }
        );
    }

    #[test]
    fn should_return_nothing_inside_strings_and_comments() {
        let src = "let s = \"a.b";
        assert_eq!(context_at(src, src.len()), Context::Nothing);
        let src = "x = 1; // see p.";
        assert_eq!(context_at(src, src.len()), Context::Nothing);
        let src = "let s = \"esc \\\" still in\" + p.";
        assert_eq!(
            context_at(src, src.len()),
            Context::Member {
                receiver: "p".to_string()
            }
        );
    }

    #[test]
    fn should_return_nothing_when_declaring_a_name() {
        let src = "    let ne";
        assert_eq!(context_at(src, src.len()), Context::Nothing);
        let src = "fn ";
        assert_eq!(context_at(src, src.len()), Context::Nothing);
    }

    #[test]
    fn should_return_nothing_for_unresolvable_receivers() {
        for src in ["f().", "a[0].", "f().x.", "1."] {
            assert_eq!(context_at(src, src.len()), Context::Nothing, "{src}");
        }
    }

    #[test]
    fn should_treat_range_dots_as_scope() {
        let src = "for i in 0..n";
        assert_eq!(context_at(src, src.len()), Context::Scope);
        let src = "for i in 0..p.";
        assert_eq!(
            context_at(src, src.len()),
            Context::Member {
                receiver: "p".to_string()
            }
        );
    }

    use std::path::PathBuf;

    const SRC: &str = "// A 2D point.
struct Point {
    x: int,
    y: int,
}

impl Point {
    // Sum of both axes.
    fn sum(self: Point) -> int {
        return self.x + self.y;
    }
}

enum Status {
    Pending,
    Paid,
}

fn helper(n: int) -> str {
    return to_str(n);
}

fn run(count: int) -> int {
    let p = Point { x: 1, y: 2 };
    if count > 0 {
        let inner = \"s\";
        let count = \"shadowed\";
        @A
    }
    @B
    return p.sum();
}
";

    /// `SRC` with `text` typed at `marker` (other markers removed), and
    /// the byte offset where `text` ends: the cursor.
    fn typed(marker: &str, text: &str) -> (String, usize) {
        let other = if marker == "@A" { "@B" } else { "@A" };
        let base = SRC.replace(other, "");
        let cursor = base.find(marker).unwrap() + text.len();
        (base.replace(marker, text), cursor)
    }

    /// Completions after typing `text` at `marker`, indexed and queried on
    /// the same buffer (the editor's normal keystroke flow).
    fn complete(marker: &str, text: &str) -> Vec<Completion> {
        let (src, cursor) = typed(marker, text);
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, &src).unwrap();
        db.completions(&path, &src, cursor)
    }

    fn labels(items: &[Completion]) -> Vec<&str> {
        items.iter().map(|c| c.label.as_str()).collect()
    }

    fn find<'a>(items: &'a [Completion], label: &str) -> &'a Completion {
        items
            .iter()
            .find(|c| c.label == label)
            .unwrap_or_else(|| panic!("no completion {label} in {:?}", labels(items)))
    }

    #[test]
    fn should_offer_locals_params_and_package_items_in_nested_block() {
        let items = complete("@A", "");
        for want in [
            "p", "inner", "count", "helper", "run", "Point", "Status", "println",
        ] {
            find(&items, want);
        }
        assert_eq!(find(&items, "p").detail, "Point");
        assert_eq!(find(&items, "inner").kind, CompletionKind::Variable);
        assert_eq!(find(&items, "helper").detail, "fn helper(n: int) -> str");
        assert_eq!(find(&items, "let").kind, CompletionKind::Keyword);
    }

    #[test]
    fn should_let_inner_binding_shadow_parameter() {
        let items = complete("@A", "");
        let count: Vec<_> = items.iter().filter(|c| c.label == "count").collect();
        assert_eq!(count.len(), 1);
        assert_eq!(count[0].detail, "str");
    }

    #[test]
    fn should_not_offer_block_locals_after_block_ends() {
        let items = complete("@B", "");
        assert!(!labels(&items).contains(&"inner"));
        assert_eq!(find(&items, "count").detail, "int");
    }

    #[test]
    fn should_offer_fields_and_methods_after_dot() {
        let items = complete("@B", "p.");
        assert_eq!(labels(&items), vec!["x", "y", "sum"]);
        assert_eq!(find(&items, "x").kind, CompletionKind::Field);
        let sum = find(&items, "sum");
        assert_eq!(sum.kind, CompletionKind::Method);
        assert_eq!(sum.detail, "fn sum(self: Point) -> int");
        assert_eq!(sum.docs, "Sum of both axes.");
    }

    #[test]
    fn should_offer_members_of_self_inside_method() {
        let src = SRC
            .replace("return self.x + self.y;", "return self.;")
            .replace("@A", "")
            .replace("@B", "");
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, &src).unwrap();
        let cur = src.find("self.;").unwrap() + 5;
        assert_eq!(
            labels(&db.completions(&path, &src, cur)),
            vec!["x", "y", "sum"]
        );
    }

    #[test]
    fn should_offer_enum_variants_after_type_name() {
        let items = complete("@B", "Status.");
        assert_eq!(labels(&items), vec!["Pending", "Paid"]);
        assert_eq!(items[0].kind, CompletionKind::EnumMember);
    }

    #[test]
    fn should_offer_nothing_inside_string_or_comment() {
        // An unclosed string does not lex, so the index keeps the last
        // good buffer; the live text still decides the context.
        let (good, _) = typed("@B", "");
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, &good).unwrap();
        let (live, cur) = typed("@B", "let s = \"p.");
        assert!(db.completions(&path, &live, cur).is_empty());
        let (live, cur) = typed("@B", "// p.");
        assert!(db.completions(&path, &live, cur).is_empty());
    }

    #[test]
    fn should_offer_native_package_functions() {
        let src = "import \"time\";\nfn f() {\n    time.\n}\n";
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, src).unwrap();
        let cur = src.find("time.\n").unwrap() + 5;
        let items = db.completions(&path, src, cur);
        let sleep = find(&items, "sleep");
        assert_eq!(sleep.kind, CompletionKind::Function);
        assert!(sleep.detail.starts_with("fn sleep("), "{}", sleep.detail);
        assert!(items.iter().all(|c| c.kind == CompletionKind::Function));
    }

    #[test]
    fn should_not_offer_entry_top_level_let_inside_function() {
        let src = "let LIMIT = 10;\nfn f() -> int {\n    \n}\n";
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, src).unwrap();
        let cur = src.find("    \n").unwrap() + 4;
        assert!(!labels(&db.completions(&path, src, cur)).contains(&"LIMIT"));
    }

    #[test]
    fn should_offer_only_exported_items_of_imported_package() {
        let root = std::env::temp_dir().join(format!(
            "slanganalyser-complete-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let lib = root.join("geo");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(root.join("slang.project"), "name app\nversion 0.1.0\n").unwrap();
        std::fs::write(
            lib.join("geo.sl"),
            "// Distance between points.\npub fn dist(a: int, b: int) -> int {\n    return a - b;\n}\nfn hidden() {}\npub let ORIGIN = 0;\nlet SECRET = 1;\npub enum Axis {\n    X,\n    Y,\n}\nenum Private {\n    A,\n}\n",
        )
        .unwrap();
        let main = root.join("main.sl");
        let src = "import \"geo\";\nfn f() {\n    geo.\n}\n";
        std::fs::write(&main, src).unwrap();
        let db = Db::load(&root).unwrap();
        let cur = src.find("geo.\n").unwrap() + 4;
        let items = db.completions(&main, src, cur);
        assert_eq!(labels(&items), vec!["Axis", "ORIGIN", "dist"]);
        assert_eq!(find(&items, "dist").docs, "Distance between points.");
        // The import binding itself is offered in scope position.
        let scope = db.completions(&main, src, cur - 4);
        assert_eq!(find(&scope, "geo").kind, CompletionKind::Module);
        // `geo.Axis.` lists the exported enum's variants.
        let src2 = "import \"geo\";\nfn f() {\n    geo.Axis.\n}\n";
        let mut db = db;
        db.update_file(&main, src2.to_string());
        let cur2 = src2.find("Axis.\n").unwrap() + 5;
        assert_eq!(labels(&db.completions(&main, src2, cur2)), vec!["X", "Y"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn should_complete_in_half_typed_buffer_without_panicking() {
        // Unbalanced braces and a dangling dot: the parser recovers and
        // completion still sees the binding before the cursor.
        let src =
            "struct P { x: int }\nfn f() {\n    let q = P { x: 1 };\n    if q.x > 0 {\n        q.";
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, src).unwrap();
        assert_eq!(labels(&db.completions(&path, src, src.len())), vec!["x"]);
        for off in 0..=src.len() + 3 {
            let _ = db.completions(&path, src, off);
        }
    }
}
