# How r9 Works

**Please update with any material code change!**

This document summarises the flow of each implementation as it currently stands. We do this because r9 is a hobby project that is dipped in and out of, with months between dips. In that context, it's really hard to remember how it works and why. A document like this also serves as a useful introduction to the project.

## aarch64

### Early initialisation (Assembly)

Execution begins in `l.S` at the `start` label and is focused on Raspberry 4 ATM. Supporting rpi5 should be straightforward, but anything beyond that would get complex. That would be a strong argument to move some of this code to rust, although that code would probably need to be segrated fromt he rest of the rust code.

The code in `l.S` exists to bring us to a state where we can run kernel code in the OS level (EL1). In order to get there, we run through a number of steps:

1. Park every core except core 0 (`dnr`).
2. Drop to EL1 from EL3 or EL2, enabling AArch64 and the FPU for EL1 on the way.
3. Set up the initial stack (size set by STACKSZ - 16KiB), defined in BSS at the bottom of the file, using its physical address since the MMU is still off.
4. Clear the BSS and the early page table area (`earlyvm_pagetables` in `kernel.ld`).
5. Set up the miniuart for some kind of debugging (`init_early_uart_rpi4` in `pre_mmu/util.rs`). This is a bare bones set up - a rust driver will take over later. It allows us to do something similar to printf-style debugging - writing a single character to the UART at specific points. Once the UART is initialised, we output a single '.' so we at least know we've gotten that far.
6. Call `init_vm` in `pre_mmu/vminit.rs`, which builds the initial page tables from the early page table area while the MMU is still off. It maps the kernel image, the DTB and the kmap region's tables at KZERO + their physical addresses (`ttbr1_el1`), with the recursive entry 511 in the kernel root. It also identity maps the first 1GiB (`ttbr0_el1`), so the code in `l.S` can keep running once the MMU is enabled and then jump into the higher half. It sets `TCR_EL1` and `MAIR_EL1` too.
7. Enable the MMU and caches, jump to the higher half, and move the stack to its virtual address.
8. Jump into the rust code, entrypoint `main.rs::main9`, passing the DTB's virtual address.

### Initialisation (Rust)

We're now in rust, in `main9`, and at a high level we:

1. Set up interrupt handlers early so that we get a bit of feedback if things go wrong.
2. Parse the device tree, enabling more informed initialisation.
3. Set up the mailbox (rpi-specific, but allows us to extract more information from the hardware, and interact directly in certain ways).
4. Initialise the [console](#devcons), ensuring we can use `port::println` to write to the UART.
5. Write out some useful system information.
6. Set up [virtual memory](#virtual-memory) in rust. (After switching we no longer need the pagetables set up in early initialisation.)
7. Write out the page tables.
8. Switch back and forth between `main9` and a second kernel context to test `swtch`.
9. Run a hand-assembled test program in EL0 that makes a couple of syscalls, ending with an exit syscall that halts.

### Exceptions

`trap.S` holds the exception vector table. Every entry pushes a `TrapFrame` (all general purpose registers, plus `ESR_EL1`, `ELR_EL1`, `FAR_EL1`, `SP_EL0` and `SPSR_EL1`) onto the current stack, calls `trap` in `trap.rs`, then leaves through `trap_return`. `trap_return` takes the return state from the frame rather than the live registers, so a handler can change where we return to, or what's returned in x0.

- Exceptions from EL1 are taken on the interrupted kernel stack.
- Exceptions from EL0 are taken on whatever `SP_EL1` was when we entered EL0. `touser` enters EL0 by building a `TrapFrame` at the top of a kernel stack and jumping to `trap_return`, which leaves `SP_EL1` at the top of that stack. Until processes have their own kernel stacks, the interrupt stack is used.

The kernel is built softfloat (see `lib/aarch64-unknown-none-elf.json`), so it never uses the FP/SIMD registers and exception entry doesn't need to save them. User FP/SIMD state still needs saving when switching between processes.

IRQs, FIQs, SErrors and debug exceptions are masked in both EL1 and EL0 until we have a use for them. Syscalls are currently placeholders: the `svc` immediate is the syscall number, syscall 3 halts, and any other returns x0 + 1.

### Kernel context switching

`swtch(save, next)` in `swtch.rs` saves the current kernel context's callee-saved registers (x19-x29, the return address and `sp`) into the `save` `Label`, loads them from `next`, and returns into wherever `next` last called `swtch`. `Label::new_context` builds a label that starts a never-returning function on a fresh stack. This is only for switching between kernel contexts at EL1; entering EL0 is done with `touser`.

### User Address Space

User addresses are translated through `ttbr0_el1`. The layout is defined in `param.rs`:

- `UTZERO` (0x10000): start of user text. The pages below are unmapped to catch null pointers.
- `USTKTOP` (2^47): top of the user stack, which is `USTKSIZE` (64KiB), mapped up front, with nothing mapped below it.
- `UADDRTOP` (0x0000_ff80_0000_0000): end of the user address space. Root entry 511 above it recursively maps the user page table. `map_to` refuses addresses in the recursive range, or outside the half of the address space its table translates.

No user page is ever both writable and executable. AArch64 has no "kernel read-write, user read-only" permission, so the kernel fills user pages through [`kmap`](#virtual-address-map), never through their user address. `map_zeroed_user_pages` zeroes each page this way before mapping it. The user test's text is still a page-aligned page of the kernel image's read-only data, mapped into user space as is, until there's an ELF loader.

### Virtual Memory

#### Virtual address map

4KiB granules and 48-bit addresses in each half (`T0SZ` = `T1SZ` = 16). `ttbr0_el1` translates the low half and `ttbr1_el1` the high half. The example addresses are from QEMU's raspi4b.

```
                        ttbr0_el1: user (one root table per address space)
0x0000_0000_0000_0000  +--------------------------------------------------+
                       | unmapped: catches null pointers                  |
0x0000_0000_0001_0000  +--------------------------------------------------+ UTZERO
                       | user text (ro_user_text: user RX)                |
                       +--------------------------------------------------+
                       |                       ...                        |
0x0000_7fff_ffff_0000  +--------------------------------------------------+ USTKTOP - USTKSIZE
                       | user stack, 64KiB (rw_user_data: user RW)        |
0x0000_8000_0000_0000  +--------------------------------------------------+ USTKTOP
                       |                       ...                        |
0x0000_ff80_0000_0000  +--------------------------------------------------+ UADDRTOP
                       | root entry 511: recursive map of the user tables |
0x0000_ffff_ffff_ffff  +--------------------------------------------------+

                       non-canonical: faults

                        ttbr1_el1: kernel (shared)
0xffff_0000_0000_0000  +--------------------------------------------------+
                       | unused                                           |
0xffff_8000_0000_0000  +--------------------------------------------------+ KZERO
                       | KZERO + physical address, for what the kernel    |
                       | maps for itself (not all RAM):                   |
                       |   0x00_0000 kernel text (incl. boottext)    RX   |
                       |   0x20_0000 kernel rodata                   RO   |
                       |   0x40_0000 kernel data, bss, early tables  RW   |
                       |   0x800_0000 DTB                            RO   |
                       |   kernel pages from allocate_virtpage,           |
                       |   4KiB each (e.g. stacks)                   RW   |
                       +--------------------------------------------------+
                       |                       ...                        |
0xffff_9000_0000_0000  +--------------------------------------------------+ KZERO + 16TiB
                       | Device mappings (rw_device), allocated 4KiB at a |
                       | time upwards by next_free_device_page4k: UART,   |
                       | GPIO, mailbox registers and buffer, ...          |
                       +--------------------------------------------------+
                       |                       ...                        |
0xffff_ff00_0000_0000  +--------------------------------------------------+ KMAPBASE
                       | kmap: KMAPSLOTS 4KiB slots per CPU, mapped and   |
                       | unmapped one page at a time                 RW   |
                       +--------------------------------------------------+
                       |                       ...                        |
0xffff_ff80_0000_0000  +--------------------------------------------------+
                       | root entry 511: recursive map of the kernel      |
                       | tables                                           |
0xffff_ffff_ffff_ffff  +--------------------------------------------------+
```

- There's no direct map of RAM. `kaddr(pa)` and `paddr(va)` in `kmem.rs` convert only for what's mapped at KZERO + its physical address: the kernel image, the DTB and pages the kernel maps for itself with `allocate_virtpage`.
- Page tables are reached through the recursive entries. Any other physical page, such as a user page, is reached through `kmap` (`kmap.rs`, as in Plan 9). `kmap(pa)` maps the page in one of the current CPU's slots and returns a guard. Dropping the guard unmaps it and invalidates that address in the TLB. The tables for the region are built before the MMU is on, so `kmap` never allocates. It's for thread context only.
- The kernel image sections are 2MiB aligned by `kernel.ld`, so they can have their own permissions using 2MiB blocks.
- Before the switch to the higher half, `ttbr0_el1` holds an identity map of the first 1GiB, so `l.S` keeps running when the MMU is turned on. It's replaced by the user page table.

VM initialisation proceeds as follows:

1. Set up the page allocator. We use a bitmap allocator defined in `pagealloc.rs`. This makes use of the `earlypages` reserved in the `kernel.ld` linker script. The 16KiB reserved here allows us to manage 16\*4096\*8 pages. Each bit represents 4096KiB, allowing us to map 2GiB of RAM. The allocator assumes everything except the early pages are allocated at first. This is to ensure we don't mistakenly allocate a page that should not be available (e.g. will be mapped below).
2. Build an initial memory map. This is also a bit rpi4-specific. It maps:

- Device tree DTB
- Kernel text, data and BSS
- The tables for the kmap region, with no pages mapped
- MMIO

3. We map each of the above ranges according to the defined entry flags.
4. We now mark all the unallocated pages free.

To document:

- The innards of page table mapping.

Future Improvements:

- The page table management code needs to be able to obtain new page frames while updating page tables. It's simplest to ensure we have at least X page frames pre-mapped, and available in the page allocator.
- Replace the page allocator with something a little more sophisticated with a higher upper bound.
- If the page allocator made use of something like a free list, we could add the early pages to that free list, and avoid the weird hack of marking everything as allocated up front and having to later mark as unallocated.
- Move the page allocator to port.

## x86_64

## riscv64

## port

`port` is where all the shared subsytems are defined.

### devcons

### fdt

### mcslock
