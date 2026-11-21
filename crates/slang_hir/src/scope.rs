//! Lexical scopes: which local bindings are visible at a byte offset.
//!
//! One walker serves both name lookup (hover / goto-definition / type
//! inference) and completion, so the two can never disagree about what
//! is in scope. Rules mirror slangc (`src/codegen/stmt.c` scoping, and
//! `src/codegen/program.c` `emit_globals` for top-level `let`s, checked
//! against slang `ec4a417f`):
//!
//! - A binding is never visible inside its own initializer: in
//!   `let x = x.y;` the right-hand `x` is the *previous* `x`. Besides
//!   being the language rule, this makes type inference terminate: every
//!   lookup made while inferring an initializer lands strictly earlier in
//!   the file, so self-referential `let`s can no longer recurse forever.
//! - Block-scoped names (`if`/`while`/`for` bodies, `guard`/`if let`
//!   bindings, `select` case binds) are visible only inside their block,
//!   at any nesting depth.
//! - Top-level `let`s of the entry (root) package are locals of `main()`:
//!   functions cannot see them. Top-level `let`s of any other package are
//!   package globals, visible inside every function of that package.

use crate::{bracket_content, fn_end, map_value, type_string, unwrap_single, Db, FileData};
use slang_syntax::{Block, Else, Expr, FnDecl, Item, Span, Stmt, SwitchBody, Type};

