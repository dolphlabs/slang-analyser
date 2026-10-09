//! Canonical formatter for `.sl` files.
//!
//! Parses with `slang_syntax` and re-emits in one style: 4-space indent,
//! `;`-terminated statements, spaces around operators, `//` comments kept
//! on their own line or trailed on the statement they share a line with.
//! Formatting is idempotent: `format(format(x)) == format(x)`.

use slang_syntax::{
    Block, Else, EnumDecl, Expr, ExprKind, File, FnDecl, ImplDecl, Item, SelectCase, Stmt,
    StructDecl, SwitchBody, SwitchCase, Type, TypeKind, lex_with_comments, parse,
};

#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub indent_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self { indent_width: 4 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    pub message: String,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FormatError {}

struct Printer {
    out: String,
    indent: usize,
    opt: FormatOptions,
    comments: Vec<slang_syntax::Comment>,
    cursor: usize,
}

impl Printer {
    fn pad(&mut self) {
        for _ in 0..self.indent * self.opt.indent_width {
            self.out.push(' ');
        }
    }

    fn drain_leading(&mut self, line: u32) {
        while self.cursor < self.comments.len() && self.comments[self.cursor].line < line {
            let c = self.comments[self.cursor].clone();
            self.cursor += 1;
            self.pad();
            self.out.push_str(c.text.trim());
            self.out.push('\n');
        }
    }

    fn drain_trailing(&mut self, line: u32) {
        while self.cursor < self.comments.len() && self.comments[self.cursor].line == line {
            let c = self.comments[self.cursor].clone();
            self.cursor += 1;
            self.out.push(' ');
            self.out.push_str(c.text.trim());
        }
    }

    fn stmt_with_comments(&mut self, line: u32, emit: impl FnOnce(&mut Self)) {
        self.drain_leading(line);
        emit(self);
        self.drain_trailing(line);
    }
}

/// Format `.sl` source into canonical style.
pub fn format(src: &str) -> Result<String, FormatError> {
    format_with(&FormatOptions::default(), src)
}

pub fn format_with(opt: &FormatOptions, src: &str) -> Result<String, FormatError> {
    let (toks, comments) = lex_with_comments(src)
        .map_err(|e| FormatError { message: e.to_string() })?;
    let (file, errors) = parse(toks);
    if let Some(first) = errors.first() {
        return Err(FormatError {
            message: first.to_string(),
        });
    }
    let mut p = Printer {
        out: String::new(),
        indent: 0,
        opt: opt.clone(),
        comments,
        cursor: 0,
    };
    fmt_file(&mut p, &file);
    // Any leftover comments (trailing file comments) flush at column 0.
    while p.cursor < p.comments.len() {
        let c = p.comments[p.cursor].clone();
        p.cursor += 1;
        p.out.push_str(c.text.trim());
        p.out.push('\n');
    }
    Ok(p.out)
}

fn fmt_file(p: &mut Printer, f: &File) {
    for item in &f.items {
        let line = item_line(item);
        p.drain_leading(line);
        fmt_item(p, item);
        p.drain_trailing(line);
        p.out.push('\n');
    }
    if !f.items.is_empty() && !f.body.is_empty() {
        p.out.push('\n');
    }
    for (i, s) in f.body.iter().enumerate() {
        let line = stmt_line(s);
        p.drain_leading(line);
        fmt_stmt(p, s);
        p.drain_trailing(line);
        p.out.push('\n');
        let _ = i;
    }
    // Blank line between consecutive top-level statements is normalized
    // away; one item per line keeps diffs minimal for agents.
}

fn item_line(item: &Item) -> u32 {
    match item {
        Item::Import { span, .. } | Item::Link { span, .. } => span.line,
        Item::Let { span, .. } => span.line,
        Item::Fn(f) => f.span.line,
        Item::Struct(s) => s.span.line,
        Item::Impl(i) => i.span.line,
        Item::Enum(e) => e.span.line,
    }
}

