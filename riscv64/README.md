build setup

```
rustup target add riscv64gc-unknown-none-elf
```

build:

```
cargo xtask build --arch riscv64gc
```

run:

```
cargo xtask qemu --arch riscv64gc
```
