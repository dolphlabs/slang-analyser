//! Import resolution for the analyser Db.
//!
//! Mirrors the search order of `load_import` / `loader_resolve_dir` in
//! slang's `src/loader.c`: a directory next to the importer, then a
//! compiler-provided (native) package, then `stdlib/<path>`, then a
//! `slang.project` pin from the package cache. Unlike the compiler, a
//! miss is lenient (`Missing`) rather than fatal: the IDE resolves what
//! it can and never blinds the workspace over one bad import.

use super::{is_native_package, sanitize_package};
use std::path::{Path, PathBuf};

/// How one import resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedImport {
    /// Source directory to index, with its canonical package name.
    Dir { dir: PathBuf, package: String },
    /// Compiler-provided package: signatures only, no files.
    Native(String),
    /// Nothing matched; `slangc` would refuse this import.
    Missing,
}

/// Resolver inputs. The cache root is threaded in rather than read from
/// the environment so resolution stays unit-testable; `Db` wires the
/// real `$SLANG_CACHE/pkg` (see `slang_project::cache_root`).
pub struct ImportResolver {
    /// Parsed workspace manifest, when the root has one.
    pub project: Option<slang_project::SlProject>,
    /// Resolved stdlib root for slang-source packages, if found.
    pub stdlib: Option<PathBuf>,
    /// `$CACHE/pkg` directory holding fetched pins, if known.
    pub cache_pkg: Option<PathBuf>,
}

impl ImportResolver {
    /// Resolve `ipath` as written in a file inside `from_dir`.
    pub fn resolve(&self, from_dir: &Path, ipath: &str) -> ResolvedImport {
        if ipath.is_empty() {
            return ResolvedImport::Missing;
        }
        // 1. A directory next to the importer. Like upstream, a directory
        // whose name matches a native package only shadows it when the
        // directory actually holds sources, so a data dir named `json`
        // changes nothing.
        let candidate = from_dir.join(ipath);
        if candidate.is_dir() && (!is_native_package(ipath) || dir_has_sl(&candidate)) {
            return ResolvedImport::Dir {
                package: sanitize_package(basename(ipath)),
                dir: candidate,
            };
        }
        // 2. Compiler-provided package: signatures only, no files.
        if is_native_package(ipath) {
            return ResolvedImport::Native(ipath.to_string());
        }
        // 3. Standard library (path-safe shapes only, like stdlib_ipath_ok).
        if is_safe_ipath(ipath) {
            if let Some(root) = &self.stdlib {
                let candidate = root.join(ipath);
                if candidate.is_dir() {
                    return ResolvedImport::Dir {
                        package: sanitize_package(basename(ipath)),
                        dir: candidate,
                    };
                }
            }
        }
        // 4. Project pin, matched by exact import path. The package name
        // is the pin name (== ipath here), not the checkout dir basename:
        // a `dir: src` pin still answers to its import name.
        if let Some(proj) = &self.project {
            if let Some(pin) = proj.find_pin(ipath) {
                if let Some(dir) = self.pin_dir(pin) {
                    if dir.is_dir() {
                        return ResolvedImport::Dir {
                            package: pin.name.clone(),
                            dir,
                        };
                    }
                }
            }
        }
        ResolvedImport::Missing
    }

    /// On-disk directory of a fetched pin: `$CACHE/pkg/<name>/<hash>/[dir]`,
    /// or `None` when the pin has no lock hash yet (`slangc get` pending).
    /// Mirrors `project_cache_dir` + `project_pkg_dir`.
    fn pin_dir(&self, pin: &slang_project::PkgPin) -> Option<PathBuf> {
        let hash = pin.hash.as_ref()?;
        let mut dir = self.cache_pkg.as_ref()?.join(&pin.name).join(hash);
        if let Some(sub) = &pin.dir {
            dir = dir.join(sub);
        }
        Some(dir)
    }
}

/// Last `/`-separated segment of an import path.
pub fn basename(ipath: &str) -> &str {
    ipath.rsplit('/').next().unwrap_or(ipath)
}

/// True when a directory holds at least one `.sl` file, mirroring
/// `dir_has_sl_files` (which is what lets a source dir shadow a native
/// package of the same name).
pub fn dir_has_sl(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten()
        .any(|e| e.file_name().to_string_lossy().ends_with(".sl"))
}

/// Path-safe import shapes for stdlib lookup, mirroring
/// `stdlib_ipath_ok`: non-empty, not absolute, no `..` segments.
pub fn is_safe_ipath(ipath: &str) -> bool {
    if ipath.is_empty() || ipath.starts_with('/') {
        return false;
    }
    !ipath.split('/').any(|seg| seg == "..")
}

/// Locate the stdlib root: `$SLANG_STDLIB`, else beside a `slangc` on
/// `PATH` (`<exe>/stdlib`, `<exe>/../stdlib`, `<exe>/../lib/slang/stdlib`),
/// mirroring `slang_stdlib_pkg`'s root order.
pub fn find_stdlib() -> Option<PathBuf> {
    let env = std::env::var("SLANG_STDLIB").ok().filter(|s| !s.is_empty());
    find_stdlib_in(env.as_deref(), &slangc_on_path())
}

/// Testable core of [`find_stdlib`]: explicit env value + candidate
/// compiler paths instead of ambient environment.
pub fn find_stdlib_in(slang_stdlib_env: Option<&str>, slangc_exes: &[PathBuf]) -> Option<PathBuf> {
    if let Some(env) = slang_stdlib_env.filter(|s| !s.is_empty()) {
        let root = PathBuf::from(env);
        if root.is_dir() {
            return Some(root);
        }
    }
    for exe in slangc_exes {
        // Only real compiler binaries anchor the layout search.
        if !exe.is_file() {
            continue;
        }
        let Some(dir) = exe.parent() else {
            continue;
        };
        for cand in [
            dir.join("stdlib"),
            dir.join("../stdlib"),
            dir.join("../lib/slang/stdlib"),
        ] {
            if cand.is_dir() {
                return Some(cand);
            }
        }
    }
    None
}

