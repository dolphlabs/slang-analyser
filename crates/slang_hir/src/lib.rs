//! Name resolution + type inference over a `slang.project` workspace.
//!
//! Loads every `.sl` file under the project root (each directory with
//! `.sl` files is a package, mirroring `src/loader.c`), indexes `pub`
//! functions / structs / enums / methods with their doc comments, then
//! answers hover (`inferred type + docs`) and goto-definition queries.
//! Types mirror `src/codegen/infer.c` only far enough for IDE answers:
//! scalars, `[T]`, `map[K]V`, `opt[T]`, `result[T, E]`, structs, and
//! function signatures.

use slang_syntax::{
    lex_with_comments, parse, Block, Else, Expr, ExprKind, File, Item, Stmt, SwitchBody, Type,
    TypeKind,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileId(usize);

#[derive(Debug, Clone)]
struct FileData {
    path: PathBuf,
    package: String,
    src: String,
    file: File,
}

#[derive(Debug, Clone)]
pub struct FuncSym {
    pub package: String,
    pub name: String,
    pub file: PathBuf,
    pub line: u32,
    pub params: Vec<(String, String)>,
    pub ret: Option<String>,
    pub docs: String,
}

#[derive(Debug, Clone)]
pub struct StructSym {
    pub package: String,
    pub name: String,
    pub file: PathBuf,
    pub line: u32,
    pub fields: Vec<(String, String)>,
    pub docs: String,
}

#[derive(Debug, Clone)]
pub struct InlayHint {
    /// Byte offset where the hint renders (end of the binding name).
    pub offset: usize,
    pub label: String,
}

/// Only hint when inference produced something more useful than the
/// initializer's own text (a bare identifier or opaque call/field).
fn is_concrete_hint(label: &str, init: &Expr) -> bool {
    match &init.kind {
        ExprKind::Ident(n) => label != n,
        ExprKind::Call { .. } | ExprKind::Field { .. } => false,
        _ => true,
    }
}

fn collect_hints(file: &File, body: &Block, out: &mut Vec<InlayHint>) {
    for s in &body.stmts {
        stmt_hints(file, s, out);
    }
}

fn stmt_hints(file: &File, s: &Stmt, out: &mut Vec<InlayHint>) {
    match s {
        Stmt::Let {
            name,
            ty,
            init,
            span,
        } if ty.is_none() => {
            let label = infer_expr(file, init);
            if is_concrete_hint(&label, init) {
                out.push(InlayHint {
                    offset: span.offset + 4 + name.len(),
                    label: format!(": {label}"),
                });
            }
        }
        Stmt::If { then, else_, .. } => {
            collect_hints(file, then, out);
            if let Some(Else::Block(b)) = else_ {
                collect_hints(file, b, out);
            }
        }
        Stmt::While { body, .. }
        | Stmt::ForRange { body, .. }
        | Stmt::ForIn { body, .. }
        | Stmt::Unsafe { body, .. } => {
            collect_hints(file, body, out);
        }
        Stmt::GuardLet { body, .. } => collect_hints(file, body, out),
        Stmt::IfLet { then, else_, .. } => {
            collect_hints(file, then, out);
            if let Some(b) = else_ {
                collect_hints(file, b, out);
            }
        }
        Stmt::Select { cases, default, .. } => {
            for c in cases {
                collect_hints(file, &c.body, out);
            }
            if let Some(d) = default {
                collect_hints(file, d, out);
            }
        }
        Stmt::Switch { cases, default, .. } => {
            for c in cases {
                if let SwitchBody::Block(b) = &c.body {
                    collect_hints(file, b, out);
                }
            }
            if let Some(d) = default {
                collect_hints(file, d, out);
            }
        }
        _ => {}
    }
}
#[derive(Debug, Clone)]
pub struct Location {
    pub file: PathBuf,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Hover {
    pub symbol: String,
    pub ty: String,
    pub docs: String,
    pub location: Location,
}

pub struct Db {
    files: Vec<FileData>,
    file_index: HashMap<PathBuf, FileId>,
    funcs: HashMap<(String, String), FuncSym>,
    structs: HashMap<(String, String), StructSym>,
    /// (package, type, method) -> function symbol (methods share fn table shape).
    methods: HashMap<(String, String, String), FuncSym>,
    /// (package, alias) -> target package for `import "x" [as y]`.
    imports: HashMap<(String, String), String>,
}

impl Db {
    /// Load a workspace rooted at `root` (must contain `slang.project`).
    /// Reads `*.sl` files excluding `*_test.sl`, like a normal build.
    pub fn load(root: &Path) -> Result<Self, String> {
        let mut db = Db {
            files: Vec::new(),
            file_index: HashMap::new(),
            funcs: HashMap::new(),
            structs: HashMap::new(),
            methods: HashMap::new(),
            imports: HashMap::new(),
        };
        let mut sl_files = Vec::new();
        collect_sl_files(root, &mut sl_files)
            .map_err(|e| format!("cannot scan {}: {e}", root.display()))?;
        sl_files.sort();
        for path in sl_files {
            let src = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let package = package_of(root, &path);
            let (toks, comments) =
                lex_with_comments(&src).map_err(|e| format!("{}: {e}", path.display()))?;
            let (file, errors) = parse(toks);
            if let Some(first) = errors.first() {
                return Err(format!("{}: {first}", path.display()));
            }
            let docs = attach_docs(&comments, &file);
            let fid = FileId(db.files.len());
            db.file_index.insert(path.clone(), fid);
            // Record imports before indexing symbols so aliases resolve.
            for item in &file.items {
                if let Item::Import {
                    path: ip, alias, ..
                } = item
                {
                    let target = import_target(ip);
                    let bind = alias.clone().unwrap_or_else(|| target.clone());
                    db.imports.entry((package.clone(), bind)).or_insert(target);
                }
            }
            db.index_file_symbols(&file, &package, &path, &docs);
            db.files.push(FileData {
                path: path.to_path_buf(),
                package,
                src,
                file,
            });
        }
        Ok(db)
    }

    /// Replace an open file's buffer (unsaved editor contents) and
    /// re-index the workspace. Returns diagnostics for the new buffer;
    /// the old index is kept when the buffer does not lex at all.
    pub fn update_file(&mut self, path: &Path, src: String) -> Vec<slang_syntax::ParseError> {
        let (toks, _) = match lex_with_comments(&src) {
            Ok(v) => v,
            Err(e) => {
                return vec![slang_syntax::ParseError {
                    line: e.line,
                    offset: e.offset,
                    message: e.message,
                }]
            }
        };
        let (file, errors) = parse(toks);
        match self.file_index.get(path) {
            Some(id) => {
                let fd = &mut self.files[id.0];
                fd.src = src;
                fd.file = file;
            }
            None => {
                let package = package_of_path(path);
                let fid = FileId(self.files.len());
                self.file_index.insert(path.to_path_buf(), fid);
                self.files.push(FileData {
                    path: path.to_path_buf(),
                    package,
                    src,
                    file,
                });
            }
        }
        self.rebuild();
        errors
    }

    fn rebuild(&mut self) {
        self.funcs.clear();
        self.structs.clear();
        self.methods.clear();
        self.imports.clear();
        let snapshot: Vec<(File, String, PathBuf, HashMap<u32, String>)> = self
            .files
            .iter()
            .map(|fd| {
                let docs = lex_with_comments(&fd.src)
                    .map(|(_, comments)| attach_docs(&comments, &fd.file))
                    .unwrap_or_default();
                (fd.file.clone(), fd.package.clone(), fd.path.clone(), docs)
            })
            .collect();
        for (file, package, path, docs) in &snapshot {
            for item in &file.items {
                if let Item::Import {
                    path: ip, alias, ..
                } = item
                {
                    let target = import_target(ip);
                    let bind = alias.clone().unwrap_or_else(|| target.clone());
                    self.imports
                        .entry((package.clone(), bind))
                        .or_insert(target);
                }
            }
            self.index_file_symbols(file, package, path, docs);
        }
    }
    pub fn load_single(package: &str, path: &Path, src: &str) -> Result<Self, String> {
        let mut db = Db {
            files: Vec::new(),
            file_index: HashMap::new(),
            funcs: HashMap::new(),
            structs: HashMap::new(),
            methods: HashMap::new(),
            imports: HashMap::new(),
        };
        let (toks, comments) = lex_with_comments(src).map_err(|e| e.to_string())?;
        let (file, errors) = parse(toks);
        if let Some(first) = errors.first() {
            return Err(first.to_string());
        }
        let docs = attach_docs(&comments, &file);
        db.index_file_symbols(&file, package, path, &docs);
        let fid = FileId(db.files.len());
        db.file_index.insert(path.to_path_buf(), fid);
        db.files.push(FileData {
            path: path.to_path_buf(),
            package: package.to_string(),
            src: src.to_string(),
            file,
        });
        Ok(db)
    }

    fn index_file_symbols(
        &mut self,
        file: &File,
        package: &str,
        path: &Path,
        docs: &HashMap<u32, String>,
    ) {
        for item in &file.items {
            match item {
                Item::Fn(f) => {
                    let sym = FuncSym {
                        package: package.to_string(),
                        name: f.name.clone(),
                        file: path.to_path_buf(),
                        line: f.span.line,
                        params: f
                            .params
                            .iter()
                            .map(|p| (p.name.clone(), type_string(&p.ty)))
                            .collect(),
                        ret: f.ret.as_ref().map(type_string),
                        docs: docs.get(&f.span.line).cloned().unwrap_or_default(),
                    };
                    self.funcs
                        .entry((package.to_string(), f.name.clone()))
                        .or_insert(sym);
                }
                Item::Struct(s) => {
                    let sym = StructSym {
                        package: package.to_string(),
                        name: s.name.clone(),
                        file: path.to_path_buf(),
                        line: s.span.line,
                        fields: s
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), type_string(&f.ty)))
                            .collect(),
                        docs: docs.get(&s.span.line).cloned().unwrap_or_default(),
                    };
                    self.structs
                        .entry((package.to_string(), s.name.clone()))
                        .or_insert(sym);
                }
                Item::Enum(e) => {
                    // Enums resolve as named types for hover; variants are
                    // reached through `Type.Variant` field chains.
                    let sym = StructSym {
                        package: package.to_string(),
                        name: e.name.clone(),
                        file: path.to_path_buf(),
                        line: e.span.line,
                        fields: e
                            .variants
                            .iter()
                            .map(|v| (v.name.clone(), String::new()))
                            .collect(),
                        docs: docs.get(&e.span.line).cloned().unwrap_or_default(),
                    };
                    self.structs
                        .entry((package.to_string(), e.name.clone()))
                        .or_insert(sym);
                }
                Item::Impl(i) => {
                    for m in &i.methods {
                        let sym = FuncSym {
                            package: package.to_string(),
                            name: format!("{}.{}", i.type_name, m.name),
                            file: path.to_path_buf(),
                            line: m.span.line,
                            params: m
                                .params
                                .iter()
                                .map(|p| (p.name.clone(), type_string(&p.ty)))
                                .collect(),
                            ret: m.ret.as_ref().map(type_string),
                            docs: docs.get(&m.span.line).cloned().unwrap_or_default(),
                        };
                        self.methods
                            .entry((package.to_string(), i.type_name.clone(), m.name.clone()))
                            .or_insert(sym);
                    }
                }
                _ => {}
            }
        }
    }

    fn file(&self, path: &Path) -> Option<&FileData> {
        self.file_index.get(path).map(|id| &self.files[id.0])
    }

    /// Hover at a byte offset: inferred type + docs + definition site.
    /// Returns `None` when the offset is not on a resolvable name.
    pub fn hover(&self, path: &Path, offset: usize) -> Option<Hover> {
        let fd = self.file(path)?;
        let (toks, _) = lex_with_comments(&fd.src).ok()?;
        let tok = toks
            .iter()
            .find(|t| t.span.offset <= offset && offset < t.span.offset + t.span.len.max(1))?;
        let name = tok.text.clone()?;
        // Only names resolve: identifiers plus keyword-spelled members.
        let kind = tok.kind;
        if !matches!(kind, slang_syntax::TokenKind::Ident)
            && !kind.is_keyword()
            && !kind.is_type_name()
        {
            return None;
        }
        // Qualified `pkg.name` or `recv.method`: look left for `a.b`.
        let ident = qualified_at(&fd.src, offset, &name);
        self.resolve(&fd.package, path, &ident, offset)
    }

    /// Goto-definition at a byte offset.
    pub fn goto_def(&self, path: &Path, offset: usize) -> Option<Location> {
        self.hover(path, offset).map(|h| h.location)
    }

    /// Inlay hints: inferred types for `let` bindings without annotation.
    /// Returns the byte offset where the hint renders plus its label.
    pub fn inlay_hints(&self, path: &Path) -> Vec<InlayHint> {
        let fd = match self.file(path) {
            Some(fd) => fd,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for item in &fd.file.items {
            match item {
                Item::Let {
                    name,
                    ty,
                    init,
                    span,
                    ..
                } if ty.is_none() => {
                    let label = infer_expr(&fd.file, init);
                    if is_concrete_hint(&label, init) {
                        out.push(InlayHint {
                            offset: span.offset + 4 + name.len(),
                            label: format!(": {label}"),
                        });
                    }
                }
                Item::Fn(f) => {
                    if let Some(body) = &f.body {
                        collect_hints(&fd.file, body, &mut out);
                    }
                }
                Item::Impl(i) => {
                    for m in &i.methods {
                        if let Some(body) = &m.body {
                            collect_hints(&fd.file, body, &mut out);
                        }
                    }
                }
                _ => {}
            }
        }
        for s in &fd.file.body {
            stmt_hints(&fd.file, s, &mut out);
        }
        out.sort_by_key(|h| h.offset);
        out
    }

    fn resolve(&self, package: &str, path: &Path, ident: &str, offset: usize) -> Option<Hover> {
        // 1. Locals first: walk enclosing lets/params at this offset.
        if !ident.contains('.') {
            if let Some((ty, line)) = local_type(self.file(path)?, ident, offset) {
                return Some(Hover {
                    symbol: ident.to_string(),
                    ty,
                    docs: String::new(),
                    location: Location {
                        file: path.to_path_buf(),
                        line,
                    },
                });
            }
        }
        // 2. `pkg.name`: alias through imports, else same-package type.
        if let Some((head, rest)) = ident.split_once('.') {
            // Method on a value (`p.sum`) vs package call (`geo.area`):
            // try package-qualified first, then method on inferred receiver.
            if let Some(target) = self.imports.get(&(package.to_string(), head.to_string())) {
                if let Some(f) = self.funcs.get(&(target.clone(), rest.to_string())) {
                    return Some(func_hover(f));
                }
                if let Some(s) = self.structs.get(&(target.clone(), rest.to_string())) {
                    return Some(struct_hover(s));
                }
            }
            if let Some(f) = self.funcs.get(&(package.to_string(), ident.to_string())) {
                return Some(func_hover(f));
            }
            // `Type.Variant` enum reference.
            if let Some(s) = self.structs.get(&(package.to_string(), head.to_string())) {
                if s.fields.iter().any(|(n, _)| n == rest) {
                    return Some(Hover {
                        symbol: ident.to_string(),
                        ty: head.to_string(),
                        docs: s.docs.clone(),
                        location: Location {
                            file: s.file.clone(),
                            line: s.line,
                        },
                    });
                }
            }
            // Method call: infer receiver type from locals, then look up impl.
            if let Some(recv_ty) = receiver_type(self.file(path)?, head, offset) {
                let base = recv_ty.split('[').next().unwrap_or(&recv_ty).to_string();
                if let Some(m) = self
                    .methods
                    .get(&(package.to_string(), base, rest.to_string()))
                {
                    return Some(func_hover(m));
                }
            }
            return None;
        }
        // 3. Same-package function / struct / method on Self-like receiver.
        if let Some(f) = self.funcs.get(&(package.to_string(), ident.to_string())) {
            return Some(func_hover(f));
        }
        if let Some(s) = self.structs.get(&(package.to_string(), ident.to_string())) {
            return Some(struct_hover(s));
        }
        // 4. Imported name used unqualified via alias shadowing a function.
        for ((pkg, bind), target) in &self.imports {
            if pkg == package && bind == ident {
                if let Some(f) = self.funcs.get(&(target.clone(), ident.to_string())) {
                    return Some(func_hover(f));
                }
            }
        }
        None
    }
}

