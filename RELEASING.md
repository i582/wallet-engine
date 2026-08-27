# Release Wallet Engine

Wallet Engine uses explicit Git tags. A release tag has the form `vMAJOR.MINOR.PATCH`.
SemVer pre-release tags such as `v0.2.0-rc.1` are also valid.

The release process publishes files to GitHub Releases. It does not publish packages to language registries.

## Release files

Each release contains these files:

| File | Contents |
| --- | --- |
| `wallet-engine-VERSION-x86_64-unknown-linux-gnu.tar.gz` | Linux x86-64 static library, shared library, and C++ wrapper |
| `wallet-engine-VERSION-aarch64-unknown-linux-gnu.tar.gz` | Linux ARM64 static library, shared library, and C++ wrapper |
| `wallet-engine-VERSION-x86_64-apple-darwin.tar.gz` | macOS x86-64 static library, dynamic library, and C++ wrapper |
| `wallet-engine-VERSION-aarch64-apple-darwin.tar.gz` | macOS ARM64 static library, dynamic library, and C++ wrapper |
| `wallet-engine-VERSION-x86_64-pc-windows-msvc.zip` | Windows x86-64 static library, DLL with import library, and C++ wrapper |
| `wallet-engine-swift-VERSION.zip` | Swift package with a macOS and iOS `XCFramework` |
| `wallet-engine-android-VERSION.aar` | Kotlin wrapper and Android libraries for ARM64 and x86-64 |
| `wallet-engine-android-VERSION.pom` | Maven metadata for the Android archive |
| `ton-wallet-engine-VERSION.tgz` | TypeScript package and WebAssembly runtime |

Each package has a `.sha256` checksum file. The release also contains `SHA256SUMS` and `release-manifest.json`.

GitHub creates a build-provenance attestation for the release files.

## Prepare a release

1. Add the release section to `CHANGELOG.md`.
2. Merge all release changes into `master`.
3. Make sure that the local `master` branch is clean.
4. Make sure that all required GitHub checks passed for the current commit.
5. Run the release command:

```shell
cargo xtask release --version 0.2.0
```

The command updates all public package versions and their lockfiles. Then it shows the version diff and requests confirmation.

After confirmation, the command creates an annotated tag and a release commit. Then it pushes both objects atomically.

Use `--yes` only in an automation environment that already approved the release:

```shell
cargo xtask release --version 0.2.0 --yes
```

## Tag workflow

The `Release` GitHub Actions workflow starts after the tag push. It makes each platform package in a separate job.

The final job checks the complete file set and every checksum. Then it creates the GitHub Release from the matching changelog section.

The workflow marks a SemVer pre-release as a GitHub pre-release.

## Run a dry build

Start the `Release` workflow manually and supply a tag that matches the selected revision. A manual run uploads one workflow artifact.

The manual run does not create a GitHub Release. It also does not create provenance attestations.

You can run package commands locally:

```shell
cargo xtask dist verify-tag --tag v0.1.0
cargo xtask dist native --target aarch64-apple-darwin
cargo xtask dist native --target x86_64-pc-windows-msvc
cargo xtask dist swift
cargo xtask dist android
cargo xtask dist web
```

The `dist manifest` command requires the complete file set from all platform jobs.
