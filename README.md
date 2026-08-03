# corvette

Rust work for the frigate-vulkan project: eventually a Leptos UI and an NVR
behind it, with Frigate retired piece by piece. Today it is one thing only --
the spike that decides whether any of that is worth starting.

The plan this repo implements lives in the sibling repository,
`frigate-vulkan`, at `docs/distroless-split-plan.md`. That repo keeps the
ncnn/Vulkan detector plugin and the container packaging; nothing here is pinned
to `FRIGATE_VERSION`, and nothing there is pinned to a Rust toolchain.

## The ncnn spike

Everything on the roadmap after stage 2 assumes ncnn is usable from Rust with
Vulkan. That assumption rests on ncnn's **C** API, which is narrower than the
C++ API the Python detector uses -- it covers the entire inference path but has
no GPU enumeration at all. See `docs/ncnn-spike.md` for the result.

```
scripts/run_spike.sh                 # build, run, and diff against the Python detector
```

## Layout

| Path | What |
| --- | --- |
| `crates/ncnn-sys` | raw FFI over ncnn's C API, plus `csrc/c_api_ext.cpp` -- the GPU enumeration the C API is missing |
| `crates/ncnn-spike` | the benchmark/parity binary |
| `docker/Dockerfile.spike` | builds ncnn from source with `NCNN_VULKAN=ON`, then the Rust binary |
| `scripts/` | the harness that runs both implementations over one input and compares them |
| `docs/ncnn-spike.md` | findings |
