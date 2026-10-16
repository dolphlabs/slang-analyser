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

mod native_sigs_generated;
use native_sigs_generated::native_sig;

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
    pub fields: Vec<(String, String, u32)>,
    pub docs: String,
}

#[derive(Debug, Clone)]
pub struct InlayHint {
    /// Byte offset where the hint renders (end of the binding name).
    pub offset: usize,
    pub label: String,
}

/// Hint when inference says more than the initializer's own text.
/// Calls and field walks now resolve to real types, so they hint too;
/// only unknown or circular results are withheld.
fn is_concrete_hint(label: &str, init: &Expr) -> bool {
    if label.starts_with('{') || label == "call" || label == "field" {
        return false;
    }
    match &init.kind {
        ExprKind::Ident(n) => label != n,
        _ => true,
    }
}

#[derive(Debug, Clone)]
pub struct ConstSym {
    pub package: String,
    pub name: String,
    pub file: PathBuf,
    pub line: u32,
    pub ty: String,
    pub docs: String,
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
    /// Definition site. `None` for compiler-provided items (native
    /// package functions) that have no source file to jump to.
    pub location: Option<Location>,
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
    /// (package, name) -> package-level `let` global.
    consts: HashMap<(String, String), ConstSym>,
    /// Parse errors per file. Files with errors are still indexed from
    /// their recovered AST: one unknown construct must never blind the
    /// whole workspace. Only unreadable/unlexable files are skipped.
    file_errors: HashMap<PathBuf, Vec<slang_syntax::ParseError>>,
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
            consts: HashMap::new(),
            file_errors: HashMap::new(),
        };
        let mut sl_files = Vec::new();
        collect_sl_files(root, &mut sl_files)
            .map_err(|e| format!("cannot scan {}: {e}", root.display()))?;
        sl_files.sort();
        for path in sl_files {
            let src = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let package = package_of(root, &path);
            let (toks, comments) = match lex_with_comments(&src) {
                Ok(v) => v,
                // Unlexable files are skipped but recorded; the rest of
                // the workspace still indexes.
                Err(e) => {
                    db.file_errors.insert(
                        path.clone(),
                        vec![slang_syntax::ParseError {
                            line: e.line,
                            offset: e.offset,
                            message: e.message,
                        }],
                    );
                    continue;
                }
            };
            let (file, errors) = parse(toks);
            if !errors.is_empty() {
                db.file_errors.insert(path.clone(), errors);
            }
            let docs = attach_docs(&comments, &file);
            let fd = FileData {
                path: path.clone(),
                package,
                src,
                file,
            };
            let fid = FileId(db.files.len());
            db.file_index.insert(path.clone(), fid);
            db.record_imports(&fd);
            db.index_file_symbols(&fd, &docs);
            db.files.push(fd);
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

    /// Record a file's `import` bindings before indexing so aliases
    /// resolve: `import "a/b"` binds `b`, `import "x" as y` binds `y`.
    fn record_imports(&mut self, fd: &FileData) {
        for item in &fd.file.items {
            if let Item::Import {
                path: ip, alias, ..
            } = item
            {
                let target = import_target(ip);
                let bind = alias.clone().unwrap_or_else(|| target.clone());
                self.imports
                    .entry((fd.package.clone(), bind))
                    .or_insert(target);
            }
        }
    }

    fn rebuild(&mut self) {
        self.funcs.clear();
        self.structs.clear();
        self.methods.clear();
        self.imports.clear();
        self.consts.clear();
        let snapshot = self.files.clone();
        for fd in &snapshot {
            let docs = lex_with_comments(&fd.src)
                .map(|(_, comments)| attach_docs(&comments, &fd.file))
                .unwrap_or_default();
            self.record_imports(fd);
            self.index_file_symbols(fd, &docs);
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
            consts: HashMap::new(),
            file_errors: HashMap::new(),
        };
        let (toks, comments) = lex_with_comments(src).map_err(|e| e.to_string())?;
        let (file, errors) = parse(toks);
        if let Some(first) = errors.first() {
            return Err(first.to_string());
        }
        let docs = attach_docs(&comments, &file);
        let fd = FileData {
            path: path.to_path_buf(),
            package: package.to_string(),
            src: src.to_string(),
            file,
        };
        for item in &fd.file.items {
            if let Item::Import {
                path: ip, alias, ..
            } = item
            {
                let target = import_target(ip);
                let bind = alias.clone().unwrap_or_else(|| target.clone());
                db.imports
                    .entry((fd.package.clone(), bind))
                    .or_insert(target);
            }
        }
        db.index_file_symbols(&fd, &docs);
        let fid = FileId(db.files.len());
        db.file_index.insert(path.to_path_buf(), fid);
        db.files.push(fd);
        Ok(db)
    }

    fn index_file_symbols(&mut self, fd: &FileData, docs: &HashMap<u32, String>) {
        let (file, package, path) = (&fd.file, fd.package.clone(), fd.path.clone());
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
                            .map(|f| (f.name.clone(), type_string(&f.ty), f.span.line))
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
                            .map(|v| (v.name.clone(), String::new(), v.span.line))
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
                            .entry((package.clone(), i.type_name.clone(), m.name.clone()))
                            .or_insert(sym);
                    }
                }
                Item::Let {
                    name,
                    ty,
                    init,
                    span,
                    ..
                } => {
                    // Package-level globals: visible file-wide and, when
                    // `pub`, to importers. Unannotated ones infer.
                    let t = ty
                        .as_ref()
                        .map(type_string)
                        .unwrap_or_else(|| self.infer_expr(fd, init, init.span.offset));
                    let sym = ConstSym {
                        package: package.clone(),
                        name: name.clone(),
                        file: path.clone(),
                        line: span.line,
                        ty: t,
                        docs: docs.get(&span.line).cloned().unwrap_or_default(),
                    };
                    self.consts
                        .entry((package.clone(), name.clone()))
                        .or_insert(sym);
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
        // Qualified `pkg.name`, `recv.method`, or full chains like
        // `c.config.database`: the path up to the cursor's own segment.
        let ident = qualified_at(&fd.src, offset, &name);
        // A `(` after optional whitespace means the cursor names a call:
        // prefer methods over fields for `recv.method(...)`.
        let after = &fd.src[tok.span.offset + tok.span.len..];
        let is_call = after.trim_start_matches([' ', '\t']).starts_with('(');
        self.resolve(&fd.package, path, &ident, offset, is_call)
    }

    /// Goto-definition at a byte offset.
    pub fn goto_def(&self, path: &Path, offset: usize) -> Option<Location> {
        self.hover(path, offset).and_then(|h| h.location)
    }

    /// Parse errors recorded for a file at load time, if any.
    pub fn file_errors(&self, path: &Path) -> &[slang_syntax::ParseError] {
        self.file_errors.get(path).map(Vec::as_slice).unwrap_or(&[])
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
                    let label = self.infer_expr(fd, init, init.span.offset);
                    if is_concrete_hint(&label, init) {
                        out.push(InlayHint {
                            offset: span.offset + 4 + name.len(),
                            label: format!(": {label}"),
                        });
                    }
                }
                Item::Fn(f) => {
                    if let Some(body) = &f.body {
                        self.collect_hints(fd, body, &mut out);
                    }
                }
                Item::Impl(i) => {
                    for m in &i.methods {
                        if let Some(body) = &m.body {
                            self.collect_hints(fd, body, &mut out);
                        }
                    }
                }
                _ => {}
            }
        }
        for s in &fd.file.body {
            self.stmt_hints(fd, s, &mut out);
        }
        out.sort_by_key(|h| h.offset);
        out
    }

    fn resolve(
        &self,
        package: &str,
        path: &Path,
        ident: &str,
        offset: usize,
        is_call: bool,
    ) -> Option<Hover> {
        let fd = self.file(path)?;
        // 1. Locals first (unqualified names only), then package globals.
        if !ident.contains('.') {
            if let Some((ty, line)) = self.local_type(fd, ident, offset) {
                return Some(Hover {
                    symbol: ident.to_string(),
                    ty,
                    docs: String::new(),
                    location: Some(Location {
                        file: path.to_path_buf(),
                        line,
                    }),
                });
            }
            if let Some(c) = self.consts.get(&(package.to_string(), ident.to_string())) {
                return Some(const_hover(c));
            }
            if let Some(f) = self.funcs.get(&(package.to_string(), ident.to_string())) {
                return Some(func_hover(f));
            }
            if let Some(s) = self.structs.get(&(package.to_string(), ident.to_string())) {
                return Some(struct_hover(s));
            }
            // Imported name used unqualified via alias.
            for ((pkg, bind), target) in &self.imports {
                if pkg == package && bind == ident {
                    if let Some(f) = self.funcs.get(&(target.clone(), ident.to_string())) {
                        return Some(func_hover(f));
                    }
                    if let Some(c) = self.consts.get(&(target.clone(), ident.to_string())) {
                        return Some(const_hover(c));
                    }
                }
            }
            return None;
        }
        let parts: Vec<&str> = ident.split('.').collect();
        // 2. Two-part import-qualified names: functions, constants, types.
        if parts.len() == 2 {
            let (head, rest) = (parts[0], parts[1]);
            if let Some(target) = self.imports.get(&(package.to_string(), head.to_string())) {
                if let Some(f) = self.funcs.get(&(target.clone(), rest.to_string())) {
                    return Some(func_hover(f));
                }
                if let Some(c) = self.consts.get(&(target.clone(), rest.to_string())) {
                    return Some(const_hover(c));
                }
                if let Some(s) = self.structs.get(&(target.clone(), rest.to_string())) {
                    return Some(struct_hover(s));
                }
                // Compiler-provided package: signatures, no jump target.
                if is_native_package(target) {
                    if let Some(h) = native_hover(target, rest) {
                        return Some(h);
                    }
                }
            }
            // `Type.Variant` enum reference.
            if let Some(s) = self.structs.get(&(package.to_string(), head.to_string())) {
                if s.fields.iter().any(|(n, _, _)| n == rest) {
                    return Some(Hover {
                        symbol: ident.to_string(),
                        ty: head.to_string(),
                        docs: s.docs.clone(),
                        location: Some(Location {
                            file: s.file.clone(),
                            line: s.line,
                        }),
                    });
                }
            }
        }
        // 3. Value chain walk: `c.config.database`. Plain access prefers
        // fields, call position prefers methods; each falls back.
        if let Some(chain) = self.walk_chain(fd, package, &parts, offset, is_call) {
            return Some(chain);
        }
        None
    }

    /// Walk `head.mid....last` starting from a local binding: each step
    /// resolves as a struct field or a method, with the definition site.
    fn walk_chain(
        &self,
        fd: &FileData,
        package: &str,
        parts: &[&str],
        offset: usize,
        is_call: bool,
    ) -> Option<Hover> {
        let (mut cur_ty, head_line) = self.local_type(fd, parts[0], offset)?;
        let mut loc = Location {
            file: fd.path.clone(),
            line: head_line,
        };
        for (i, part) in parts[1..].iter().enumerate() {
            let last = i == parts.len() - 2;
            let base = generic_base(&cur_ty).to_string();
            // A call's name resolves to the method itself (signature +
            // docs), like a plain function call does.
            if last && is_call {
                if let Some(m) =
                    self.methods
                        .get(&(package.to_string(), base.clone(), part.to_string()))
                {
                    let params = m
                        .params
                        .iter()
                        .map(|(n, t)| format!("{n}: {t}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sig = match &m.ret {
                        Some(r) => format!("fn {}({params}) -> {r}", m.name),
                        None => format!("fn {}({params})", m.name),
                    };
                    return Some(Hover {
                        symbol: parts.join("."),
                        ty: sig,
                        docs: m.docs.clone(),
                        location: Some(Location {
                            file: m.file.clone(),
                            line: m.line,
                        }),
                    });
                }
            }
            let mut hit: Option<(String, Location)> = None;
            // Plain access prefers the field; a call prefers the method.
            if !(last && is_call) {
                if let Some((ft, ff, fl)) = self.struct_field_loc(package, &base, part) {
                    hit = Some((ft, Location { file: ff, line: fl }));
                }
            }
            if hit.is_none() {
                if let Some(m) =
                    self.methods
                        .get(&(package.to_string(), base.clone(), part.to_string()))
                {
                    hit = Some((
                        m.ret.clone().unwrap_or_else(|| "()".to_string()),
                        Location {
                            file: m.file.clone(),
                            line: m.line,
                        },
                    ));
                }
            }
            if hit.is_none() {
                if let Some((ft, ff, fl)) = self.struct_field_loc(package, &base, part) {
                    hit = Some((ft, Location { file: ff, line: fl }));
                }
            }
            match hit {
                Some((t, l)) => {
                    cur_ty = t;
                    loc = l;
                }
                None => return None,
            }
        }
        Some(Hover {
            symbol: parts.join("."),
            ty: cur_ty,
            docs: String::new(),
            location: Some(loc),
        })
    }
}

