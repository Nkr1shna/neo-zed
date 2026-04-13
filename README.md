# Neo Zed

[![Neo Zed](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/Nkr1shna/neo-zed/main/assets/badge/v0.json)](https://neozed.dev)
[![CI](https://github.com/Nkr1shna/neo-zed/actions/workflows/run_tests.yml/badge.svg)](https://github.com/Nkr1shna/neo-zed/actions/workflows/run_tests.yml)

Welcome to Neo Zed, a high-performance, multiplayer code editor.

## Fork Attribution Notice

Neo Zed is a modified version of Zed. This repository includes modifications as of 2026-04-13 and is released under the GNU Affero General Public License v3.0 or later; see [LICENSE-AGPL](./LICENSE-AGPL).

If you make Neo Zed or its collaboration services available to users over a network, you must also offer those users access to the Corresponding Source of the running version at no charge, as required by AGPLv3 section 13.

---

### Installation

On macOS, Linux, and Windows you can [download Neo Zed directly](https://neozed.dev/download) or install Neo Zed via your local package manager ([macOS](https://neozed.dev/docs/installation#macos)/[Linux](https://neozed.dev/docs/linux#installing-via-a-package-manager)/[Windows](https://neozed.dev/docs/windows#package-managers)).

Other platforms are not yet available:

- Web (not yet available)

### Developing Neo Zed

- [Building Neo Zed for macOS](./docs/src/development/macos.md)
- [Building Neo Zed for Linux](./docs/src/development/linux.md)
- [Building Neo Zed for Windows](./docs/src/development/windows.md)

### Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for ways you can contribute to Neo Zed.

### Licensing

License information for third party dependencies must be correctly provided for CI to pass.

We use [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) to automatically comply with open source licenses. If CI is failing, check the following:

- Is it showing a `no license specified` error for a crate you've created? If so, add `publish = false` under `[package]` in your crate's Cargo.toml.
- Is the error `failed to satisfy license requirements` for a dependency? If so, first determine what license the project has and whether this system is sufficient to comply with this license's requirements. If you're unsure, ask a lawyer. Once you've verified that this system is acceptable add the license's SPDX identifier to the `accepted` array in `script/licenses/zed-licenses.toml`.
- Is `cargo-about` unable to find the license for a dependency? If so, add a clarification field at the end of `script/licenses/zed-licenses.toml`, as specified in the [cargo-about book](https://embarkstudios.github.io/cargo-about/cli/generate/config.html#crate-configuration).

## Sponsorship

Neo Zed is maintained as an open-source fork. If sponsorship options are available for this repository, they are the best way to support ongoing maintenance.