fn func_hover(f: &FuncSym) -> Hover {
    let params = f
        .params
        .iter()
        .map(|(n, t)| format!("{n}: {t}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sig = match &f.ret {
        Some(r) => format!("fn {}({params}) -> {r}", f.name),
        None => format!("fn {}({params})", f.name),
    };
    Hover {
        symbol: f.name.clone(),
        ty: sig,
        docs: f.docs.clone(),
        location: Location {
            file: f.file.clone(),
            line: f.line,
        },
    }
}

fn struct_hover(s: &StructSym) -> Hover {
    Hover {
        symbol: s.name.clone(),
        ty: format!("struct {}", s.name),
        docs: s.docs.clone(),
        location: Location {
            file: s.file.clone(),
            line: s.line,
        },
    }
}

/// Collect `.sl` files, skipping `*_test.sl` like a normal build.
fn collect_sl_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        if p.is_dir() {
            collect_sl_files(&p, out)?;
        } else if p.extension().map(|e| e == "sl").unwrap_or(false) {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !name.ends_with("_test.sl") {
                out.push(p);
            }
        }
    }
    Ok(())
}

/// Package name from a directory, mirroring `pkg_name_of_path`.
fn package_of(root: &Path, file: &Path) -> String {
    let dir = file.parent().unwrap_or(root);
    let base = if dir == root {
        // Entry package keeps its directory name (usually the project name).
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "main".to_string())
    } else {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "main".to_string())
    };
    let mut out: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out = format!("p_{out}");
    }
    out
}