fn fmt_item(p: &mut Printer, item: &Item) {
    match item {
        Item::Import { path, alias, .. } => {
            p.out.push_str(&format!("import \"{path}\""));
            if let Some(a) = alias {
                p.out.push_str(&format!(" as {a}"));
            }
            p.out.push(';');
        }
        Item::Link { lib, .. } => {
            p.out.push_str(&format!("link \"{lib}\";"));
        }
        Item::Let { pub_, name, ty, init, .. } => {
            if *pub_ {
                p.out.push_str("pub ");
            }
            p.out.push_str("let ");
            p.out.push_str(name);
            if let Some(t) = ty {
                p.out.push_str(": ");
                fmt_type(p, t);
            }
            p.out.push_str(" = ");
            fmt_expr(p, init, 0);
            p.out.push(';');
        }
        Item::Fn(f) => fmt_fn(p, f),
        Item::Struct(s) => fmt_struct(p, s),
        Item::Impl(i) => fmt_impl(p, i),
        Item::Enum(e) => fmt_enum(p, e),
    }
}

fn fmt_fn(p: &mut Printer, f: &FnDecl) {
    if f.pub_ {
        p.out.push_str("pub ");
    }
    if f.extern_ {
        p.out.push_str("extern ");
    }
    p.out.push_str("fn ");
    p.out.push_str(&f.name);
    if !f.tparams.is_empty() {
        p.out.push_str(&format!("[{}]", f.tparams.join(", ")));
    }
    p.out.push('(');
    for (i, param) in f.params.iter().enumerate() {
        if i > 0 {
            p.out.push_str(", ");
        }
        p.out.push_str(&param.name);
        p.out.push_str(": ");
        fmt_type(p, &param.ty);
    }
    p.out.push(')');
    if let Some(ret) = &f.ret {
        p.out.push_str(" -> ");
        fmt_type(p, ret);
    }
    if f.extern_ {
        p.out.push(';');
        return;
    }
    p.out.push(' ');
    if let Some(body) = &f.body {
        fmt_block(p, body);
    } else {
        p.out.push_str("{}");
    }
}

fn fmt_struct(p: &mut Printer, s: &StructDecl) {
    if s.pub_ {
        p.out.push_str("pub ");
    }
    if s.gc {
        p.out.push_str("gc ");
    }
    p.out.push_str("struct ");
    p.out.push_str(&s.name);
    if !s.tparams.is_empty() {
        p.out.push_str(&format!("[{}]", s.tparams.join(", ")));
    }
    p.out.push_str(" { ");
    for (i, f) in s.fields.iter().enumerate() {
        if i > 0 {
            p.out.push_str(", ");
        }
        p.out.push_str(&f.name);
        p.out.push_str(": ");
        fmt_type(p, &f.ty);
    }
    p.out.push_str(" }");
}

fn fmt_impl(p: &mut Printer, i: &ImplDecl) {
    p.out.push_str("impl ");
    p.out.push_str(&i.type_name);
    if !i.tparams.is_empty() {
        p.out.push_str(&format!("[{}]", i.tparams.join(", ")));
    }
    p.out.push_str(" {\n");
    p.indent += 1;
    for m in &i.methods {
        p.pad();
        fmt_fn(p, m);
        p.out.push('\n');
    }
    p.indent -= 1;
    p.pad();
    p.out.push('}');
}

fn fmt_enum(p: &mut Printer, e: &EnumDecl) {
    if e.pub_ {
        p.out.push_str("pub ");
    }
    p.out.push_str("enum ");
    p.out.push_str(&e.name);
    p.out.push_str(" { ");
    for (i, v) in e.variants.iter().enumerate() {
        if i > 0 {
            p.out.push_str(", ");
        }
        p.out.push_str(&v.name);
        if let Some(n) = v.value {
            // Only print explicit values; the parser resolves default
            // ordinals too, so re-derive: explicit iff it breaks the run.
            let expected = if i == 0 { 0 } else { i as i64 };
            if n != expected {
                p.out.push_str(&format!(" = {n}"));
            }
        }
    }
    p.out.push_str(" }");
}

