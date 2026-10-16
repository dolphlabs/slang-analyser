//! `slang.project` + `slang.lock` model, mirroring `src/project.c`.
//!
//! Format (comments start with `#`):
//! ```text
//! name myserver
//! version 0.1.0
//! exclude bench
//! pkg foo git https://github.com/dolphlabs/foo tag v0.1.0
//! pkg bar git https://github.com/dolphlabs/bar tag v0.2.0 dir src
//! ```
//!
//! Lock lines (`slang.lock`) look like:
//! ```text
//! foo sha256:<64 hex> git https://github.com/dolphlabs/foo tag v0.1.0
//! ```

use std::path::{Path, PathBuf};

/// Maximum `exclude` lines per project, mirroring MAX_PACKAGE_EXCLUDES.
pub const MAX_EXCLUDES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkgPin {
    pub name: String,
    pub git: String,
    pub tag: String,
    pub dir: Option<String>,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlProject {
    pub root: Option<PathBuf>,
    pub name: String,
    pub version: String,
    pub pins: Vec<PkgPin>,
    pub excludes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ProjectError {}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A pin `dir`: relative path inside the repo; refuses `..`, leading `/`,
/// empty segments. Mirrors `is_pkg_subdir` in `src/project.c`.
fn is_pkg_subdir(s: &str) -> bool {
    if s.is_empty() || s.starts_with('/') {
        return false;
    }
    for seg in s.split('/') {
        if seg.is_empty() {
            return false;
        }
        if seg == "." || seg == ".." {
            return false;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            return false;
        }
    }
    true
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => line[..i].trim_end(),
        None => line.trim_end(),
    }
}

/// Parse `slang.project` file contents.
pub fn parse_project(src: &str) -> Result<SlProject, ProjectError> {
    let mut out = SlProject::default();
    for (idx, raw) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let fail = || {
            ProjectError {
            line: line_no,
            message: "expected 'name <ident>' | 'version <value>' | 'exclude <subdir>' | 'pkg <name> git <url> tag <tag> [dir <subdir>]'"
                .to_string(),
        }
        };
        match words.next().ok_or_else(fail)? {
            "name" => {
                let v = words.next().ok_or_else(fail)?;
                if words.next().is_some() || !is_ident(v) {
                    return Err(fail());
                }
                if !out.name.is_empty() {
                    return Err(ProjectError {
                        line: line_no,
                        message: "duplicate name".to_string(),
                    });
                }
                out.name = v.to_string();
            }
            "version" => {
                let v = words.next().ok_or_else(fail)?;
                if words.next().is_some() {
                    return Err(fail());
                }
                if !out.version.is_empty() {
                    return Err(ProjectError {
                        line: line_no,
                        message: "duplicate version".to_string(),
                    });
                }
                out.version = v.to_string();
            }
            "pkg" => {
                let name = words.next().ok_or_else(fail)?;
                let gkw = words.next().ok_or_else(fail)?;
                let git = words.next().ok_or_else(fail)?;
                let tkw = words.next().ok_or_else(fail)?;
                let tag = words.next().ok_or_else(fail)?;
                if !is_ident(name) || gkw != "git" || tkw != "tag" {
                    return Err(fail());
                }
                let mut dir: Option<String> = None;
                if let Some(dkw) = words.next() {
                    let d = words.next().ok_or_else(fail)?;
                    if dkw != "dir" || !is_pkg_subdir(d) || words.next().is_some() {
                        return Err(fail());
                    }
                    dir = Some(d.to_string());
                }
                if let Some(e) = out.pins.iter().find(|p| p.name == name) {
                    if e.git != git || e.tag != tag || e.dir != dir {
                        return Err(ProjectError {
                            line: line_no,
                            message: format!(
                                "package '{name}' needed twice with different sources"
                            ),
                        });
                    }
                } else {
                    out.pins.push(PkgPin {
                        name: name.to_string(),
                        git: git.to_string(),
                        tag: tag.to_string(),
                        dir,
                        hash: None,
                    });
                }
            }
            "exclude" => {
                let d = words.next().ok_or_else(fail)?;
                if words.next().is_some() || !is_pkg_subdir(d) {
                    return Err(ProjectError {
                        line: line_no,
                        message: format!(
                            "expected 'exclude <subdir>' (up to {MAX_EXCLUDES} paths)"
                        ),
                    });
                }
                if out.excludes.iter().any(|e| e == d) {
                    return Err(ProjectError {
                        line: line_no,
                        message: format!("duplicate exclude '{d}'"),
                    });
                }
                if out.excludes.len() >= MAX_EXCLUDES {
                    return Err(ProjectError {
                        line: line_no,
                        message: format!(
                            "expected 'exclude <subdir>' (up to {MAX_EXCLUDES} paths)"
                        ),
                    });
                }
                out.excludes.push(d.to_string());
            }
            other => {
                return Err(ProjectError {
                    line: line_no,
                    message: format!("unknown field '{other}'"),
                })
            }
        }
    }
    Ok(out)
}