/// Package name for a file added after load (e.g. a new untitled file):
/// its parent directory's name under the same sanitization rule.
fn package_of_path(file: &Path) -> String {
    let base = file
        .parent()
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".to_string());
    sanitize_package(&base)
}

fn sanitize_package(base: &str) -> String {
    let mut out: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out = format!("p_{out}");
    }
    out
}

fn import_target(path: &str) -> String {
    // `import "a/b/util"` binds `util`; the package name is the same rule.
    let base = path.rsplit('/').next().unwrap_or(path);
    let mut out: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out = format!("p_{out}");
    }
    out
}

/// Attach `//` doc runs directly above each item's start line.
fn attach_docs(comments: &[slang_syntax::Comment], file: &File) -> HashMap<u32, String> {
    use std::collections::HashMap;
    let mut by_line: HashMap<u32, &slang_syntax::Comment> = HashMap::new();
    for c in comments {
        by_line.insert(c.line, c);
    }
    let mut out = HashMap::new();
    for line in item_lines(file) {
        let mut run: Vec<String> = Vec::new();
        let mut l = line.saturating_sub(1);
        while let Some(c) = by_line.get(&l) {
            run.push(c.text.trim().trim_start_matches("//").trim().to_string());
            if l == 0 {
                break;
            }
            l -= 1;
        }
        if !run.is_empty() {
            run.reverse();
            out.insert(line, run.join("\n"));
        }
    }
    out
}