fn const_hover(c: &ConstSym) -> Hover {
    Hover {
        symbol: c.name.clone(),
        ty: format!("let {}: {}", c.name, c.ty),
        docs: c.docs.clone(),
        location: Some(Location {
            file: c.file.clone(),
            line: c.line,
        }),
    }
}

/// True for compiler-provided packages (`time`, `net`, `json`, ...),
/// mirroring NATIVE_PKGS in slang's loader.c. Their functions have no
/// source files; signatures come from the generated NatSig table.
fn is_native_package(name: &str) -> bool {
    native_sigs_generated::NATIVE_SIGS
        .iter()
        .any(|s| s.pkg == name)
}

/// Hover for a native-package function: signature exactly as
/// `slangc doc` renders it (`fn sleep(int)`), with no jump target.
fn native_hover(pkg: &str, name: &str) -> Option<Hover> {
    let sig = native_sig(pkg, name)?;
    let mut ty = format!("fn {}({})", name, sig.params.join(", "));
    if let Some(ret) = sig.ret {
        ty.push_str(&format!(" -> {ret}"));
    }
    Some(Hover {
        symbol: format!("{pkg}.{name}"),
        ty,
        docs: String::new(),
        location: None,
    })
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
        location: Some(Location {
            file: f.file.clone(),
            line: f.line,
        }),
    }
}

