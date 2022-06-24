# r9
Plan 9 in Rust

R9 is a reimplementation of the plan9 kernel in Rust.  It is
not only inspired by but in many ways derived from the original
Plan 9 source code.

## Building

We use `cargo` and the `xtask` pattern to build the kernel.

To build r9, we assume you have cloned the git repository
somewhere convenient.  Then simply change into the top-level
directory and, `cargo xtask build`.

There are other useful `xtask` subcommands; run
`cargo xtask help` to see what is available.

Right now, r9 is not self-hosting.