fn item_lines(file: &File) -> Vec<u32> {
    file.items
        .iter()
        .map(|i| match i {
            Item::Import { span, .. } | Item::Link { span, .. } | Item::Let { span, .. } => {
                span.line
            }
            Item::Fn(f) => f.span.line,
            Item::Struct(s) => s.span.line,
            Item::Impl(i) => i.span.line,
            Item::Enum(e) => e.span.line,
        })
        .collect()
}

fn type_string(t: &Type) -> String {
    match &t.kind {
        TypeKind::Named(n) => n.clone(),
        TypeKind::Array(i) => format!("[{}]", type_string(i)),
        TypeKind::Map { key, val } => format!("map[{}]{}", type_string(key), type_string(val)),
        TypeKind::Opt(i) => format!("opt[{}]", type_string(i)),
        TypeKind::Result { ok, err } => {
            format!("result[{}, {}]", type_string(ok), type_string(err))
        }
        TypeKind::Chan(i) => format!("chan[{}]", type_string(i)),
        TypeKind::Join(i) => format!("join[{}]", type_string(i)),
        TypeKind::Ref(i) => format!("&{}", type_string(i)),
        TypeKind::RefMut(i) => format!("&mut {}", type_string(i)),
        TypeKind::Own(i) => format!("own {}", type_string(i)),
        TypeKind::Gc(i) => format!("gc {}", type_string(i)),
        TypeKind::Ptr(i) => format!("*{}", type_string(i)),
        TypeKind::Fn { params, ret } => {
            let ps = params
                .iter()
                .map(type_string)
                .collect::<Vec<_>>()
                .join(", ");
            match ret {
                Some(r) => format!("fn({ps}) -> {}", type_string(r)),
                None => format!("fn({ps})"),
            }
        }
    }
}

