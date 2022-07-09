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

### Aarch64
By default, the x86_64 architecture is built and run.  To run the
xtask commands for the aarch64 architecture, prepend with
`ARCH=aarch64 TARGET=aarch64-unknown-none-elf`.  E.g. to build and
run in qemu simulating a Raspberry Pi 3, run:
`ARCH=aarch64 TARGET=aarch64-unknown-none-elf cargo xtask qemu`.

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
