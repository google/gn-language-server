# AGENTS.md

## 🏗 Project Architecture & Philosophy

- **Goal**: Provide a fast, configuration-agnostic IDE experience for GN.
- **Golden Rule**: **NEVER read `args.gn`**.
  - The analyzer must be configuration-agnostic.
  - It must simulate all branches of `if` conditions (e.g., both `if (is_win)` and `else` branches) to provide a holistic view.
  - Do not try to evaluate build arguments or execute `exec_script`.
- **Latency**: Analysis runs on the UI thread in some clients; performance is critical. Re-use cached `AnalyzedFile` structure where possible.

## 🛠 Tech Stack

- **Server**: Rust (using `tower-lsp`, `pest`, `tokio`).
- **VS Code Extension**: TypeScript (`vscode-gn/`).
- **IntelliJ Plugin**: Kotlin (`intellij-gn/`).
- **Parsing**: The GN grammar is formally defined in `src/parser/gn.pest`. Any syntax changes MUST be reflected there.

## 📦 Versioning Strategy

- **Release Cycle**:
  - `1.<even>.x`: Stable releases (e.g., 1.2.0).
  - `1.<odd>.x`: Pre-release/Dev versions (e.g., 1.3.0).
- **Rust Crates**: Pre-releases for crates.io use the `-prerelease` suffix (e.g., `1.11.1-prerelease`).

## 📂 Key Directories

- `src/`: Rust source code for the Language Server.
- `src/analyzer/`: The core semantic analysis logic.
- `src/server/`: LSP implementation and request handling.
- `vscode-gn/`: Source for the VS Code extension.
- `intellij-gn/`: Source for the IntelliJ plugin.

## 🧪 Testing & Verification

- **Rust**: `cargo test` runs the server unit tests.
- **VS Code**: `npm test` in `vscode-gn/` (if available) or manual verification in the extension host.
- **Modifying Grammar**: If `src/parser/gn.pest` is changed, ensure the parser tests in `src/parser/mod.rs` pass and cover new cases.

---

## 🔐 Git & Commit Rules (Global)

### Commit philosophy
- Commit history must be **linear** — no merge commits in central branches.
- Every commit must **pass all tests**.

### Git restrictions for the agent
- **Never create commits** or amend Git history.
- **Never modify the Git index or staging area**.
- **Only read and write files** in the working tree.

## 🤖 Agent Personality (Global)
- **Plan first**: Understand the architectural impact (especially on the `args.gn` rule) before coding.
- **Readability**: Code is for humans first.
- **Safety**: Do not delete files without confirmation.