/// Find the enclosing local (`let` or parameter) for `name` visible at
/// `offset`, returning its inferred type and definition line.
fn local_type(fd: &FileData, name: &str, offset: usize) -> Option<(String, u32)> {
    // Top-level lets are visible everywhere in the file.
    for item in &fd.file.items {
        match item {
            Item::Let {
                name: n,
                ty,
                init,
                span,
                ..
            } if n == name && span.offset <= offset => {
                let ty = ty
                    .as_ref()
                    .map(type_string)
                    .unwrap_or_else(|| infer_expr(&fd.file, init));
                return Some((ty, span.line));
            }
            Item::Fn(f) if f.span.offset <= offset && offset <= fn_end(f) => {
                // Parameters first.
                for p in &f.params {
                    if p.name == name {
                        return Some((type_string(&p.ty), p.span.line));
                    }
                }
                if let Some(body) = &f.body {
                    if let Some(hit) = block_local(body, &fd.file, name, offset) {
                        return Some(hit);
                    }
                }
            }
            _ => {}
        }
    }
    // Main-body statements before the offset.
    let mut best: Option<(String, u32, usize)> = None;
    for s in &fd.file.body {
        if let Stmt::Let {
            name: n,
            ty,
            init,
            span,
            ..
        } = s
        {
            if n == name && span.offset <= offset {
                let ty = ty
                    .as_ref()
                    .map(type_string)
                    .unwrap_or_else(|| infer_expr(&fd.file, init));
                best = Some((ty, span.line, span.offset));
            }
        }
    }
    best.map(|(t, l, _)| (t, l))
}

