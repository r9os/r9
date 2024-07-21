# r9
[Plan 9](https://plan9.io/plan9/) in Rust

R9 is a reimplementation of the plan9 kernel in Rust.  It is
not only inspired by but in many ways derived from the original
[Plan 9](https://plan9.io/plan9/) source code.

## Building

We use `cargo` and the `xtask` pattern to build the kernel.

To build r9 for x86_64, we assume you have cloned the git repository
somewhere convenient.  Then simply change into the top-level
directory and, `cargo xtask build --arch x86-64`.

To build for aarch64, run `cargo xtask build --arch aarch64` (Currently only Raspberry Pi 3 is supported).

There are other useful `xtask` subcommands; run
`cargo xtask help` to see what is available.

Right now, r9 is not self-hosting.

## Runtime Dependencies

`cargo xtask dist`, which `cargo xtask qemu` depends on, requires `llvm-objcopy`. 
This is expected to live in the rust toolchain path.  You can install by running:
```
rustup component add llvm-tools
```

If you get `No such file or directory (os error 2)` messages, 
then install `llvm` separate from the rust toolchain and set:
```
OBJCOPY=$(which llvm-objcopy) cargo xtask qemukvm
```

If `No such file or directory (os error 2)` messages persist, 
check to ensure `qemu` or `qemu-kvm` is installed and the 
`qemu-system-x86_64` binary is in your path (or `qemu-system-aarch64` in the case of aarch64).

## Running on Qemu

R9 can be run using qemu for the various supported architectures:

|Arch|Platform|Commandline|
|----|--------|-----------|
|aarch64|raspi3b|cargo xtask qemu --arch aarch64 --verbose|
|aarch64|raspi4b|cargo xtask qemu --arch aarch64 --config raspi4b --verbose|
|x86-64|q35|cargo xtask qemu --arch x86-64 --verbose|
|x86-64 (with kvm)|q35|cargo xtask qemu --arch x86-64 --kvm --verbose|
|riscv|virt|cargo xtask qemu --arch riscv64 --verbose|

## Running on Real Hardware™️

R9 has been run on the following hardware to a greater or lesser degree:
- Raspberry Pi 4 (Gets as far as printing 'r9' via the miniuart)

### Raspberry Pi, Netboot

Assuming you can set up a TFTP server (good luck, it's incredibly fiddly, but for what it's worth, dnsmasq can work occasionally), and assuming the location of your netboot directory, you can build and copy the binary using the following command:
```
cargo xtask dist --arch aarch64 --verbose && cp target/aarch64-unknown-none-elf/debug/aarch64-qemu.gz ../netboot/kernel8.img
```

This copies a compressed binary, which should be much faster to copy across the network.

The Raspberry Pi firmware loads `config.txt` before the kernel.  Here we can set which UART to use, amongst other things.  The following contents will set up to use the miniuart:
```
enable_uart=1
core_freq_min=500
```