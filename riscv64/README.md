build setup

```
rustup target add riscv64gc-unknown-none-elf
```

build:

```
cargo xtask build --arch riscv64
```

run:

```
cargo xtask qemu --arch riscv64
```
