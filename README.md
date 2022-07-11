# r9
Plan 9 in Rust

R9 is a reimplementation of the plan9 kernel in Rust.  It is
not only inspired by but in many ways derived from the original
Plan 9 source code.

## Building

We use `cargo` and the `xtask` pattern to build the kernel.

To build r9 for x86_64, we assume you have cloned the git repository
somewhere convenient.  Then simply change into the top-level
directory and, `cargo xtask build --arch x86_64`.

To build for aarch64, run `cargo xtask build --arch aarch64` (Currently only Raspberry Pi 3 is supported).

There are other useful `xtask` subcommands; run
`cargo xtask help` to see what is available.

Right now, r9 is not self-hosting.

## Runtime Dependencies

`cargo xtask dist`, which `cargo xtask qemu` and 
`cargo xtask qemukvm` depend on, requires `llvm-objcopy`. 
This is expected to live in the rust toolchain path. If 
you get `No such file or directory (os error 2)` messages, 
then install `llvm` separate from the rust toolchain and set:
```
OBJCOPY=$(which llvm-objcopy) cargo xtask qemukvm
```

If `No such file or directory (os error 2)` messages persist, 
check to ensure `qemu` or `qemu-kvm` is installed and the 
`qemu-system-x86_64` binary is in your path (or `qemu-system-aarch64` in the case of aarch64).
