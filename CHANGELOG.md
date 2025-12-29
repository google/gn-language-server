# Changelog

## 1.10.0 (unreleased)

- Introduce the JetBrains IDE plugin
- Features:
  - Highlight undefined variables
  - Suggest completion items from the workspace
  - Auto-import variables and templates on completion
  - Quick fix to import variables
  - Context-aware completion
  - Code lens for targets
  - Support workspace symbols
- Fixes:
  - Fix crash on completing foreach variables
  - Fix incorrect analysis for circular imports
- Misc:
  - Add icons for the extension and build files
  - Update GN base version to support `path_exists`
  - Improve workspace scan efficiency
  - Add build attestation

## 1.8.0 (2025-10-01)

- Workaround for "cycle detected" problem (#49)
- Experimental support of workspace symbols (behind an experimental setting)
- Experimental support of undefined variable analysis (behind an experimental setting)
- A lot of internal rework of the analysis mechanism

## 1.6.0 (2025-08-06)

- Support completing file names (#6)
- Support finding references for targets (#38)
- Support relative labels (#41)
- Enable syntax error reporting by default
- Enable background indexing by default
- Improve parsing robustness
- Speed up extension activation

## 1.4.0 (2025-04-09)

- feat: Support GN prebuilt paths for Fuchsia
- chore: Improve error messages on GN binary not found
- docs: Mention Fuchsia in docs

## 1.2.0 (2025-04-01)

- Support "Go to definition" for dependency labels and file paths (#25).

## 1.0.0 (2025-03-26)

- Initial release.