/// Walk up from `start_dir` looking for `slang.project`, like `project_find_root`.
pub fn find_root(start_dir: &Path) -> Option<PathBuf> {
    let mut cur = start_dir.to_path_buf();
    loop {
        if cur.join("slang.project").is_file() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Load and validate a project rooted at `root` (requires name + version).
pub fn load(root: &Path) -> Result<SlProject, ProjectError> {
    let path = root.join("slang.project");
    let src = std::fs::read_to_string(&path).map_err(|e| ProjectError {
        line: 0,
        message: format!("cannot read {}: {e}", path.display()),
    })?;
    let mut p = parse_project(&src)?;
    if p.name.is_empty() || p.version.is_empty() {
        return Err(ProjectError {
            line: 0,
            message: "missing the name and/or version lines; a project file needs both, e.g.\n  name app\n  version 0.1.0"
                .to_string(),
        });
    }
    // Merge slang.lock when present, like project_load does. A missing or
    // unreadable lock just leaves hashes unset (`slangc get` pending);
    // a malformed one fails the load, like upstream.
    let lock_path = root.join("slang.lock");
    if let Ok(lock_src) = std::fs::read_to_string(&lock_path) {
        if let Err(e) = apply_lock(&mut p, &lock_src) {
            return Err(ProjectError {
                line: e.line,
                message: format!("{}: {}", lock_path.display(), e.message),
            });
        }
    }
    p.root = Some(root.to_path_buf());
    Ok(p)
}

/// Merge a `slang.lock` file into an already-parsed project: each line is
/// `<name> sha256:<64 hex> [git <url> tag <tag> [dir <subdir>]]`.
/// Mirrors `parse_lock_file` in `src/project.c`.
pub fn apply_lock(project: &mut SlProject, src: &str) -> Result<(), ProjectError> {
    for (idx, raw) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let fail = || ProjectError {
            line: line_no,
            message: "expected '<name> sha256:<hex> [git <url> tag <tag> [dir <subdir>]]'"
                .to_string(),
        };
        let name = words.next().ok_or_else(fail)?;
        let hash = words.next().ok_or_else(fail)?;
        if !is_sha256(hash) {
            return Err(ProjectError {
                line: line_no,
                message: "invalid hash".to_string(),
            });
        }
        // Optional `git <url> tag <tag> [dir <subdir>]` trailer.
        let mut git: Option<String> = None;
        let mut tag: Option<String> = None;
        let mut dir: Option<String> = None;
        if let Some(gkw) = words.next() {
            let g = words.next().ok_or_else(fail)?;
            let tkw = words.next().ok_or_else(fail)?;
            let t = words.next().ok_or_else(fail)?;
            if gkw != "git" || tkw != "tag" {
                return Err(fail());
            }
            git = Some(g.to_string());
            tag = Some(t.to_string());
            if let Some(dkw) = words.next() {
                let d = words.next().ok_or_else(fail)?;
                if dkw != "dir" || !is_pkg_subdir(d) || words.next().is_some() {
                    return Err(fail());
                }
                dir = Some(d.to_string());
            }
        }
        match project.pins.iter_mut().find(|p| p.name == name) {
            Some(pin) => {
                if pin.hash.is_some() {
                    return Err(ProjectError {
                        line: line_no,
                        message: format!("duplicate lock entry '{name}'"),
                    });
                }
                if let (Some(g), Some(t)) = (&git, &tag) {
                    if pin.git != *g || pin.tag != *t || pin.dir != dir {
                        return Err(ProjectError {
                            line: line_no,
                            message: format!(
                                "lock for '{name}' disagrees with slang.project"
                            ),
                        });
                    }
                }
                pin.hash = Some(hash.to_string());
            }
            None => {
                let (g, t) = match (git, tag) {
                    (Some(g), Some(t)) => (g, t),
                    _ => {
                        return Err(ProjectError {
                            line: line_no,
                            message: format!(
                                "lock entry '{name}' is not in slang.project"
                            ),
                        })
                    }
                };
                project.pins.push(PkgPin {
                    name: name.to_string(),
                    git: g,
                    tag: t,
                    dir,
                    hash: Some(hash.to_string()),
                });
            }
        }
    }
    Ok(())
}

fn is_sha256(s: &str) -> bool {
    let hex = s.strip_prefix("sha256:").unwrap_or("");
    hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

impl SlProject {
    /// Find a pin by package name, mirroring `project_find_pin`.
    pub fn find_pin(&self, name: &str) -> Option<&PkgPin> {
        self.pins.iter().find(|p| p.name == name)
    }

    /// True when `rel` (a repository-relative path with `/` separators)
    /// falls under one of this manifest's `exclude` dirs.
    pub fn is_excluded(&self, rel: &str) -> bool {
        self.excludes.iter().any(|ex| {
            rel == ex || rel.starts_with(&format!("{ex}/"))
        })
    }
}

/// Package-cache root: `$SLANG_CACHE`, else `~/.cache/slang`.
/// Returns `None` (never an error) when neither resolves, mirroring
/// `cache_pkg_real`'s tolerance in the loader path.
pub fn cache_root() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("SLANG_CACHE") {
        if !env.is_empty() {
            return Some(PathBuf::from(env));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".cache/slang"))
}

/// On-disk directory of a fetched pin: `$CACHE/pkg/<name>/<hash>/[dir]`.
/// Returns `None` when the pin has no lock hash yet (`slangc get` pending).
/// Mirrors `project_cache_dir` + `project_pkg_dir`.
pub fn pin_pkg_dir(pin: &PkgPin) -> Option<PathBuf> {
    let hash = pin.hash.as_ref()?;
    let mut dir = cache_root()?.join("pkg").join(&pin.name).join(hash);
    if let Some(sub) = &pin.dir {
        dir = dir.join(sub);
    }
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_minimal_project() {
        let p = parse_project("name hello\nversion 0.1.0\n").unwrap();
        assert_eq!(p.name, "hello");
        assert_eq!(p.version, "0.1.0");
        assert!(p.pins.is_empty());
    }

    #[test]
    fn should_parse_pkg_with_dir() {
        let p = parse_project(
            "name app\nversion 0.1.0\npkg bar git https://example.com/bar tag v0.2.0 dir src\n",
        )
        .unwrap();
        assert_eq!(p.pins.len(), 1);
        assert_eq!(p.pins[0].dir.as_deref(), Some("src"));
    }

    #[test]
    fn should_reject_bad_dir() {
        assert!(parse_project("name a\nversion 0.1.0\npkg x git u tag t dir ../evil\n").is_err());
    }

    #[test]
    fn should_reject_unknown_field() {
        assert!(parse_project("name a\nversion 0.1.0\nweird 1\n").is_err());
    }

    #[test]
    fn should_parse_excludes() {
        let p = parse_project("name a\nversion 0.1.0\nexclude bench\nexclude docs\n").unwrap();
        assert_eq!(p.excludes, vec!["bench".to_string(), "docs".to_string()]);
        assert!(p.is_excluded("bench"));
        assert!(p.is_excluded("bench/foo.sl"));
        assert!(p.is_excluded("docs"));
        assert!(!p.is_excluded("benchmarks"));
        assert!(!p.is_excluded("src/main.sl"));
    }

    #[test]
    fn should_reject_duplicate_exclude() {
        assert!(parse_project("name a\nversion 0.1.0\nexclude bench\nexclude bench\n").is_err());
    }

    #[test]
    fn should_reject_bad_exclude() {
        assert!(parse_project("name a\nversion 0.1.0\nexclude ../evil\n").is_err());
    }

    #[test]
    fn should_apply_lock_hashes() {
        let mut p =
            parse_project("name a\nversion 0.1.0\npkg foo git https://x/y tag v0.1.0 dir src\n")
                .unwrap();
        assert!(p.pins[0].hash.is_none());
        apply_lock(
            &mut p,
            "foo sha256:461da723122f9d7569c45cab1e57575ff94d0bea7b974db7a95f0ad20ae1fa4a git https://x/y tag v0.1.0 dir src\n",
        )
        .unwrap();
        assert!(p.pins[0].hash.as_ref().unwrap().starts_with("sha256:461d"));
    }

    #[test]
    fn should_reject_lock_for_unknown_pin() {
        let mut p = parse_project("name a\nversion 0.1.0\n").unwrap();
        assert!(apply_lock(&mut p, "ghost sha256:0000000000000000000000000000000000000000000000000000000000000000\n").is_err());
    }

    #[test]
    fn should_reject_bad_lock_hash() {
        let mut p = parse_project("name a\nversion 0.1.0\npkg foo git u tag t\n").unwrap();
        assert!(apply_lock(&mut p, "foo notahash\n").is_err());
    }
}
