//! `slang.project` + `slang.lock` model, mirroring `src/project.c`.
//!
//! Format (comments start with `#`):
//! ```text
//! name myserver
//! version 0.1.0
//! pkg foo git https://github.com/dolphlabs/foo tag v0.1.0
//! pkg bar git https://github.com/dolphlabs/bar tag v0.2.0 dir src
//! ```

use std::path::{Path, PathBuf};

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
        let fail = || ProjectError {
            line: line_no,
            message: "expected 'name <ident>' | 'version <value>' | 'pkg <name> git <url> tag <tag> [dir <subdir>]'"
                .to_string(),
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
                            message: format!("package '{name}' needed twice with different sources"),
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
    p.root = Some(root.to_path_buf());
    Ok(p)
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
}