fn fmt_block(p: &mut Printer, b: &Block) {
    p.out.push_str("{\n");
    p.indent += 1;
    for s in &b.stmts {
        let line = stmt_line(s);
        p.drain_leading(line);
        p.pad();
        fmt_stmt_inline(p, s);
        p.drain_trailing(line);
        p.out.push('\n');
    }
    p.indent -= 1;
    p.pad();
    p.out.push('}');
}

fn stmt_line(s: &Stmt) -> u32 {
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
        | Stmt::Unsafe { span, .. } => span.line,
    }
}

fn fmt_stmt(p: &mut Printer, s: &Stmt) {
    p.pad();
    fmt_stmt_inline(p, s);
}

fn fmt_stmt_inline(p: &mut Printer, s: &Stmt) {
    match s {
        Stmt::Let { name, ty, init, .. } => {
            p.out.push_str("let ");
            p.out.push_str(name);
            if let Some(t) = ty {
                p.out.push_str(": ");
                fmt_type(p, t);
            }
            p.out.push_str(" = ");
            fmt_expr(p, init, 0);
            p.out.push(';');
        }
        Stmt::Assign { target, op, value, .. } => {
            fmt_expr(p, target, 0);
            p.out.push_str(&format!(" {op} "));
            fmt_expr(p, value, 0);
            p.out.push(';');
        }
        Stmt::If { cond, then, else_, .. } => {
            p.out.push_str("if ");
            fmt_expr(p, cond, 0);
            p.out.push(' ');
            fmt_block(p, then);
            match else_ {
                Some(Else::Block(b)) => {
                    p.out.push_str(" else ");
                    fmt_block(p, b);
                }
                Some(Else::If(nested)) => {
                    p.out.push_str(" else ");
                    fmt_stmt_inline(p, nested);
                }
                None => {}
            }
        }
        Stmt::While { cond, body, .. } => {
            p.out.push_str("while ");
            fmt_expr(p, cond, 0);
            p.out.push(' ');
            fmt_block(p, body);
        }
        Stmt::ForRange { name, start, end, inclusive, body, .. } => {
            p.out.push_str(&format!("for {name} in "));
            fmt_expr(p, start, 0);
            p.out.push_str(if *inclusive { "..=" } else { ".." });
            fmt_expr(p, end, 0);
            p.out.push(' ');
            fmt_block(p, body);
        }
        Stmt::ForIn { name, name2, iter, body, .. } => {
            p.out.push_str(&format!("for {name}"));
            if let Some(n2) = name2 {
                p.out.push_str(&format!(", {n2}"));
            }
            p.out.push_str(" in ");
            fmt_expr(p, iter, 0);
            p.out.push(' ');
            fmt_block(p, body);
        }
        Stmt::Return { value, .. } => {
            p.out.push_str("return");
            if let Some(v) = value {
                p.out.push(' ');
                fmt_expr(p, v, 0);
            }
            p.out.push(';');
        }
        Stmt::Break { .. } => p.out.push_str("break;"),
        Stmt::Continue { .. } => p.out.push_str("continue;"),
        Stmt::Expr { expr, .. } => {
            fmt_expr(p, expr, 0);
            p.out.push(';');
        }
        Stmt::GuardLet { name, expr, err_name, err_expr, body, .. } => {
            p.out.push_str(&format!("guard let {name} = "));
            fmt_expr(p, expr, 0);
            p.out.push_str(" else");
            if let Some(e) = err_name {
                p.out.push_str(&format!(" let {e} = "));
                if let Some(ee) = err_expr {
                    fmt_expr(p, ee, 0);
                }
                p.out.push(' ');
            } else {
                p.out.push(' ');
            }
            fmt_block(p, body);
        }
        Stmt::Spawn { call, .. } => {
            p.out.push_str("spawn ");
            fmt_expr(p, call, 0);
            p.out.push(';');
        }
        Stmt::IfLet { name, expr, then, err_name, err_expr, else_, .. } => {
            p.out.push_str(&format!("if let {name} = "));
            fmt_expr(p, expr, 0);
            p.out.push(' ');
            fmt_block(p, then);
            if err_name.is_some() || else_.is_some() {
                p.out.push_str(" else");
                if let Some(e) = err_name {
                    p.out.push_str(&format!(" let {e} = "));
                    if let Some(ee) = err_expr {
                        fmt_expr(p, ee, 0);
                    }
                    p.out.push(' ');
                } else {
                    p.out.push(' ');
                }
                if let Some(b) = else_ {
                    fmt_block(p, b);
                }
            }
        }
        Stmt::Select { cases, default, .. } => {
            p.out.push_str("select {\n");
            p.indent += 1;
            for c in cases {
                fmt_select_case(p, c);
            }
            if let Some(d) = default {
                p.pad();
                p.out.push_str("default ");
                fmt_block(p, d);
                p.out.push('\n');
            }
            p.indent -= 1;
            p.pad();
            p.out.push('}');
        }
        Stmt::Switch { scrut, cases, default, .. } => {
            p.out.push_str("switch ");
            fmt_expr(p, scrut, 0);
            p.out.push_str(" {\n");
            p.indent += 1;
            for c in cases {
                fmt_switch_case_block(p, c);
            }
            if let Some(d) = default {
                p.pad();
                p.out.push_str("default ");
                fmt_block(p, d);
                p.out.push('\n');
            }
            p.indent -= 1;
            p.pad();
            p.out.push('}');
        }
        Stmt::Unsafe { body, .. } => {
            p.out.push_str("unsafe ");
            fmt_block(p, body);
        }
    }
}