fn struct_hover(s: &StructSym) -> Hover {
    Hover {
        symbol: s.name.clone(),
        ty: format!("struct {}", s.name),
        docs: s.docs.clone(),
        location: Some(Location {
            file: s.file.clone(),
            line: s.line,
        }),
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

impl Db {
    /// Find the enclosing local (`let`, parameter, `guard`/`if let` binding)
    /// for `name` visible at `offset`: inferred type + definition line.
    /// Identifier chains (`let b = a;`) are followed to a concrete type.
    fn local_type(&self, fd: &FileData, name: &str, offset: usize) -> Option<(String, u32)> {
        let (ty, line) = self.local_raw(fd, name, offset)?;
        Some((self.chase(fd, ty, name, offset), line))
    }

    /// Single lookup step without chain-following (see `chase`).
    fn local_raw(&self, fd: &FileData, name: &str, offset: usize) -> Option<(String, u32)> {
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
                        .unwrap_or_else(|| self.infer_expr(fd, init, init.span.offset));
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
                        if let Some(hit) = self.block_local(fd, body, name, offset) {
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
                        .unwrap_or_else(|| self.infer_expr(fd, init, init.span.offset));
                    best = Some((ty, span.line, span.offset));
                }
            }
        }
        best.map(|(t, l, _)| (t, l))
    }

    /// Follow `let`-chains to a concrete type, bounded so odd cycles end.
    fn chase(&self, fd: &FileData, ty: String, exclude: &str, offset: usize) -> String {
        let mut cur = ty;
        for _ in 0..8 {
            if cur == exclude || !is_bare_ident(&cur) {
                break;
            }
            let next = cur.clone();
            match self.local_raw(fd, &next, offset) {
                Some((t, _)) if t != cur => cur = t,
                _ => break,
            }
        }
        cur
    }

    fn block_local(
        &self,
        fd: &FileData,
        body: &Block,
        name: &str,
        offset: usize,
    ) -> Option<(String, u32)> {
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
                        .unwrap_or_else(|| self.infer_expr(fd, init, init.span.offset));
                    best = Some((ty, span.line, span.offset));
                }
                Stmt::GuardLet {
                    name: n,
                    expr,
                    err_name,
                    err_expr,
                    body,
                    span,
                    ..
                } => {
                    if n == name && span.offset <= offset {
                        // `guard let x = opt_or_result`: x is the unwrapped value.
                        let t = self.infer_expr(fd, expr, expr.span.offset);
                        let t = self.chase(fd, t, name, offset);
                        return Some((unwrap_single(&t), span.line));
                    }
                    // `else let e = err_of(x)`: e lives in the diverging
                    // else block and in the header declaring it.
                    if let (Some(en), Some(ee)) = (err_name, err_expr) {
                        let end = body.span.offset + body.span.len;
                        if en == name && span.offset <= offset && offset <= end {
                            return Some((self.infer_expr(fd, ee, ee.span.offset), span.line));
                        }
                    }
                }
                Stmt::IfLet {
                    name: n,
                    expr,
                    then,
                    err_name,
                    err_expr,
                    else_,
                    span,
                    ..
                } => {
                    // The binding is declared in the header (`if/else let`),
                    // so the header itself counts as in scope, through the
                    // end of the `then` block.
                    let then_end = then.span.offset + then.span.len;
                    if n == name && span.offset <= offset && offset <= then_end {
                        let t = self.infer_expr(fd, expr, expr.span.offset);
                        let t = self.chase(fd, t, name, offset);
                        return Some((unwrap_single(&t), span.line));
                    }
                    if let (Some(en), Some(ee), Some(eb)) = (err_name, err_expr, else_) {
                        let else_end = eb.span.offset + eb.span.len;
                        if en == name && span.offset <= offset && offset <= else_end {
                            return Some((self.infer_expr(fd, ee, ee.span.offset), span.line));
                        }
                    }
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
                        let t = self.iter_elem_type(fd, iter, offset);
                        return Some((t, span.line));
                    }
                    if name2.as_deref() == Some(name) {
                        let it = self.infer_expr(fd, iter, iter.span.offset);
                        let it = self.chase(fd, it, name, offset);
                        let t = map_value(&it).unwrap_or_else(|| "str".to_string());
                        return Some((t, span.line));
                    }
                    let _ = b;
                }
                _ => {}
            }
        }
        best.map(|(t, l, _)| (t, l))
    }

    /// Infer the element type of a `for x in iter` loop for hover labels.
    fn iter_elem_type(&self, fd: &FileData, iter: &Expr, offset: usize) -> String {
        let t = self.infer_expr(fd, iter, iter.span.offset);
        let t = self.chase(fd, t, "", offset);
        if t == "bytes" {
            return "int".to_string();
        }
        if t.starts_with("map[") {
            if let Some(inner) = map_key(&t) {
                return inner;
            }
            return "str".to_string();
        }
        if let Some(inner) = bracket_content(&t, "") {
            return inner;
        }
        "str".to_string()
    }

    /// Best-effort expression type inference for hover labels.
    fn infer_expr(&self, fd: &FileData, e: &Expr, offset: usize) -> String {
        self.infer_inner(fd, e, offset, 0)
    }

    fn infer_inner(&self, fd: &FileData, e: &Expr, offset: usize, depth: u8) -> String {
        if depth > 8 {
            return "{unknown}".to_string();
        }
        let d = depth + 1;
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
                if op == "??" {
                    let lt = self.infer_inner(fd, lhs, offset, d);
                    let lt = self.chase(fd, lt, "", offset);
                    return unwrap_single(&lt);
                }
                let lt = self.infer_inner(fd, lhs, offset, d);
                let rt = self.infer_inner(fd, rhs, offset, d);
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
            ExprKind::Unary { operand, .. } => self.infer_inner(fd, operand, offset, d),
            ExprKind::List(items) => {
                let inner = items
                    .first()
                    .map(|e| self.infer_inner(fd, e, offset, d))
                    .unwrap_or_else(|| "int".to_string());
                format!("[{inner}]")
            }
            ExprKind::Index { base, .. } => {
                let bt = self.infer_inner(fd, base, offset, d);
                let bt = self.chase(fd, bt, "", offset);
                if bt == "str" || bt == "bytes" {
                    "int".to_string()
                } else if bt.starts_with("map[") {
                    map_value(&bt).unwrap_or_else(|| "int".to_string())
                } else if let Some(inner) = bracket_content(&bt, "") {
                    inner
                } else {
                    "int".to_string()
                }
            }
            ExprKind::Field { base, name } => {
                let bt = self.infer_inner(fd, base, offset, d);
                let bt = self.chase(fd, bt, "", offset);
                if let Some(ft) = self.struct_field_type(&fd.package, generic_base(&bt), name) {
                    return ft;
                }
                "field".to_string()
            }
            ExprKind::Call {
                name: Some(n),
                args,
                ..
            } => {
                // `err_of(x)` yields the E of x's `result[T, E]`.
                if n == "err_of" && args.len() == 1 {
                    let at = self.infer_inner(fd, &args[0], offset, d);
                    let at = self.chase(fd, at, "", offset);
                    if let Some((_, err)) = result_parts(&at) {
                        return err;
                    }
                    return "{unknown}".to_string();
                }
                if let Some((head, rest)) = n.split_once('.') {
                    // Import-qualified `pkg.func` first ...
                    if let Some(target) = self.imports.get(&(fd.package.clone(), head.to_string()))
                    {
                        if let Some(r) = self.func_return(target, rest) {
                            return r;
                        }
                        // ... including compiler-provided packages, whose
                        // return types come from the NatSig table.
                        // (`json.decode` is generic over the annotated
                        // target type, which inference can't see here.)
                        if is_native_package(target) {
                            if target == "json" && rest == "decode" {
                                return "{unknown}".to_string();
                            }
                            if let Some(sig) = native_sig(target, rest) {
                                return sig.ret.unwrap_or("()").to_string();
                            }
                        }
                    }
                    // ... else a method on a receiver value: `p.sum()`
                    // keeps its dotted-call shape, so resolve it here.
                    let rt = self
                        .local_type(fd, head, offset)
                        .map(|(t, _)| t)
                        .unwrap_or_else(|| head.to_string());
                    if let Some(r) = self.method_return(&fd.package, generic_base(&rt), rest) {
                        return r;
                    }
                    return "{unknown}".to_string();
                }
                if let Some(r) = self.func_return(&fd.package, n) {
                    return r;
                }
                "{unknown}".to_string()
            }
            ExprKind::Call { .. } => "{unknown}".to_string(),
            ExprKind::MethodCall { recv, name, .. } => {
                let rt = self.infer_inner(fd, recv, offset, d);
                let rt = self.chase(fd, rt, "", offset);
                if let Some(r) = self.method_return(&fd.package, generic_base(&rt), name) {
                    return r;
                }
                "{unknown}".to_string()
            }
            ExprKind::Cast { ty, .. } => type_string(ty),
            ExprKind::StructLit { tyname, .. } => tyname.clone(),
            ExprKind::Spawn(call) => {
                let inner = self.infer_inner(fd, call, offset, d);
                if inner.starts_with('{') {
                    return "{unknown}".to_string();
                }
                format!("join[{inner}]")
            }
            _ => "int".to_string(),
        }
    }

    /// A plain function's declared return type (`()` for void).
    fn func_return(&self, package: &str, name: &str) -> Option<String> {
        self.funcs
            .get(&(package.to_string(), name.to_string()))
            .map(|f| f.ret.clone().unwrap_or_else(|| "()".to_string()))
    }

    /// A struct method's declared return type.
    fn method_return(&self, package: &str, base: &str, method: &str) -> Option<String> {
        self.methods
            .get(&(package.to_string(), base.to_string(), method.to_string()))
            .map(|f| f.ret.clone().unwrap_or_else(|| "()".to_string()))
    }

    /// A struct/enum field's declared type, file, and line. Enum variants
    /// (recorded with an empty type) resolve to the enum itself.
    fn struct_field_loc(
        &self,
        package: &str,
        base: &str,
        field: &str,
    ) -> Option<(String, PathBuf, u32)> {
        if let Some((pkg, ty)) = base.split_once('.') {
            if let Some(s) = self.structs.get(&(pkg.to_string(), ty.to_string())) {
                if let Some((_, ft, fl)) = s.fields.iter().find(|(n, _, _)| n == field) {
                    let t = if ft.is_empty() {
                        base.to_string()
                    } else {
                        ft.clone()
                    };
                    return Some((t, s.file.clone(), *fl));
                }
            }
        }
        self.structs
            .get(&(package.to_string(), base.to_string()))
            .and_then(|s| {
                s.fields
                    .iter()
                    .find(|(n, _, _)| n == field)
                    .map(|(_, ft, fl)| {
                        let t = if ft.is_empty() {
                            base.to_string()
                        } else {
                            ft.clone()
                        };
                        (t, s.file.clone(), *fl)
                    })
            })
    }

    /// A struct/enum field's declared type.
    fn struct_field_type(&self, package: &str, base: &str, field: &str) -> Option<String> {
        self.struct_field_loc(package, base, field)
            .map(|(t, _, _)| t)
    }

    fn collect_hints(&self, fd: &FileData, body: &Block, out: &mut Vec<InlayHint>) {
        for s in &body.stmts {
            self.stmt_hints(fd, s, out);
        }
    }

    fn stmt_hints(&self, fd: &FileData, s: &Stmt, out: &mut Vec<InlayHint>) {
        match s {
            Stmt::Let {
                name,
                ty,
                init,
                span,
                ..
            } if ty.is_none() => {
                let label = self.infer_expr(fd, init, init.span.offset);
                if is_concrete_hint(&label, init) {
                    out.push(InlayHint {
                        offset: span.offset + 4 + name.len(),
                        label: format!(": {label}"),
                    });
                }
            }
            Stmt::If { then, else_, .. } => {
                self.collect_hints(fd, then, out);
                if let Some(Else::Block(b)) = else_ {
                    self.collect_hints(fd, b, out);
                }
            }
            Stmt::While { body, .. }
            | Stmt::ForRange { body, .. }
            | Stmt::ForIn { body, .. }
            | Stmt::Unsafe { body, .. } => {
                self.collect_hints(fd, body, out);
            }
            Stmt::GuardLet { body, .. } => self.collect_hints(fd, body, out),
            Stmt::IfLet { then, else_, .. } => {
                self.collect_hints(fd, then, out);
                if let Some(b) = else_ {
                    self.collect_hints(fd, b, out);
                }
            }
            Stmt::Select { cases, default, .. } => {
                for c in cases {
                    self.collect_hints(fd, &c.body, out);
                }
                if let Some(d) = default {
                    self.collect_hints(fd, d, out);
                }
            }
            Stmt::Switch { cases, default, .. } => {
                for c in cases {
                    if let SwitchBody::Block(b) = &c.body {
                        self.collect_hints(fd, b, out);
                    }
                }
                if let Some(d) = default {
                    self.collect_hints(fd, d, out);
                }
            }
            _ => {}
        }
    }
}

