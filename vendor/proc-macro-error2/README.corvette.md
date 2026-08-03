# Vendored proc-macro-error2

This directory contains the source files required to build
`proc-macro-error2` 2.0.1, copied from its crates.io release. The upstream
repository is archived, but Leptos 0.8 still uses its diagnostic API from
several procedural macros.

The only source change makes the `proc_macro` crate public, as required by
rust-lang/rust#127909. This matches the pending upstream fix in
GnomedDev/proc-macro-error-2#14.

The original MIT and Apache-2.0 license texts are retained alongside the
source. Remove this patch when Leptos no longer resolves `proc-macro-error2`.
