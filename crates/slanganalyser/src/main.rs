//! `slanganalyser`: language server for slang (`.sl`).
//!
//! Workspace root is the directory containing `slang.project`.
//! Serves hover (inferred types + docs), goto-definition, diagnostics,
//! formatting, inlay hints, and semantic tokens for editor themes.

mod backend;
mod convert;

use backend::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
