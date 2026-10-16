use crate::convert::{encode_semantic, line_starts, offset_to_position, position_to_offset};
use slang_hir::Db;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

pub struct Backend {
    client: Client,
    root: Arc<RwLock<Option<PathBuf>>>,
    db: Arc<RwLock<Option<Db>>>,
    docs: Arc<RwLock<HashMap<Url, String>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            root: Arc::new(RwLock::new(None)),
            db: Arc::new(RwLock::new(None)),
            docs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn ensure_db(&self, file: &std::path::Path) {
        if self.db.read().await.is_some() {
            return;
        }
        let root = self
            .root
            .read()
            .await
            .clone()
            .or_else(|| file.parent().and_then(slang_project::find_root));
        if let Some(root) = root {
            match Db::load(&root) {
                Ok(db) => {
                    *self.db.write().await = Some(db);
                    *self.root.write().await = Some(root);
                }
                Err(e) => {
                    self.client
                        .log_message(MessageType::ERROR, format!("slanganalyser: {e}"))
                        .await;
                }
            }
        }
    }

    async fn publish_for(&self, uri: Url, text: &str) {
        let diags = diagnostics(text);
        self.client.publish_diagnostics(uri, diags, None).await;
    }
}

fn uri_to_path(uri: &Url) -> Option<PathBuf> {
    if uri.scheme() != "file" {
        return None;
    }
    uri.to_file_path().ok()
}

fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let starts = line_starts(text);
    let mut out = Vec::new();
    let toks = match slang_syntax::lex(text) {
        Ok(t) => t,
        Err(e) => {
            out.push(lex_diagnostic(&starts, text, e.line, e.offset, &e.message));
            return out;
        }
    };
    let (_, errors) = slang_syntax::parse(toks);
    for e in errors {
        let (line, ch) = offset_to_position(&starts, text, e.offset);
        out.push(Diagnostic {
            range: Range {
                start: Position {
                    line,
                    character: ch,
                },
                end: Position {
                    line,
                    character: ch + 1,
                },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("slanganalyser".to_string()),
            message: e.message,
            ..Default::default()
        });
    }
    out
}