fn fmt_select_case(p: &mut Printer, c: &SelectCase) {
    p.pad();
    if c.is_send {
        p.out.push_str("case ");
        fmt_expr(p, &c.ch, 0);
        if let Some(v) = &c.val {
            p.out.push_str(", ");
            fmt_expr(p, v, 0);
        }
    } else {
        p.out.push_str("case let ");
        p.out.push_str(c.bind.as_deref().unwrap_or("_"));
        p.out.push_str(" = ");
        fmt_expr(p, &c.ch, 0);
    }
    p.out.push(' ');
    fmt_block(p, &c.body);
    p.out.push('\n');
}

fn fmt_switch_case_block(p: &mut Printer, c: &SwitchCase) {
    p.pad();
    p.out.push_str("case ");
    for (i, v) in c.vals.iter().enumerate() {
        if i > 0 {
            p.out.push_str(", ");
        }
        fmt_expr(p, v, 0);
    }
    p.out.push(' ');
    match &c.body {
        SwitchBody::Block(b) => fmt_block(p, b),
        SwitchBody::Expr(e) => {
            p.out.push_str("{ ");
            fmt_expr(p, e, 0);
            p.out.push_str("; }");
        }
    }
    p.out.push('\n');
}

fn fmt_type(p: &mut Printer, t: &Type) {
    match &t.kind {
        TypeKind::Named(n) => p.out.push_str(n),
        TypeKind::Array(i) => {
            p.out.push('[');
            fmt_type(p, i);
            p.out.push(']');
        }
        TypeKind::Map { key, val } => {
            p.out.push_str("map[");
            fmt_type(p, key);
            p.out.push(']');
            fmt_type(p, val);
        }
        TypeKind::Opt(i) => {
            p.out.push_str("opt[");
            fmt_type(p, i);
            p.out.push(']');
        }
        TypeKind::Result { ok, err } => {
            p.out.push_str("result[");
            fmt_type(p, ok);
            p.out.push_str(", ");
            fmt_type(p, err);
            p.out.push(']');
        }
        TypeKind::Chan(i) => {
            p.out.push_str("chan[");
            fmt_type(p, i);
            p.out.push(']');
        }
        TypeKind::Join(i) => {
            p.out.push_str("join[");
            fmt_type(p, i);
            p.out.push(']');
        }
        TypeKind::Ref(i) => {
            p.out.push('&');
            fmt_type(p, i);
        }
        TypeKind::RefMut(i) => {
            p.out.push_str("&mut ");
            fmt_type(p, i);
        }
        TypeKind::Own(i) => {
            p.out.push_str("own ");
            fmt_type(p, i);
        }
        TypeKind::Gc(i) => {
            p.out.push_str("gc ");
            fmt_type(p, i);
        }
        TypeKind::Ptr(i) => {
            p.out.push('*');
            fmt_type(p, i);
        }
        TypeKind::Fn { params, ret } => {
            p.out.push_str("fn(");
            for (i, q) in params.iter().enumerate() {
                if i > 0 {
                    p.out.push_str(", ");
                }
                fmt_type(p, q);
            }
            p.out.push(')');
            if let Some(r) = ret {
                p.out.push_str(" -> ");
                fmt_type(p, r);
            }
        }
    }
}

