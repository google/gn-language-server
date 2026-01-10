# GN Language Server

[![CI](https://github.com/google/gn-language-server/actions/workflows/ci.yml/badge.svg)](https://github.com/google/gn-language-server/actions/workflows/ci.yml)

A [language server](https://microsoft.github.io/language-server-protocol/) for
[GN](https://gn.googlesource.com/gn/),
the build configuration language used in Chromium, Fuchsia, and other projects.

## Features

- Syntax highlighting
- Syntax error reporting
- Context-aware completion and auto-import
- Hover documentation
- Go to definition
- Finding target references
- Following imports
- Following dependencies
- Quick fix
- Sticky scroll with useful lines
- Code lens
- Outline
- Code folding
- Formatting
- Go to the nearest `BUILD.gn` (VSCode only)

## Installing

### VSCode and its derivatives

You can install an extension with prebuilt language server binaries from the
official
[VSCode marketplace](https://marketplace.visualstudio.com/items?itemName=Google.gn)
or the [OpenVSX marketplace](https://open-vsx.org/extension/Google/gn).
Search for "GN Language" in the VSCode's extension window.

![VSCode Marketplace](/docs/screenshots/marketplace.png)

### JetBrains IDEs (IntelliJ, CLion, Android Studio, etc.)

You can install a plugin with prebuilt language server binaries from the
[JetBrains marketplace](https://plugins.jetbrains.com/plugin/29463-gn-language).
Search for "GN Language" in the IDE's plugin window.

### NeoVim

gn-language-server is registered to
[nvim-lspconfig](https://github.com/neovim/nvim-lspconfig),
[mason-registry](https://github.com/mason-org/mason-registry), and
[mason-lspconfig](https://github.com/mason-org/mason-lspconfig.nvim). Thus,
assuming that you have enabled
[mason-lspconfig](https://github.com/mason-org/mason-lspconfig.nvim),
you can install and enable it with the following simple command:

```
:MasonInstall gn-language-server
```

The language server does not provide syntax highlighting though. You can use
[nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter) for this.

### Emacs

Install the language server with [Cargo](https://doc.rust-lang.org/cargo/).

```sh
cargo install --locked gn-language-server
```

Then set up [gn-mode](https://github.com/lashtear/gn-mode) or any syntax
highlighting mode for GN, and add the following to your config:

```elisp
(use-package gn-mode
  :ensure t
  :mode ("\\.gn\\'" "\\.gni\\'")
  :hook (gn-mode . eglot-ensure)
  :config
  (with-eval-after-load 'eglot
    (add-to-list 'eglot-server-programs '(gn-mode . ("gn-language-server")))))
```

### Other Editors/IDEs

You can download prebuilt language server binaries from
[GitHub releases page](https://github.com/google/gn-language-server/releases?q=prerelease%3Afalse).

Alternatively, you can build the language server from source with
[Cargo](https://doc.rust-lang.org/cargo/).

```sh
cargo install --locked gn-language-server
```

Then follow editor-specific instructions to install the language server.

## Gallery

### Syntax highlighting

![Syntax highlighting](/docs/screenshots/syntax_highlighting.png)

### Completion and auto-import

![Completion and auto-import](/docs/screenshots/completion.png)

### Hover documentation

![Hover documentation](/docs/screenshots/hover_documentation.png)

### Go to definition

![Go to definition](/docs/screenshots/go_to_definition.png)

### Following imports

![Following imports](/docs/screenshots/following_imports.png)

### Following dependencies

![Following dependencies](/docs/screenshots/following_dependencies.png)

### Quick fix

![Quick fix](/docs/screenshots/quick_fix.png)

### Sticky scroll with useful lines

![Sticky scroll with useful lines](/docs/screenshots/sticky_scroll.png)

### Code lens

![Code lens](/docs/screenshots/code_lens.png)

### Outline

![Outline](/docs/screenshots/outline.png)

### Code folding

![Code folding](/docs/screenshots/code_folding.png)

## Building from source

### Language server binary

```sh
cargo build --release
```

### VSCode extension

```sh
cd vscode-gn
npm install
npm run build
npm run package
```

## Versioning scheme

We use the versioning scheme recommended by the
[VSCode's official documentation](https://code.visualstudio.com/api/working-with-extensions/publishing-extension#prerelease-extensions).
That is:

- Pre-release versions are `1.<odd>.x`
- Release versions are `1.<even>.x`

For Rust releases on crates.io, we use `1.<even>.x-prerelease` for pre-releases.

## Architecture

For an overview of the project's architecture, see [ARCHITECTURE.md](./ARCHITECTURE.md).

## Disclaimer

This is not an officially supported Google product. This project is not
eligible for the [Google Open Source Software Vulnerability Rewards
Program](https://bughunters.google.com/open-source-security).