/// True for a plain `name` (possibly `pkg.Name`): something a `let`-chain
/// can resolve through, as opposed to a rendered container type.
fn is_bare_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// `opt[T]` / `result[T, E]` project to `T`: what `guard let` binds.
fn unwrap_single(ty: &str) -> String {
    if let Some(inner) = bracket_content(ty, "opt") {
        return inner;
    }
    if let Some((ok, _)) = result_parts(ty) {
        return ok;
    }
    ty.to_string()
}

/// Contents of the first top-level `[...]`, optionally after a `head`
/// prefix (`opt`, or `""` for a bare `[T]`).
fn bracket_content(ty: &str, head: &str) -> Option<String> {
    let mut rest = ty.strip_prefix(head)?.strip_prefix('[')?;
    let _ = &mut rest;
    let mut depth = 0;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                if depth == 0 {
                    return Some(rest[..i].to_string());
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Split `result[OK, ERR]` into its two sides at the top-level comma.
fn result_parts(ty: &str) -> Option<(String, String)> {
    let rest = ty.strip_prefix("result")?.strip_prefix('[')?;
    let mut depth = 0;
    let mut comma = None;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                if depth == 0 {
                    end = Some(i);
                    break;
                }
                depth -= 1;
            }
            ',' if depth == 0 && comma.is_none() => comma = Some(i),
            _ => {}
        }
    }
    let (c, e) = (comma?, end?);
    Some((
        rest[..c].trim().to_string(),
        rest[c + 1..e].trim().to_string(),
    ))
}