/// Precedence levels mirroring `parse_*` (higher binds tighter).
fn precedence(op: &str) -> u8 {
    match op {
        "??" => 1,
        "||" => 2,
        "&&" => 3,
        "|" => 4,
        "^" => 5,
        "&" => 6,
        "==" | "!=" => 7,
        "<" | ">" | "<=" | ">=" => 8,
        "<<" | ">>" => 9,
        ".." | "..=" => 10,
        "+" | "-" => 11,
        "*" | "/" | "%" => 12,
        _ => 0,
    }
}

fn fmt_expr(p: &mut Printer, e: &Expr, parent_prec: u8) {
    match &e.kind {
        ExprKind::Int { value, big_u64 } => {
            if *big_u64 {
                p.out.push_str(&(*value as u64).to_string());
            } else {
                p.out.push_str(&value.to_string());
            }
        }
        ExprKind::Float(v) => p.out.push_str(&v.to_string()),
        ExprKind::String(s) => fmt_string(p, s),
        ExprKind::Bytes(b) => fmt_bytes(p, b),
        ExprKind::Bool(b) => p.out.push_str(if *b { "true" } else { "false" }),
        ExprKind::Ident(n) => p.out.push_str(n),
        ExprKind::Binary { op, lhs, rhs } => {
            let prec = precedence(op);
            let need = prec < parent_prec;
            if need {
                p.out.push('(');
            }
            fmt_expr(p, lhs, prec);
            p.out.push_str(&format!(" {op} "));
            fmt_expr(p, rhs, prec + 1);
            if need {
                p.out.push(')');
            }
        }
        ExprKind::Unary { op, operand } => {
            p.out.push_str(op);
            fmt_expr(p, operand, 13);
        }
        ExprKind::Call { name, callee, args } => {
            if let Some(n) = name {
                p.out.push_str(n);
            } else if let Some(c) = callee {
                fmt_expr(p, c, 14);
            }
            p.out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    p.out.push_str(", ");
                }
                fmt_expr(p, a, 0);
            }
            p.out.push(')');
        }
        ExprKind::Cast { operand, ty } => {
            fmt_expr(p, operand, 13);
            p.out.push_str(" as ");
            fmt_type(p, ty);
        }
        ExprKind::Index { base, index } => {
            fmt_expr(p, base, 14);
            p.out.push('[');
            fmt_expr(p, index, 0);
            p.out.push(']');
        }
        ExprKind::Slice { base, start, end, inclusive } => {
            fmt_expr(p, base, 14);
            p.out.push('[');
            if let Some(s) = start {
                fmt_expr(p, s, 0);
            }
            p.out.push_str(if *inclusive { "..=" } else { ".." });
            if let Some(e) = end {
                fmt_expr(p, e, 0);
            }
            p.out.push(']');
        }
        ExprKind::List(items) => {
            p.out.push('[');
            for (i, e) in items.iter().enumerate() {
                if i > 0 {
                    p.out.push_str(", ");
                }
                fmt_expr(p, e, 0);
            }
            p.out.push(']');
        }
        ExprKind::MapLit(pairs) => {
            p.out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    p.out.push_str(", ");
                }
                fmt_expr(p, k, 0);
                p.out.push_str(": ");
                fmt_expr(p, v, 0);
            }
            p.out.push('}');
        }
        ExprKind::Field { base, name } => {
            fmt_expr(p, base, 14);
            p.out.push('.');
            p.out.push_str(name);
        }
        ExprKind::MethodCall { recv, name, args } => {
            fmt_expr(p, recv, 14);
            p.out.push('.');
            p.out.push_str(name);
            p.out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    p.out.push_str(", ");
                }
                fmt_expr(p, a, 0);
            }
            p.out.push(')');
        }
        ExprKind::StructLit { tyname, fields } => {
            p.out.push_str(tyname);
            p.out.push_str(" { ");
            for (i, (n, v)) in fields.iter().enumerate() {
                if i > 0 {
                    p.out.push_str(", ");
                }
                p.out.push_str(n);
                p.out.push_str(": ");
                fmt_expr(p, v, 0);
            }
            p.out.push_str(" }");
        }
        ExprKind::Spawn(call) => {
            p.out.push_str("spawn ");
            fmt_expr(p, call, 0);
        }
        ExprKind::Switch { scrut, cases, default } => {
            p.out.push_str("switch ");
            fmt_expr(p, scrut, 0);
            p.out.push_str(" { ");
            for c in cases {
                p.out.push_str("case ");
                for (i, v) in c.vals.iter().enumerate() {
                    if i > 0 {
                        p.out.push_str(", ");
                    }
                    fmt_expr(p, v, 0);
                }
                p.out.push_str(" { ");
                match &c.body {
                    SwitchBody::Expr(e) => fmt_expr(p, e, 0),
                    SwitchBody::Block(_) => p.out.push_str("..."),
                }
                p.out.push_str(" } ");
            }
            if let Some(d) = default {
                p.out.push_str("default { ");
                if let SwitchBody::Expr(e) = &**d {
                    fmt_expr(p, e, 0);
                }
                p.out.push_str(" } ");
            }
            p.out.push('}');
        }
    }
}

