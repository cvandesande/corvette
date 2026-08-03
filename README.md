# corvette

Rust work for the frigate-vulkan project: a Leptos UI and, eventually, an NVR
behind it, with Frigate retired piece by piece. The ncnn/Vulkan spike has proved
the detector path and the first UI foundation is now in place.

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
scripts/run_spike.sh                 # build, benchmark, and check device selection
```

## Building

Rust **1.97.1**, pinned in `rust-toolchain.toml` and honoured by all three
paths -- rustup reads it directly, `flake.nix` feeds it to rust-overlay, and
`docker/Dockerfile.spike` pins the matching image tag.

```
nix develop                          # toolchain + ncnn
make check                           # format, lint, and test everything
make serve-ui                        # port-forward Frigate and serve the UI
nix build .#ncnn-spike               # or .#ncnn for the pinned ncnn alone
docker build -f docker/Dockerfile.spike --target lint .   # clippy + rustfmt gate
```

`serve-ui` defaults to the `icams/frigate` service and the kubeconfig at
`~/dockers/talos/tirnanog/generated/kubeconfig`. It forwards Frigate's API and
go2rtc, using MSE so live media stays inside the TCP tunnel. Override
`FRIGATE_KUBECONFIG`, `FRIGATE_NAMESPACE`, `FRIGATE_SERVICE`, or
`FRIGATE_POD_SELECTOR` for another deployment. Press Ctrl-C to stop Cargo Leptos and
both port-forwards.

Lints are deliberately loud: clippy's `pedantic`, `nursery` and `cargo` groups
are **denied** workspace-wide, along with `undocumented_unsafe_blocks` -- in an
FFI crate every unsafe block should have to say why it is sound. `restriction`
is not enabled as a group, on upstream's own advice. See the bottom of
`Cargo.toml`.

## Layout

| Path | What |
| --- | --- |
| `crates/corvette-api` | shared HTTP wire contracts for the UI and future Rust service |
| `crates/corvette-ui` | Leptos client-side UI, built as split WebAssembly with Cargo Leptos |
| `crates/corvette-ui-server` | static fallback and Frigate/go2rtc proxies used by local UI development |
| `crates/ncnn-sys` | raw FFI over ncnn's C API, plus `csrc/c_api_ext.cpp` -- the GPU enumeration the C API is missing |
| `crates/ncnn-spike` | the benchmark/parity binary |
| `docker/Dockerfile.spike` | builds ncnn from source with `NCNN_VULKAN=ON`, then the Rust binary |
| `scripts/` | the benchmark and device-selection harness |
| `Makefile` | the repository-wide local and CI check entry point |
| `flake.nix` | pinned toolchain, a Vulkan-enabled ncnn, and a dev shell |
| `docs/ncnn-spike.md` | findings |

## License

MIT, in `LICENSE`.

ncnn is BSD-3-Clause, which is attribution-only, so linking it carries no
further obligation -- but a redistributed binary has to carry the notice. The
spike image ships it under `/usr/share/licenses/`. Note that the container build
links glslang (Apache-2.0 and others) *into* `libncnn.so`, so that image carries
both notices; the Nix build leaves glslang as separate libraries.

**Models are not covered by any of this.** Nothing here ships model weights, and
neither does `frigate-vulkan` -- both gitignore `models/`. Ultralytics' YOLOv9
weights are AGPL-3.0, and upstream YOLOv9 is GPL-3.0, so exported `.param`/`.bin`
files are copyleft artifacts. Export your own; see `frigate-vulkan`'s
`docs/free-yolov9-model-guide.md`.