/// Where a binding's type comes from. Resolved lazily so that a lookup
/// by name only infers the one binding it returns.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TypeSrc<'a> {
    /// Explicit annotation (`let x: T`, parameters).
    Declared(&'a Type),
    /// Unannotated `let x = init` (also `else let e = err_of(x)`).
    Init(&'a Expr),
    /// Fixed by the construct (`for i in 0..n` binds `int`).
    Fixed(&'static str),
    /// `guard let` / `if let`: the `T` of `opt[T]` / `result[T, E]`.
    Unwrap(&'a Expr),
    /// First `for` name: the element (or map key) of the iterable.
    Elem(&'a Expr),
    /// Second `for` name over a map: the value type.
    MapVal(&'a Expr),
    /// `select` receive bind: the element type of the channel.
    ChanElem(&'a Expr),
}

#[derive(Debug, Clone)]
pub(crate) struct Binding<'a> {
    pub name: &'a str,
    /// 1-based definition line.
    pub line: u32,
    pub src: TypeSrc<'a>,
}

/// How the visibility window is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Name lookup: also counts the declaration site itself (the name and
    /// annotation before the initializer), so hovering `x` in `let x = 1`
    /// resolves.
    Lookup,
    /// Completion: only names already usable at the cursor.
    Complete,
}

fn end(span: &Span) -> usize {
    span.offset + span.len
}

fn within(span: &Span, offset: usize) -> bool {
    span.offset <= offset && offset <= end(span)
}

/// In scope at the declaration site: from the statement start up to (not
/// including) the expression the binding is computed from.
fn at_decl_site(mode: Mode, stmt: &Span, rhs: &Expr, offset: usize) -> bool {
    mode == Mode::Lookup && stmt.offset <= offset && offset < rhs.span.offset
}

pub(crate) fn stmt_span(s: &Stmt) -> &Span {
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
        | Stmt::Unsafe { span, .. } => span,
    }
}

impl Db {
    /// True when `fd` belongs to the entry package (the workspace root
    /// directory), whose top-level `let`s are `main()` locals.
    pub(crate) fn is_entry_file(&self, fd: &FileData) -> bool {
        fd.path.parent() == Some(self.root.as_path())
    }

    /// The function or method whose body contains `offset`, if any.
    pub(crate) fn enclosing_fn<'a>(&self, fd: &'a FileData, offset: usize) -> Option<&'a FnDecl> {
        let contains = |f: &FnDecl| f.span.offset <= offset && offset <= fn_end(f);
        for item in &fd.file.items {
            match item {
                Item::Fn(f) if contains(f) => return Some(f),
                Item::Impl(i) => {
                    if let Some(m) = i.methods.iter().find(|m| contains(m)) {
                        return Some(m);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Every local binding visible at `offset`, outermost first. When a
    /// name appears more than once, the last entry is the one in effect
    /// (inner scopes and later `let`s shadow earlier ones).
    pub(crate) fn bindings_at<'a>(
        &self,
        fd: &'a FileData,
        offset: usize,
        mode: Mode,
    ) -> Vec<Binding<'a>> {
        let mut out = Vec::new();
        let enclosing = self.enclosing_fn(fd, offset);
        let library = !self.is_entry_file(fd);
        for item in &fd.file.items {
            let Item::Let {
                name,
                ty,
                init,
                span,
                ..
            } = item
            else {
                continue;
            };
            let declared_before = end(span) <= offset;
            let visible = match enclosing {
                // Globals are visible in every function of a library
                // package. Inference of a global's own initializer runs at
                // top level, where only earlier globals count, so globals
                // cannot form a lookup cycle.
                Some(_) => library,
                None => declared_before || at_decl_site(mode, span, init, offset),
            };
            if visible {
                out.push(Binding {
                    name,
                    line: span.line,
                    src: ty.as_ref().map_or(TypeSrc::Init(init), TypeSrc::Declared),
                });
            }
        }
        match enclosing {
            Some(f) => {
                for p in &f.params {
                    out.push(Binding {
                        name: &p.name,
                        line: p.span.line,
                        src: TypeSrc::Declared(&p.ty),
                    });
                }
                if let Some(body) = &f.body {
                    walk_stmts(&body.stmts, offset, mode, &mut out);
                }
            }
            None => walk_stmts(&fd.file.body, offset, mode, &mut out),
        }
        out
    }
}

fn walk_block<'a>(b: &'a Block, offset: usize, mode: Mode, out: &mut Vec<Binding<'a>>) {
    if within(&b.span, offset) {
        walk_stmts(&b.stmts, offset, mode, out);
    }
}

fn walk_stmts<'a>(stmts: &'a [Stmt], offset: usize, mode: Mode, out: &mut Vec<Binding<'a>>) {
    for s in stmts {
        // Statements are in source order: nothing past the cursor binds.
        if stmt_span(s).offset > offset {
            break;
        }
        walk_stmt(s, offset, mode, out);
    }
}

fn walk_stmt<'a>(s: &'a Stmt, offset: usize, mode: Mode, out: &mut Vec<Binding<'a>>) {
    match s {
        Stmt::Let {
            name,
            ty,
            init,
            span,
        } => {
            if end(span) <= offset || at_decl_site(mode, span, init, offset) {
                out.push(Binding {
                    name,
                    line: span.line,
                    src: ty.as_ref().map_or(TypeSrc::Init(init), TypeSrc::Declared),
                });
            }
        }
        Stmt::If { then, else_, .. } => {
            walk_block(then, offset, mode, out);
            match else_ {
                Some(Else::Block(b)) => walk_block(b, offset, mode, out),
                Some(Else::If(inner)) if within(stmt_span(inner), offset) => {
                    walk_stmt(inner, offset, mode, out);
                }
                Some(Else::If(_)) | None => {}
            }
        }
        Stmt::While { body, .. } | Stmt::Unsafe { body, .. } => walk_block(body, offset, mode, out),
        Stmt::ForRange {
            name,
            start,
            body,
            span,
            ..
        } => {
            if within(&body.span, offset) || at_decl_site(mode, span, start, offset) {
                out.push(Binding {
                    name,
                    line: span.line,
                    src: TypeSrc::Fixed("int"),
                });
            }
            walk_block(body, offset, mode, out);
        }
        Stmt::ForIn {
            name,
            name2,
            iter,
            body,
            span,
        } => {
            if within(&body.span, offset) || at_decl_site(mode, span, iter, offset) {
                out.push(Binding {
                    name,
                    line: span.line,
                    src: TypeSrc::Elem(iter),
                });
                if let Some(n2) = name2 {
                    out.push(Binding {
                        name: n2,
                        line: span.line,
                        src: TypeSrc::MapVal(iter),
                    });
                }
            }
            walk_block(body, offset, mode, out);
        }
        Stmt::GuardLet {
            name,
            expr,
            err_name,
            err_expr,
            body,
            span,
        } => {
            // The diverging else block sees the error binding only; the
            // unwrapped value is bound after the whole guard statement.
            if let (Some(en), Some(ee)) = (err_name, err_expr) {
                if within(&body.span, offset) || at_decl_site(mode, span, ee, offset) {
                    out.push(Binding {
                        name: en,
                        line: span.line,
                        src: TypeSrc::Init(ee),
                    });
                }
            }
            if within(&body.span, offset) {
                walk_stmts(&body.stmts, offset, mode, out);
            } else if end(span) <= offset || at_decl_site(mode, span, expr, offset) {
                out.push(Binding {
                    name,
                    line: span.line,
                    src: TypeSrc::Unwrap(expr),
                });
            }
        }
        Stmt::IfLet {
            name,
            expr,
            then,
            err_name,
            err_expr,
            else_,
            span,
        } => {
            if within(&then.span, offset) || at_decl_site(mode, span, expr, offset) {
                out.push(Binding {
                    name,
                    line: span.line,
                    src: TypeSrc::Unwrap(expr),
                });
                walk_block(then, offset, mode, out);
            }
            if let (Some(en), Some(ee)) = (err_name, err_expr) {
                let in_else = else_.as_ref().is_some_and(|eb| within(&eb.span, offset));
                if in_else || at_decl_site(mode, span, ee, offset) {
                    out.push(Binding {
                        name: en,
                        line: span.line,
                        src: TypeSrc::Init(ee),
                    });
                }
            }
            if let Some(eb) = else_ {
                walk_block(eb, offset, mode, out);
            }
        }
        Stmt::Select { cases, default, .. } => {
            for c in cases {
                if within(&c.body.span, offset) {
                    if let (Some(b), false) = (&c.bind, c.is_send) {
                        out.push(Binding {
                            name: b,
                            line: c.span.line,
                            src: TypeSrc::ChanElem(&c.ch),
                        });
                    }
                    walk_stmts(&c.body.stmts, offset, mode, out);
                }
            }
            if let Some(d) = default {
                walk_block(d, offset, mode, out);
            }
        }
        Stmt::Switch { cases, default, .. } => {
            for c in cases {
                if let SwitchBody::Block(b) = &c.body {
                    walk_block(b, offset, mode, out);
                }
            }
            if let Some(d) = default {
                walk_block(d, offset, mode, out);
            }
        }
        Stmt::Assign { .. }
        | Stmt::Return { .. }
        | Stmt::Break { .. }
        | Stmt::Continue { .. }
        | Stmt::Expr { .. }
        | Stmt::Spawn { .. } => {}
    }
}

impl Db {
    /// Resolve a binding's type. Inference of the binding's own expression
    /// runs at that expression, where the binding itself is not yet
    /// visible, so resolution always moves strictly earlier in the file.
    pub(crate) fn binding_type(&self, fd: &FileData, b: &Binding<'_>) -> String {
        match b.src {
            TypeSrc::Declared(t) => type_string(t),
            TypeSrc::Init(e) => self.infer_expr(fd, e, e.span.offset),
            TypeSrc::Fixed(t) => t.to_string(),
            TypeSrc::Unwrap(e) => {
                let t = self.infer_expr(fd, e, e.span.offset);
                unwrap_single(&self.chase(fd, t, b.name, e.span.offset))
            }
            TypeSrc::Elem(e) => self.iter_elem_type(fd, e, e.span.offset),
            TypeSrc::MapVal(e) => {
                let t = self.infer_expr(fd, e, e.span.offset);
                let t = self.chase(fd, t, b.name, e.span.offset);
                map_value(&t).unwrap_or_else(|| "str".to_string())
            }
            TypeSrc::ChanElem(e) => {
                let t = self.infer_expr(fd, e, e.span.offset);
                let t = self.chase(fd, t, b.name, e.span.offset);
                bracket_content(&t, "chan").unwrap_or_else(|| "{unknown}".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Db;
    use std::path::PathBuf;

    fn hover_ty(src: &str, needle: &str, delta: usize) -> Option<String> {
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, src).unwrap();
        let off = src.find(needle).unwrap() + delta;
        db.hover(&path, off).map(|h| h.ty)
    }

    #[test]
    fn should_not_overflow_stack_on_self_referential_let() {
        // Used to recurse forever (stack overflow aborts the server):
        // inferring `x.x` looked `x` up and found this very `let`.
        let src = "struct P { x: P }\nfn f() {\n    let x = x.x;\n}\n";
        assert_eq!(hover_ty(src, "let x", 4).as_deref(), Some("field"));
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, src).unwrap();
        let _ = db.inlay_hints(&path);
    }

    #[test]
    fn should_resolve_shadowed_let_from_previous_binding() {
        let src = "struct P { x: int }\nfn f(p: P) {\n    let p = p.x;\n    println(p);\n}\n";
        assert_eq!(hover_ty(src, "println(p", 8).as_deref(), Some("int"));
    }

    #[test]
    fn should_hover_let_inside_nested_block() {
        let src = "fn f(n: int) {\n    if n > 1 {\n        for i in 0..n {\n            let inner = \"s\";\n            println(inner);\n        }\n    }\n}\n";
        assert_eq!(hover_ty(src, "println(inner", 9).as_deref(), Some("str"));
        assert_eq!(hover_ty(src, "for i", 4).as_deref(), Some("int"));
    }

    #[test]
    fn should_hover_method_receiver_and_its_fields() {
        let src = "struct P { x: int }\nimpl P {\n    fn get(self: P) -> int {\n        return self.x;\n    }\n}\n";
        assert_eq!(hover_ty(src, "self.x", 1).as_deref(), Some("P"));
        assert_eq!(hover_ty(src, "self.x", 5).as_deref(), Some("int"));
    }

    #[test]
    fn should_attach_doc_comment_to_method() {
        let src = "struct P { x: int }\nimpl P {\n    // Reads x.\n    fn get(self: P) -> int {\n        return self.x;\n    }\n}\nfn f(p: P) -> int {\n    return p.get();\n}\n";
        let path = PathBuf::from("main.sl");
        let db = Db::load_single("main", &path, src).unwrap();
        let h = db.hover(&path, src.find("p.get").unwrap() + 3).unwrap();
        assert_eq!(h.docs, "Reads x.");
    }

    #[test]
    fn should_scope_guard_binding_after_statement_only() {
        let src = "fn g() -> opt[int] {\n    return none;\n}\nfn f() -> int {\n    guard let v = g() else {\n        return 0;\n    }\n    return v;\n}\n";
        assert_eq!(hover_ty(src, "return v", 7).as_deref(), Some("int"));
    }
}