fn fmt_string(p: &mut Printer, s: &str) {
    p.out.push('"');
    // Decoded strings carry interpolations between \x01 markers.
    let mut in_interp = false;
    for chunk in s.split('\x01') {
        if in_interp {
            p.out.push_str("${");
            p.out.push_str(chunk);
            p.out.push('}');
        } else {
            for c in chunk.chars() {
                match c {
                    '"' => p.out.push_str("\\\""),
                    '\\' => p.out.push_str("\\\\"),
                    '\n' => p.out.push_str("\\n"),
                    '\t' => p.out.push_str("\\t"),
                    '\r' => p.out.push_str("\\r"),
                    '\0' => p.out.push_str("\\0"),
                    _ => p.out.push(c),
                }
            }
        }
        in_interp = !in_interp;
    }
    p.out.push('"');
}

fn fmt_bytes(p: &mut Printer, b: &[u8]) {
    p.out.push_str("b\"");
    for &c in b {
        match c {
            b'"' => p.out.push_str("\\\""),
            b'\\' => p.out.push_str("\\\\"),
            10 => p.out.push_str("\\n"),
            9 => p.out.push_str("\\t"),
            13 => p.out.push_str("\\r"),
            0 => p.out.push_str("\\0"),
            32..=126 => p.out.push(c as char),
            _ => p.out.push_str(&format!("\\x{c:02x}")),
        }
    }
    p.out.push('"');
}

// Re-exported for the LSP layer: statement span lines.
pub use slang_syntax::Span;

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        format(src).unwrap()
    }

    #[test]
    fn should_format_hello() {
        let out = fmt("fn fib(n:int)->int{return n;}\n");
        assert_eq!(out, "fn fib(n: int) -> int {\n    return n;\n}\n");
    }

    #[test]
    fn should_be_idempotent() {
        let once = fmt("let   x=1+2*3;\nfor k in 1..=3{println(\"tick ${k}\");}\n");
        let twice = format(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn should_keep_comments() {
        let out = fmt("// top\nlet x = 1; // trailing\n");
        assert!(out.contains("// top"));
        assert!(out.contains("// trailing"));
    }

    #[test]
    fn should_reject_broken_code() {
        assert!(format("fn broken( {").is_err());
    }
}