fn block_local(body: &Block, file: &File, name: &str, offset: usize) -> Option<(String, u32)> {
    let mut best: Option<(String, u32, usize)> = None;
    for s in &body.stmts {
        match s {
            Stmt::Let {
                name: n,
                ty,
                init,
                span,
                ..
            } if n == name && span.offset <= offset => {
                let ty = ty
                    .as_ref()
                    .map(type_string)
                    .unwrap_or_else(|| infer_expr(file, init));
                best = Some((ty, span.line, span.offset));
            }
            Stmt::ForRange {
                name: n,
                body: b,
                span,
                ..
            } if n == name && span.offset <= offset => {
                return Some(("int".to_string(), span.line));
            }
            Stmt::ForIn {
                name: n,
                name2,
                iter,
                body: b,
                span,
            } if span.offset <= offset => {
                if n == name {
                    let t = iter_elem_type(file, iter);
                    return Some((t, span.line));
                }
                if name2.as_deref() == Some(name) {
                    return Some(("str".to_string(), span.line));
                }
                let _ = b;
            }
            _ => {}
        }
    }
    best.map(|(t, l, _)| (t, l))
}

/// Infer the element type of a `for x in iter` loop for hover labels.
fn iter_elem_type(file: &File, iter: &Expr) -> String {
    let t = infer_expr(file, iter);
    if let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return inner.to_string();
    }
    if t == "bytes" {
        return "int".to_string();
    }
    "str".to_string()
}

/// Receiver type for `head.rest`: the local `head`'s inferred type.
fn receiver_type(fd: &FileData, head: &str, offset: usize) -> Option<String> {
    local_type(fd, head, offset).map(|(t, _)| t)
}

/// Best-effort expression type inference for hover labels.
fn infer_expr(_file: &File, e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int { .. } => "int".to_string(),
        ExprKind::Float(_) => "float".to_string(),
        ExprKind::String(_) => "str".to_string(),
        ExprKind::Bytes(_) => "bytes".to_string(),
        ExprKind::Bool(_) => "bool".to_string(),
        ExprKind::Ident(n) => n.clone(),
        ExprKind::Binary { op, lhs, .. } if op == ".." || op == "..=" => {
            let _ = lhs;
            "[int]".to_string()
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let lt = infer_expr(_file, lhs);
            let rt = infer_expr(_file, rhs);
            if op == "+" && (lt == "str" || rt == "str") {
                "str".to_string()
            } else if matches!(
                op.as_str(),
                "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||"
            ) {
                "bool".to_string()
            } else {
                lt
            }
        }
        ExprKind::Unary { operand, .. } => infer_expr(_file, operand),
        ExprKind::List(items) => {
            let inner = items
                .first()
                .map(|e| infer_expr(_file, e))
                .unwrap_or_else(|| "int".to_string());
            format!("[{inner}]")
        }
        ExprKind::Index { base, .. } => {
            let bt = infer_expr(_file, base);
            if bt == "str" || bt == "bytes" {
                "int".to_string()
            } else if let Some(inner) = bt.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                inner.to_string()
            } else {
                "int".to_string()
            }
        }
        ExprKind::Field { .. } => "field".to_string(),
        ExprKind::Call { .. } => "call".to_string(),
        ExprKind::Cast { ty, .. } => type_string(ty),
        ExprKind::StructLit { tyname, .. } => tyname.clone(),
        _ => "int".to_string(),
    }
}

