# GN Language Server

A [language server](https://microsoft.github.io/language-server-protocol/) for
[GN](https://gn.googlesource.com/gn/),
the build configuration language used for Chromium.

## Features

### Syntax highlighting

![Syntax highlighting](./docs/screenshots/syntax_highlighting.png)

### Completion

![Completion](./docs/screenshots/completion.png)

### Hover documentation

![Hover documentation](./docs/screenshots/hover_documentation.png)

### Go to definition

![Go to definition](./docs/screenshots/go_to_definition.png)

### Following imports

![Following imports](./docs/screenshots/following_imports.png)

### Following dependencies

![Following dependencies](./docs/screenshots/following_dependencies.png)

### Sticky scroll with useful lines

![Sticky scroll with useful lines](./docs/screenshots/sticky_scroll.png)

### Outline

![Outline](./docs/screenshots/outline.png)

### Code folding

![Code folding](./docs/screenshots/code_folding.png)

## Building

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

## Disclaimer

This is not an officially supported Google product. This project is not
eligible for the [Google Open Source Software Vulnerability Rewards
Program](https://bughunters.google.com/open-source-security).
