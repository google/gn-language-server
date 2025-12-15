# GN Language Server Architecture

This document outlines the architecture and key design decisions of the GN Language Server.

## 1. Overview

The primary goal of this language server is to provide a fast and useful IDE experience for the GN build system. It is written in **Rust** and built on top of several key libraries:
-   **`tower-lsp`**: For the core Language Server Protocol framework.
-   **`pest`**: For parsing the GN language based on a formal grammar.
-   **`tokio`**: For asynchronous I/O and concurrency.

## 2. Key Design Decisions

Several core design decisions shape the server's behavior and performance.

### Configuration-Agnostic Analysis (Ignoring `args.gn`)

The most fundamental design choice is that **the server does not read `args.gn` files**. It analyzes the build files in a configuration-agnostic way, without knowing the final values of build arguments for any specific output directory (e.g., `out/Debug`).

This is a deliberate trade-off that prioritizes simplicity and a holistic editing experience over configuration-specific precision.

**Pros:**
-   **Simplicity & Decoupling**: The server does not need to track which of the potentially many build directories is "active," simplifying state management and user configuration. It works out of the box.
-   **Holistic Code View**: By not evaluating conditionals, the server analyzes all possible code paths. This is ideal for developers who need to understand and refactor code that spans multiple configurations (e.g., `if (is_win)` and `if (is_linux)`).
-   **Performance & Stability**: The analysis is stable and depends only on the contents of the `.gn` and `.gni` source files. It avoids the high cost of a full build config evaluation. This means the analysis can be done quickly without costly evaluations like `exec_script()`.

**Cons:**
-   **Inaccurate Semantic Analysis**: The server's understanding is incomplete. It cannot know which code paths are "active" or "dead" for a specific build, nor can it compute the final value of any variable that depends on a build argument.
-   **Ambiguous Results**: LSP features may provide ambiguous results. For example, "Go to Definition" on a variable may navigate to multiple assignments across different conditional blocks.
-   **Diagnostic Mismatches**: The server's error checking may differ from `gn check`. It might produce false positives for code in an inactive block or miss errors in code that is currently disabled.

### Single-Pass Caching & On-Demand Scope Resolution

The analyzer employs a unified caching strategy combined with on-demand scope construction.

-   **Per-File Analysis (`AnalyzedFile`)**: Each file (`.gn` or `.gni`) is parsed and analyzed independently to extract local information:
    -   **Abstract Syntax Tree (AST)**: The structural representation of the code.
    -   **Exports**: Variables, templates, and targets defined at the top level.
    -   **Links**: File paths and target labels referenced in the file.
    -   **Symbols**: A simple index of symbols for document outline.
    This result is wrapped in an `AnalyzedFile` and cached. If a file hasn't changed, this cached result is reused instantly.

-   **On-Demand Scope Building (`analyze_at`)**: When a feature requires a full semantic understanding of a specific location (e.g., "Go to Definition" or "Completion" at a cursor position), the analyzer dynamically constructs an `Environment`.
    -   It starts from the target file and recursively gathers exports from all imported files (transitive imports).
    -   It combines these exports with the local definitions available at that specific position in the code.
    -   This process is fast because it relies on the pre-computed, cached `AnalyzedFile` structures, avoiding the need to re-parse or re-analyze the dependencies.

### Caching and Performance

The server uses a freshness-checking mechanism to avoid re-analyzing unchanged files and their dependencies. The `CacheConfig` struct allows the server to differentiate between interactive requests (which might trigger a shallow update) and background requests.

### Concurrency

The server is built on `tokio` to handle multiple LSP requests concurrently without blocking. Shared state is managed safely across threads. `DocumentStorage` uses `Arc<Mutex<T>>`, while the `Analyzer` uses `RwLock` and fine-grained internal locking to allow concurrent analysis of multiple files.

### Background Indexing

For workspace-wide features like "Find All References," a complete view of the project is necessary. When a `.gn` file is first opened, a background task is spawned to walk the entire workspace directory, analyzing every `.gn` and `.gni` file. This populates the analyzer's cache. The indexer skips build output directories by checking for the presence of an `args.gn` file. Subsequent requests that need this global view can then wait for the indexing task to complete.

