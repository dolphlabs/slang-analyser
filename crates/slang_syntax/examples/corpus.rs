use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().map(|x| x == "sl").unwrap_or(false) {
            out.push(p);
        }
    }
}

fn main() {
    let mut files = Vec::new();
    for arg in std::env::args().skip(1) {
        collect(Path::new(&arg), &mut files);
    }
    files.sort();
    let mut total_files = 0;
    let mut total_errs = 0;
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                println!("{}: UNREADABLE {}", path.display(), e);
                continue;
            }
        };
        total_files += 1;
        let toks = match slang_syntax::lex(&src) {
            Ok(t) => t,
            Err(e) => {
                println!(
                    "{}: LEX ERROR line {}: {}",
                    path.display(),
                    e.line,
                    e.message
                );
                total_errs += 1;
                continue;
            }
        };
        let (_file, errors) = slang_syntax::parse(toks);
        if !errors.is_empty() {
            total_errs += errors.len();
            println!("{}: {} parse errors", path.display(), errors.len());
            for e in errors.iter().take(4) {
                println!("    line {}: {}", e.line, e.message);
            }
        }
    }
    println!("--- files: {}, total errors: {}", total_files, total_errs);
}
