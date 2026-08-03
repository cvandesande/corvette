# corvette

Rust work for the frigate-vulkan project: eventually a Leptos UI and an NVR
behind it, with Frigate retired piece by piece. Today it is one thing only --
the spike that decides whether any of that is worth starting.

The plan is [docs/roadmap.md](docs/roadmap.md) -- moved here from the sibling
`frigate-vulkan` repository once the spike proved the premise. That repo keeps
the ncnn/Vulkan detector plugin and the container packaging; nothing here is
pinned to `FRIGATE_VERSION`, and nothing there is pinned to a Rust toolchain.

## The ncnn spike

Everything on the roadmap after stage 2 assumes ncnn is usable from Rust with
Vulkan. That assumption rests on ncnn's **C** API, which is narrower than the
C++ API the Python detector uses -- it covers the entire inference path but has
no GPU enumeration at all. See `docs/ncnn-spike.md` for the result.

```
scripts/run_spike.sh                 # build, run, and diff against the Python detector
```

## Building

Rust **1.97.1**, pinned in `rust-toolchain.toml` and honoured by all three
paths -- rustup reads it directly, `flake.nix` feeds it to rust-overlay, and
`docker/Dockerfile.spike` pins the matching image tag.

```
nix develop                          # toolchain + ncnn + python3/numpy
nix build .#ncnn-spike               # or .#ncnn for the pinned ncnn alone
docker build -f docker/Dockerfile.spike --target lint .   # clippy + rustfmt gate
```

Lints are deliberately loud: clippy's `pedantic`, `nursery` and `cargo` groups
are **denied** workspace-wide, along with `undocumented_unsafe_blocks` -- in an
FFI crate every unsafe block should have to say why it is sound. `restriction`
is not enabled as a group, on upstream's own advice. See the bottom of
`Cargo.toml`.

## Layout

| Path | What |
| --- | --- |
| `crates/ncnn-sys` | raw FFI over ncnn's C API, plus `csrc/c_api_ext.cpp` -- the GPU enumeration the C API is missing |
| `crates/ncnn-spike` | the benchmark/parity binary |
| `docker/Dockerfile.spike` | builds ncnn from source with `NCNN_VULKAN=ON`, then the Rust binary |
| `scripts/` | the harness that runs both implementations over one input and compares them |
| `flake.nix` | pinned toolchain, a Vulkan-enabled ncnn, and a dev shell |
| `docs/ncnn-spike.md` | findings |