fn fn_end(f: &slang_syntax::FnDecl) -> usize {
    f.body
        .as_ref()
        .map(|b| b.span.offset + b.span.len)
        .unwrap_or(f.span.offset + f.span.len)
}

/// Reconstruct `pkg.name` / `recv.method` around an identifier at `offset`.
fn qualified_at(src: &str, offset: usize, _name: &str) -> String {
    let b = src.as_bytes();
    // Normalize to full identifier bounds around the cursor first: the
    // offset may sit anywhere inside the token, not just at its start.
    let mut start = offset.min(src.len());
    while start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_') {
        start -= 1;
    }
    let mut end = offset.min(src.len());
    while end < src.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
        end += 1;
    }
    let head = &src[start..end];
    // Cursor on the head of `head.tail`: extend right.
    if b.get(end) == Some(&b'.') {
        let mut t = end + 1;
        while t < src.len() && (b[t].is_ascii_alphanumeric() || b[t] == b'_') {
            t += 1;
        }
        if t > end + 1 {
            return format!("{}.{}", head, &src[end + 1..t]);
        }
    }
    // Cursor on the tail of `head.tail`: extend left.
    if start >= 1 && b.get(start - 1) == Some(&b'.') {
        let mut h = start - 1;
        while h > 0 && (b[h - 1].is_ascii_alphanumeric() || b[h - 1] == b'_') {
            h -= 1;
        }
        return format!("{}.{}", &src[h..start - 1], head);
    }
    head.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn db(src: &str) -> Db {
        Db::load_single("main", &PathBuf::from("main.sl"), src).unwrap()
    }

    #[test]
    fn should_hover_let_inferred_type() {
        let src = "let n = 10;\nprintln(n);\n";
        let d = db(src);
        // `n` inside println resolves to the let's inferred int.
        let off = src.find("(n)").unwrap() + 1;
        let h = d.hover(&PathBuf::from("main.sl"), off).unwrap();
        assert_eq!(h.ty, "int");
    }

    #[test]
    fn should_goto_function_def() {
        let src = "// Add two.\nfn add(a: int, b: int) -> int {\n    return a + b;\n}\nlet x = add(1, 2);\n";
        let d = db(src);
        let off = src.find("add(1").unwrap() + 1;
        let h = d.hover(&PathBuf::from("main.sl"), off).unwrap();
        assert!(h.ty.contains("fn add"));
        assert_eq!(h.docs, "Add two.");
        assert_eq!(h.location.line, 2);
    }

    #[test]
    fn should_emit_inlay_hints_for_unannotated_lets() {
        let src = "let n = 10;\nlet name: str = \"x\";\n";
        let d = db(src);
        let hints = d.inlay_hints(&PathBuf::from("main.sl"));
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].label, ": int");
    }

    #[test]
    fn should_update_open_buffer() {
        let src = "fn add(a: int) -> int {\n    return a;\n}\n";
        let mut d = db(src);
        let path = PathBuf::from("main.sl");
        let diags = d.update_file(
            &path,
            "fn add(a: int) -> int {\n    return a;\n}\nlet x = add(1);\n".to_string(),
        );
        assert!(diags.is_empty());
        let off = "fn add(a: int) -> int {\n    return a;\n}\nlet x = add(1);\n"
            .find("add(1)")
            .unwrap()
            + 1;
        let h = d.hover(&path, off).unwrap();
        assert!(h.ty.contains("fn add"));
    }

    #[test]
    fn should_resolve_method_on_struct() {
        let src = "struct Point { x: int }\nimpl Point {\nfn sum(self: Point) -> int { return self.x; }\n}\nlet p = Point { x: 1 };\nprintln(p.sum());\n";
        let d = db(src);
        let off = src.find("p.sum").unwrap() + 2;
        let h = d.hover(&PathBuf::from("main.sl"), off).unwrap();
        assert!(h.ty.contains("sum"));
    }
}