fn lex_diagnostic(
    starts: &[usize],
    text: &str,
    line: u32,
    offset: usize,
    message: &str,
) -> Diagnostic {
    // Lexer lines are 1-based; clamp into the file for robustness.
    let (l, ch) = offset_to_position(starts, text, offset);
    let line = line.saturating_sub(1).min(l);
    Diagnostic {
        range: Range {
            start: Position {
                line,
                character: ch,
            },
            end: Position {
                line,
                character: ch + 1,
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("slanganalyser".to_string()),
        message: message.to_string(),
        ..Default::default()
    }
}

fn legend() -> SemanticTokensLegend {
    let types = [
        "keyword",
        "type",
        "function",
        "variable",
        "string",
        "number",
        "comment",
        "operator",
        "method",
        "enumMember",
    ];
    SemanticTokensLegend {
        token_types: types.into_iter().map(SemanticTokenType::new).collect(),
        token_modifiers: vec![],
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        if let Some(uri) = params.root_uri {
            if let Some(path) = uri_to_path(&uri) {
                let root = slang_project::find_root(&path).unwrap_or(path);
                match Db::load(&root) {
                    Ok(db) => {
                        *self.db.write().await = Some(db);
                        *self.root.write().await = Some(root);
                    }
                    Err(e) => {
                        self.client
                            .log_message(MessageType::ERROR, format!("slanganalyser: {e}"))
                            .await;
                    }
                }
            }
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: legend(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            ..Default::default()
                        },
                    ),
                ),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "slanganalyser ready")
            .await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        if let Some(path) = uri_to_path(&uri) {
            self.ensure_db(&path).await;
        }
        self.docs.write().await.insert(uri.clone(), text.clone());
        self.publish_for(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().next() {
            let text = change.text;
            self.docs.write().await.insert(uri.clone(), text.clone());
            if let Some(path) = uri_to_path(&uri) {
                self.ensure_db(&path).await;
                if let Some(db) = self.db.write().await.as_mut() {
                    db.update_file(&path, text.clone());
                }
            }
            self.publish_for(uri, &text).await;
        }
    }

    async fn hover(&self, params: HoverParams) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let docs = self.docs.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        let Some(path) = uri_to_path(&uri) else {
            return Ok(None);
        };
        self.ensure_db(&path).await;
        let db_guard = self.db.read().await;
        // Without a project, fall back to a single-file database.
        let owned;
        let db_ref = match db_guard.as_ref() {
            Some(db) => db,
            None => match Db::load_single("main", &path, &text) {
                Ok(db) => {
                    owned = db;
                    &owned
                }
                Err(_) => return Ok(None),
            },
        };
        let starts = line_starts(&text);
        let offset = position_to_offset(&starts, &text, pos.line, pos.character);
        let Some(h) = db_ref.hover(&path, offset) else {
            return Ok(None);
        };
        let mut value = format!("```slang\n{}\n```", h.ty);
        if !h.docs.is_empty() {
            value.push_str(&format!("\n\n{}", h.docs));
        }
        if let Some(loc) = &h.location {
            value.push_str(&format!(
                "\n\ndefined at `{}`:{}",
                loc.file.display(),
                loc.line
            ));
        }
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let docs = self.docs.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        let Some(path) = uri_to_path(&uri) else {
            return Ok(None);
        };
        self.ensure_db(&path).await;
        let db_guard = self.db.read().await;
        let owned;
        let db_ref = match db_guard.as_ref() {
            Some(db) => db,
            None => match Db::load_single("main", &path, &text) {
                Ok(db) => {
                    owned = db;
                    &owned
                }
                Err(_) => return Ok(None),
            },
        };
        let starts = line_starts(&text);
        let offset = position_to_offset(&starts, &text, pos.line, pos.character);
        let Some(loc) = db_ref.goto_def(&path, offset) else {
            return Ok(None);
        };
        let target: Url = match Url::from_file_path(&loc.file) {
            Ok(u) => u,
            Err(_) => return Ok(None),
        };
        let line = loc.line.saturating_sub(1);
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: target,
            range: Range {
                start: Position { line, character: 0 },
                end: Position { line, character: 0 },
            },
        })))
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let docs = self.docs.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        let formatted = match slang_fmt::format(&text) {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };
        if formatted == text {
            return Ok(None);
        }
        let starts = line_starts(&text);
        let last = starts.len().saturating_sub(1) as u32;
        let end_char = text[starts[last as usize]..].encode_utf16().count() as u32;
        Ok(Some(vec![TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: last,
                    character: end_char,
                },
            },
            new_text: formatted,
        }]))
    }

    async fn inlay_hint(
        &self,
        params: InlayHintParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let docs = self.docs.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        let Some(path) = uri_to_path(&uri) else {
            return Ok(None);
        };
        self.ensure_db(&path).await;
        let db_guard = self.db.read().await;
        let owned;
        let db_ref = match db_guard.as_ref() {
            Some(db) => db,
            None => match Db::load_single("main", &path, &text) {
                Ok(db) => {
                    owned = db;
                    &owned
                }
                Err(_) => return Ok(None),
            },
        };
        let starts = line_starts(&text);
        let hints = db_ref
            .inlay_hints(&path)
            .into_iter()
            .map(|h| {
                let (line, character) = offset_to_position(&starts, &text, h.offset);
                InlayHint {
                    position: Position { line, character },
                    label: InlayHintLabel::String(h.label),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: None,
                    padding_left: None,
                    padding_right: None,
                    data: None,
                }
            })
            .collect();
        Ok(Some(hints))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> tower_lsp::jsonrpc::Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let docs = self.docs.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        let Ok((toks, comments)) = slang_syntax::lex_with_comments(&text) else {
            return Ok(None);
        };
        // An identifier followed by `(` names a call: a function token.
        let call_idx: std::collections::HashSet<usize> = toks
            .windows(2)
            .enumerate()
            .filter(|(_, w)| {
                w[0].kind == slang_syntax::TokenKind::Ident
                    && w[1].kind == slang_syntax::TokenKind::LParen
            })
            .map(|(i, _)| i)
            .collect();
        let enc = encode_semantic(&text, &toks, &comments, &|i| call_idx.contains(&i));
        let mut data = Vec::with_capacity(enc.len());
        let (mut pl, mut ps) = (0u32, 0u32);
        for t in enc {
            data.push(tower_lsp::lsp_types::SemanticToken {
                delta_line: t.line - pl,
                delta_start: if t.line == pl { t.start - ps } else { t.start },
                length: t.length,
                token_type: t.kind,
                token_modifiers_bitset: 0,
            });
            pl = t.line;
            ps = t.start;
        }
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }
}
