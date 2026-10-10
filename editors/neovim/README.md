# slang in Neovim (0.11+)

No plugin needed. Put `slanganalyser` on your `PATH` (`cargo build --release -p slanganalyser`),
then register the filetype and server:

```lua
vim.filetype.add({ extension = { sl = "slang" } })

vim.lsp.config("slanganalyser", {
    cmd = { "slanganalyser" },
    filetypes = { "slang" },
    root_markers = { "slang.project" },
})
vim.lsp.enable("slanganalyser")
```

You get hover with inferred types, goto-definition, diagnostics,
formatting (`vim.lsp.buf.format()`), inlay hints, and semantic tokens.
Treesitter highlighting for `.sl` does not exist yet; the TextMate grammar
in `editors/vscode/syntaxes/` documents the token set for a future port.