### Interaction with `gn` CLI

The server is designed to be mostly standalone but relies on the `gn` command-line tool for specific features where re-implementing the logic would be impractical.
-   **Location**: It has a built-in strategy to find the `gn` binary, looking in common prebuilt directories within a Chromium or Fuchsia checkout, or falling back to the system `PATH`.
-   **Formatting**: Document formatting is implemented by shelling out to `gn format --stdin`, leveraging the canonical formatter directly.

## 3. Core Components

The server is designed with a modular architecture, separating concerns into distinct components.

### Server (`src/server/mod.rs`)

This is the main entry point of the application. It initializes the server, manages the LSP request/response lifecycle, and holds the shared state of the application, including the document storage and the analyzer.

### Document Storage (`src/common/storage.rs`)

This component acts as a cache for file contents. It distinguishes between:
1.  Files currently open and being edited in the client (in-memory).
2.  Files on disk that are part of the workspace but not open for editing.

It uses a combination of LSP document versions (for in-memory files) and file system modification timestamps (for on-disk files) to determine if a file is "fresh" or needs to be re-read.

### Parser (`src/parser/`)

The parser is responsible for turning raw text into a structured representation.
-   **Grammar (`src/parser/gn.pest`)**: A formal grammar defines the syntax of the GN language. This makes the parser predictable and easy to maintain.
-   **AST (`src/parser/mod.rs`, `src/parser/parse.rs`)**: The raw parse tree from `pest` is transformed into a more ergonomic Abstract Syntax Tree (AST). The AST nodes provide methods for easy traversal and inspection, forming the input for the semantic analyzer.

### Semantic Analyzer (`src/analyzer/`)

The analyzer is the brain of the language server. It consumes the AST and builds a rich semantic understanding of the code.

-   **Workspace Context**: The server establishes the workspace context by first finding the root directory, identified by a `.gn` file. This root path is essential for resolving source-absolute paths (e.g., `//path/to/file.cc`). The `WorkspaceAnalyzer` manages the state for a specific workspace, including the build configuration loaded from `build/config/BUILDCONFIG.gn`.

-   **Key Data Structures**:
    -   `AnalyzedFile`: The complete, cached semantic model for a single file, containing its AST, exports, and links.
    -   `Environment`: Represents the fully resolved scope at a specific point in execution, aggregating variables and templates from the current file and all its dependencies.
    -   `FileExports`: Summarizes the public interface of a file (variables, templates, targets) available to importers.
    -   `AnalyzedBlock`, `AnalyzedStatement`: Semantic wrappers around AST nodes, holding resolved scopes and other metadata.

-   **Analysis Flow**:
    -   `analyze_file(path)`: Returns the cached `AnalyzedFile`.
    -   `analyze_at(file, pos)`: Returns an `Environment` representing the state of the program at `pos`, aggregating definitions from the build config and imports.

### LSP Feature Providers (`src/server/providers/`)

Each LSP feature is implemented in its own module. These providers consume the data from the Semantic Analyzer to generate responses for the client. Examples include `completion`, `hover`, `goto_definition`, and `references`.

## 4. Data Flow Example: "Go to Definition"

A typical request flows through the system as follows:

1.  **Request**: The user triggers "Go to Definition" on a variable in the editor. The client sends a `textDocument/definition` request to the server.
2.  **Dispatch**: The `Backend` in `src/server/mod.rs` receives the request and dispatches it to the `goto_definition` provider.
3.  **Analysis (File)**: The provider calls `analyzer.analyze_file()` to get the `AnalyzedFile` for the current document. This returns the cached result of the local analysis (AST, links, exports).
4.  **Link Check**: The provider first checks if the cursor is on a "link" (e.g., a file path in an import or a target label in `deps`). If so, it resolves the destination immediately.
5.  **Analysis (Scope)**: If the cursor is on an identifier, the provider calls `analyzer.analyze_at()`. This triggers the on-demand scope construction, aggregating exports from imported files to build a complete `Environment` for that specific position.
6.  **Resolution**: The provider looks up the identifier in the `Environment` to find all matching variable assignments or template definitions.
7.  **Response**: The provider constructs a `LocationLink` response containing the URIs and ranges of the definitions and returns it to the client.