/// Value side of `map[K]V`.
fn map_value(ty: &str) -> Option<String> {
    let rest = ty.strip_prefix("map")?.strip_prefix('[')?;
    let mut depth = 0;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' if depth == 0 => return Some(rest[i + 1..].trim().to_string()),
            ']' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Key side of `map[K]V`.
fn map_key(ty: &str) -> Option<String> {
    let rest = ty.strip_prefix("map")?.strip_prefix('[')?;
    let mut depth = 0;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' if depth == 0 => return Some(rest[..i].trim().to_string()),
            ']' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Strip generic arguments and borrow/box prefixes: `Box[int]` -> `Box`,
/// `&mut link` -> `link`. Used before method/struct table lookups.
fn generic_base(ty: &str) -> &str {
    let t = ty
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches("own ")
        .trim_start_matches("gc ")
        .trim_start_matches('*');
    t.split('[').next().unwrap_or(t)
}

fn fn_end(f: &slang_syntax::FnDecl) -> usize {
    f.body
        .as_ref()
        .map(|b| b.span.offset + b.span.len)
        .unwrap_or(f.span.offset + f.span.len)
}

/// Reconstruct the dotted path around the cursor: every `.ident`
/// segment to the left, plus segments to the right only up to the
/// cursor's own segment. Hovering `c` in `c.config.database` names `c`;
/// hovering `database` names the whole chain.
fn qualified_at(src: &str, offset: usize, _name: &str) -> String {
    let b = src.as_bytes();
    let len = src.len();
    let mut start = offset.min(len);
    while start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_') {
        start -= 1;
    }
    let mut end = offset.min(len);
    while end < len && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
        end += 1;
    }
    // Extend left over every `.ident` segment (`c.config.database`).
    let mut full_start = start;
    loop {
        if full_start >= 1 && b.get(full_start - 1) == Some(&b'.') {
            let mut h = full_start - 1;
            while h > 0 && (b[h - 1].is_ascii_alphanumeric() || b[h - 1] == b'_') {
                h -= 1;
            }
            if h == full_start - 1 {
                break; // lone dot (after a call, a range, ..): stop here.
            }
            full_start = h;
        } else {
            break;
        }
    }
    // Extend right only past segments the cursor has passed: the path
    // names everything up to and including the cursor's own segment.
    let mut full_end = end;
    loop {
        if b.get(full_end) == Some(&b'.') {
            let mut t = full_end + 1;
            while t < len && (b[t].is_ascii_alphanumeric() || b[t] == b'_') {
                t += 1;
            }
            if t == full_end + 1 || offset < full_end + 1 {
                break;
            }
            full_end = t;
        } else {
            break;
        }
    }
    src[full_start..full_end].to_string()
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
        assert_eq!(h.location.unwrap().line, 2);
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

#[cfg(test)]
mod hover_regression_tests {
    use super::*;
    use std::path::PathBuf;

    const WIRE: &str = r#"gc struct Conn {
    config: Config,
    lock: mutex,
}

gc struct Config {
    database: str,
}

gc struct Document {
    fields: [Field],
}

fn mongo_add_database(source: Document, database: str) -> result[Document, str] {
    return ok(source);
}

pub fn command_on(c: Conn, database: str, body: Document, deadline: until)
    -> result[Document, str] {
    mutex_lock(c.lock);
    let command_result = mongo_command_locked(c, database, body, deadline);
    mutex_unlock(c.lock);
    return command_result;
}

fn mongo_command_locked(c: Conn, database: str, body: Document, deadline: until)
    -> result[Document, str] {
    let dbname = c.config.database;
    let prepared = mongo_add_database(body, database);
    guard let command_body = prepared else let e = err_of(prepared) {
        return err(e);
    }
    return ok(command_body);
}
"#;

    fn db() -> (Db, PathBuf) {
        let path = PathBuf::from("wire.sl");
        let d = Db::load_single("mongo", &path, WIRE).unwrap();
        (d, path)
    }

    #[test]
    fn should_infer_call_return_for_let() {
        let (d, path) = db();
        let off = WIRE.find("let prepared").unwrap() + 4;
        let h = d.hover(&path, off).unwrap();
        assert_eq!(h.ty, "result[Document, str]");
    }

    #[test]
    fn should_unwrap_guard_let_binding() {
        let (d, path) = db();
        let off = WIRE.find("guard let command_body").unwrap() + 10;
        let h = d.hover(&path, off).unwrap();
        assert_eq!(h.ty, "Document");
        // use site later in the function resolves too
        let use_off = WIRE.find("return ok(command_body)").unwrap() + 13;
        let h2 = d.hover(&path, use_off).unwrap();
        assert_eq!(h2.ty, "Document");
    }

    #[test]
    fn should_type_err_of_binding_as_str() {
        let (d, path) = db();
        let off = WIRE.find("let e = err_of").unwrap() + 4;
        let h = d.hover(&path, off).unwrap();
        assert_eq!(h.ty, "str");
    }

    #[test]
    fn should_walk_field_chain() {
        let (d, path) = db();
        let off = WIRE.find("c.lock").unwrap() + 2;
        let h = d.hover(&path, off).unwrap();
        assert_eq!(h.ty, "mutex");
        // two-level chain: c.config.database
        let off2 = WIRE.find("c.config.database").unwrap() + 11;
        let h2 = d.hover(&path, off2).unwrap();
        assert_eq!(h2.ty, "str");
    }

    #[test]
    fn should_resolve_same_file_call() {
        let (d, path) = db();
        let src2 =
            "fn a() -> int {\n    return b(1);\n}\nfn b(x: int) -> int {\n    return x;\n}\n";
        let d2 = Db::load_single("m", &PathBuf::from("m.sl"), src2).unwrap();
        let off = src2.find("b(1)").unwrap();
        let h = d2.hover(&PathBuf::from("m.sl"), off).unwrap();
        assert!(h.ty.contains("fn b"), "got {}", h.ty);
        let _ = d;
        let _ = path;
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use std::path::PathBuf;

    const SRC: &str = "import \"json\";\nimport \"time\";\nfn f() {\n    let t = time.wall();\n    let s = json.encode(t);\n}\n";

    fn db() -> (Db, PathBuf) {
        let path = PathBuf::from("main.sl");
        let d = Db::load_single("main", &path, SRC).unwrap();
        (d, path)
    }

    #[test]
    fn should_hover_native_fn() {
        let (d, path) = db();
        let off = SRC.find("time.wall").unwrap() + 6;
        let h = d.hover(&path, off).unwrap();
        assert_eq!(h.ty, "fn wall() -> int");
        assert!(h.location.is_none());
    }

    #[test]
    fn should_infer_native_return() {
        let (d, path) = db();
        let off = SRC.find("let t =").unwrap() + 4;
        let h = d.hover(&path, off).unwrap();
        assert_eq!(h.ty, "int", "got {}", h.ty);
        let off2 = SRC.find("let s =").unwrap() + 4;
        let h2 = d.hover(&path, off2).unwrap();
        assert_eq!(h2.ty, "str", "got {}", h2.ty);
    }

    #[test]
    fn should_reject_unknown_native_fn() {
        let (d, path) = db();
        let src2 = "import \"json\";\nfn f() {\n    let s = json.nope(\"x\");\n}\n";
        let d2 = Db::load_single("main", &PathBuf::from("m.sl"), src2).unwrap();
        let off = src2.find("json.nope").unwrap() + 5;
        assert!(d2.hover(&PathBuf::from("m.sl"), off).is_none());
        let _ = (d, path);
    }
}
