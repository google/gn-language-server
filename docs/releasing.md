# Releasing

The release process for this project is highly automated using GitHub Actions and custom Python scripts. There are two main types of releases: **Nightly Pre-releases** and **Stable Releases**.

## Versioning Scheme

- **Stable Releases**: `1.<even>.x` (e.g., `1.10.0`, `1.12.5`)
- **Pre-releases**: `1.<odd>.x` (e.g., `1.11.4`, `1.13.0`)
  - For Rust (`crates.io`), pre-releases use the suffix `-prerelease` (e.g., `1.11.4-prerelease`).

## Release Types

### 1. Nightly Pre-releases (Automated)

The [.github/workflows/nightly.yml](../.github/workflows/nightly.yml) workflow runs daily at 03:00 UTC.

1. **Check for Changes**: It checks if there are new commits since the last version bump.
2. **Bump Version**: If changes exist, it runs `./scripts/bump_version.py --update` (without `--release`).
   - This increments the patch version (e.g., `1.11.3` -> `1.11.4`) or switches to the next odd minor version if currently even.
3. **Commit & Tag**: It commits the version change, creates a tag (e.g., `v1.11.4`), and pushes to `main`.
4. **Create GitHub Release**: It creates a GitHub "Pre-release" for the new tag.
5. **CI Trigger**: The push of the tag triggers the CI workflow.

### 2. Stable Releases (Manual)

To perform a stable release:

1. **Update `CHANGELOG.md`**: Add details for the new version.
2. **Bump Version**: Run `./scripts/bump_version.py --release --update`.
   - This bumps the version to the next even minor version (e.g., `1.11.4` -> `1.12.0`).
3. **Commit**: Create a "version bump commit" that includes the manifest changes and `CHANGELOG.md` update.
4. **Push**: Push the commit to `main`.
5. **Create Tag & Release (GitHub Web UI)**: Navigate to GitHub and create a new Release. This automatically creates the tag simultaneously. **Do not create the tag from the CLI.**

### 3. Patch Releases (Rare)

Used for important fixes to stable releases (e.g., `1.10.0` -> `1.10.1`).

1. **Create Maintenance Branch**: Create a branch named `v<major>.<minor>.x` (e.g., `v1.10.x`) from the previous stable release tag.
2. **Cherry-pick Patches**: Apply the necessary fixes to this branch.
3. **Update `CHANGELOG.md`**: Update `CHANGELOG.md` **on the main branch**.
4. **Follow Stable Release Procedure**:
   - Apply necessary changes to `CHANGELOG.md` **on the maintenance branch**.
   - Run `./scripts/bump_version.py --release --update`.
   - Create a version bump commit and push the branch.
   - Use the GitHub Web UI to create a tag and release from this branch.

> [!NOTE]
> The CI workflow is configured to trigger on branches matching `v*.*.x`, allowing patch releases to be built and tested correctly.

## CI/CD Workflow ([ci.yml](../.github/workflows/ci.yml))

The CI workflow runs on every push to the `main` branch, maintenance branches (`v*.*.x`), on pull requests, and also when a release tag starting with `v` is pushed.

- **On every commit and PR**: Performs linting, testing, builds all artifacts, and generates build provenance (attestation) to ensure they continue to build correctly.
- **On release tags**: In addition to the above, it performs the following:
  - **Artifact Upload**: Uploads the following to the GitHub release:
    - VSCode extension (`.vsix`) for multiple platforms.
    - IntelliJ plugin (`.zip`).
    - Language server binaries (`gn-language-server-<version>-linux-x86_64`, `gn-language-server-<version>-darwin-aarch64`, `gn-language-server-<version>-windows-x86_64.exe`).
  - **Marketplace Publishing**:
    - **VSCode**: Published to Visual Studio Marketplace and Open VSX.
    - **Rust**: Published to [crates.io](https://crates.io/crates/gn-language-server).
    - **IntelliJ**: Published to JetBrains Marketplace (Pre-releases go to the `eap` channel).

## Key Scripts

- [scripts/bump_version.py](../scripts/bump_version.py): Handles version increments across all components (`Cargo.toml`, `package.json`, `gradle.properties`).
- [scripts/is_prerelease.py](../scripts/is_prerelease.py): Helper script used by CI to detect if the current version is a pre-release based on the minor version number.