/// Every `slangc` executable visible on `PATH`.
pub fn slangc_on_path() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&paths) {
        #[cfg(windows)]
        let name = "slangc.exe";
        #[cfg(not(windows))]
        let name = "slangc";
        let cand = dir.join(name);
        if cand.is_file() {
            out.push(cand);
        }
    }
    out
}

/// Canonicalized visited-set key for a package directory.
pub fn canon(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// `.sl` files directly inside one package directory: exactly one level
/// (like upstream's `load_package_dir`), sorted, skipping `*_test.sl`
/// which belong to `slangc test` only.
pub fn package_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.ends_with(".sl") && !name.ends_with("_test.sl") {
            out.push(p);
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slanganalyser-resolve-test-{}-{suffix}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn should_resolve_local_dir_over_nothing() {
        let root = tmp("local");
        let lib = root.join("lib");
        write(
            &lib.join("u.sl"),
            "pub fn hello() -> str {\n    return \"hi\";\n}\n",
        );
        let r = ImportResolver {
            project: None,
            stdlib: None,
            cache_pkg: None,
        };
        assert_eq!(
            r.resolve(&root, "lib"),
            ResolvedImport::Dir {
                dir: root.join("lib"),
                package: "lib".to_string()
            }
        );
        assert_eq!(r.resolve(&root, "missing"), ResolvedImport::Missing);
    }

    #[test]
    fn should_not_shadow_native_without_sources() {
        let root = tmp("shadow");
        fs::create_dir_all(root.join("json")).unwrap(); // data dir, no .sl
        let r = ImportResolver {
            project: None,
            stdlib: None,
            cache_pkg: None,
        };
        assert_eq!(
            r.resolve(&root, "json"),
            ResolvedImport::Native("json".to_string())
        );
        write(&root.join("json").join("x.sl"), "pub fn x() {}\n");
        assert!(matches!(
            r.resolve(&root, "json"),
            ResolvedImport::Dir { package, .. } if package == "json"
        ));
    }

    #[test]
    fn should_resolve_stdlib_and_reject_unsafe_shapes() {
        let root = tmp("stdlib");
        let std = root.join("stdlib");
        write(&std.join("pg").join("pg.sl"), "pub fn q() {}\n");
        let r = ImportResolver {
            project: None,
            stdlib: Some(std.clone()),
            cache_pkg: None,
        };
        assert!(matches!(
            r.resolve(&root, "pg"),
            ResolvedImport::Dir { package, .. } if package == "pg"
        ));
        assert_eq!(r.resolve(&root, "../evil"), ResolvedImport::Missing);
        assert_eq!(r.resolve(&root, "/abs"), ResolvedImport::Missing);
        assert_eq!(r.resolve(&root, ""), ResolvedImport::Missing);
    }

    #[test]
    fn should_resolve_pin_by_exact_path() {
        use slang_project::{PkgPin, SlProject};
        let root = tmp("pin");
        let hash = "ab".repeat(32);
        let pkgdir = root
            .join("cache")
            .join("pkg")
            .join("ext")
            .join(format!("sha256:{hash}"));
        write(
            &pkgdir.join("ext.sl"),
            "pub fn greet() -> str {\n    return \"hi\";\n}\n",
        );
        let proj = SlProject {
            root: None,
            name: "t".to_string(),
            version: "0.1.0".to_string(),
            pins: vec![PkgPin {
                name: "ext".to_string(),
                git: "u".to_string(),
                tag: "v0.1.0".to_string(),
                dir: None,
                hash: Some(format!("sha256:{hash}")),
            }],
            excludes: vec![],
        };
        let r = ImportResolver {
            project: Some(proj),
            stdlib: None,
            cache_pkg: Some(root.join("cache").join("pkg")),
        };
        match r.resolve(&root, "ext") {
            ResolvedImport::Dir { dir, package } => {
                assert_eq!(package, "ext");
                assert_eq!(dir, pkgdir);
            }
            other => panic!("expected Dir, got {other:?}"),
        }
        // Nested paths never match pins (upstream looks up the full path).
        assert_eq!(r.resolve(&root, "a/ext"), ResolvedImport::Missing);
    }

    #[test]
    fn should_find_stdlib_beside_slangc() {
        let root = tmp("slroot");
        let bin = root.join("bin");
        fs::create_dir_all(bin.join("../lib/slang/stdlib")).unwrap();
        let found = find_stdlib_in(None, &[bin.join("slangc")]);
        assert!(found.is_none()); // exe itself absent; only the dir exists
        write(&bin.join("slangc"), "");
        let found = find_stdlib_in(None, &[bin.join("slangc")]);
        assert_eq!(found, Some(bin.join("../lib/slang/stdlib")));
        let found = find_stdlib_in(Some(bin.join("custom").as_os_str().to_str().unwrap()), &[]);
        assert!(found.is_none()); // env dir absent
    }

    #[test]
    fn should_collect_single_level_package_files() {
        let root = tmp("pkgfiles");
        write(&root.join("a.sl"), "let x = 1;\n");
        write(&root.join("b_test.sl"), "fn test_x() {}\n");
        write(&root.join("sub").join("c.sl"), "let y = 2;\n");
        let files = package_files(&root);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("a.sl"));
    }
}
