# GN Language Server

[![CI](https://github.com/google/gn-language-server/actions/workflows/ci.yml/badge.svg)](https://github.com/google/gn-language-server/actions/workflows/ci.yml)

A [language server](https://microsoft.github.io/language-server-protocol/) for
[GN](https://gn.googlesource.com/gn/),
the build configuration language used in Chromium, Fuchsia, and other projects.

## Features

- Syntax highlighting
- Syntax error reporting
- Context-aware completion
- Hover documentation
- Go to definition
- Finding target references
- Following imports
- Following dependencies
- Sticky scroll with useful lines
- Outline
- Code folding
- Formatting
- Go to the nearest `BUILD.gn` (VSCode only)

## Installing

### VSCode

You can install from the official [VSCode marketplace](https://marketplace.visualstudio.com/items?itemName=Google.gn)
or the [OpenVSX marketplace](https://open-vsx.org/extension/Google/gn).
Search for "GN Language" in the VSCode's extension window.

![VSCode Marketplace](/docs/screenshots/marketplace.png)

### Other Editors/IDEs

You can install the language server with [Cargo](https://doc.rust-lang.org/cargo/).

```sh
cargo install gn-language-server
```

## Gallery

### Syntax highlighting

![Syntax highlighting](/docs/screenshots/syntax_highlighting.png)

### Completion

![Completion](/docs/screenshots/completion.png)

### Hover documentation

![Hover documentation](/docs/screenshots/hover_documentation.png)

### Go to definition

![Go to definition](/docs/screenshots/go_to_definition.png)

### Following imports

![Following imports](/docs/screenshots/following_imports.png)

### Following dependencies

![Following dependencies](/docs/screenshots/following_dependencies.png)

### Sticky scroll with useful lines

![Sticky scroll with useful lines](/docs/screenshots/sticky_scroll.png)

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

## Architecture

For an overview of the project's architecture, see [ARCHITECTURE.md](./ARCHITECTURE.md).

## Disclaimer

This is not an officially supported Google product. This project is not
eligible for the [Google Open Source Software Vulnerability Rewards
Program](https://bughunters.google.com/open-source-security).
