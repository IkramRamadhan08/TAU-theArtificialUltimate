# TAU - The Artificial Ultimate

TAU is a local-first, agentic coding IDE — a modified fork of Zed, wired toward the TAU Rust agent runtime.

This repository is a modified fork of Zed. It is not affiliated with, endorsed by, or sponsored by Zed Industries, Inc.

---

### Quick Start

```bash
git clone https://github.com/IkramRamadhan08/TAU-theArtificialUltimate.git
cd TAU-theArtificialUltimate/editor
cargo run
```

Or use the `tau` CLI:

```bash
../tau.sh install   # build editor + tau-chat
../tau.sh editor    # launch editor (auto-builds if needed)
../tau.sh "your prompt"  # one-shot chat
../tau run          # auto-detect & run project
```

### Architecture

TAU keeps the editor shell and the agent runtime separated:

- `TAU-core/agent` — multi-agent runtime and ACP adapter.
- `editor/` — editor shell, packaging identity, and default TAU agent wiring.

### Developing

- Build the TAU agent core from `TAU-core/agent`.
- Build this editor shell with the upstream Zed build flow.
- Keep upstream Zed notices and license files intact.

### Licensing

Upstream Zed source code is licensed under GPL-3.0-or-later, with Apache-2.0 components where marked. TAU modifications to GPL-covered parts are distributed under GPL-3.0-or-later.

License information for third party dependencies must be correctly provided for CI to pass.

See:

- `LICENSE-GPL`
- `LICENSE-APACHE`
- `NOTICE-TAU.md`

We use [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) to automatically comply with open source licenses. If CI is failing, check the following:

- Is it showing a `no license specified` error for a crate you've created? If so, add `publish = false` under `[package]` in your crate's Cargo.toml.
- Is the error `failed to satisfy license requirements` for a dependency? If so, first determine what license the project has and whether this system is sufficient to comply with this license's requirements. If you're unsure, ask a lawyer. Once you've verified that this system is acceptable add the license's SPDX identifier to the `accepted` array in `script/licenses/tau-licenses.toml`.
- Is `cargo-about` unable to find the license for a dependency? If so, add a clarification field at the end of `script/licenses/tau-licenses.toml`, as specified in the [cargo-about book](https://embarkstudios.github.io/cargo-about/cli/generate/config.html#crate-configuration).

## Upstream

TAU is based on Zed, originally developed by **Zed Industries, Inc.** Learn more at <https://zed.dev> and <https://github.com/zed-industries/zed>.
