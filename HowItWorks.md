# How r9 Works

**Please update with any material code change!**

This document summarises the flow of each implementation as it currently stands. We do this because r9 is a hobby project that is dipped in and out of, with months between dips. In that context, it's really hard to remember how it works and why. A document like this also serves as a useful introduction to the project.

## aarch64

### Early initialisation (Assembly)

Execution begins in `l.S` at the `start` label and is focused on Raspberry 4 ATM. Supporting rpi5 should be straightforward, but anything beyond that would get complex. That would be a strong argument to move some of this code to rust, although that code would probably need to be segrated fromt he rest of the rust code.

The code in `l.S` exists to bring us to a state where we can run kernel code in the OS level (EL1). In order to get there, we run through a number of steps:

1. Disable other cores
2. Ensure we're in EL1
3. Set up the miniuart for some kind of debugging. This is a bare bones set up - a rust driver will take over later. In this assembly file, this allows us to do something similar to printf-style debugging - writing a single character to the UART at specific points. These would be temporary, but once we have completed the UART initialisation, we output a single '.' so we at least know we've gotten that far.
4. Set up a set of initial page tables and enable the MMU. The page tables are defined at the bottom of the file, and maps the rpi4 MMIO, and the kernel. This is extensively described in the comments. The page tables are set up with a recursive page table entry. Tables are configured to identity map and to map in kernel space. The identiy map ensures the code in l.S can continue to run once the MMU is enabled, and allow it to then jump into the higher half of the address space.
5. Set up the initial stack (size set by STACKSZ - 4KiB), defined to be in BSS at the bottom of the file.
6. Clear the BSS.
7. Jump into the rust code, entrypoint `main.rs::main9`.

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

No user page is ever both writable and executable. AArch64 has no "kernel read-write, user read-only" permission, and the kernel has no mapping of all RAM, so it can't yet write into a page that's mapped `ro_user_text`. Until it can, the user test's text is a page-aligned page of the kernel image's read-only data, mapped into user space as is. The kernel writes into user pages through their user addresses, which only works while that page table is active. `map_zeroed_user_pages` checks this.

### Virtual Memory

VM initialisation proceeds as follows:

1. Set up the page allocator. We use a bitmap allocator defined in `pagealloc.rs`. This makes use of the `earlypages` reserved in the `kernel.ld` linker script. The 16KiB reserved here allows us to manage 16\*4096\*8 pages. Each bit represents 4096KiB, allowing us to map 2GiB of RAM. The allocator assumes everything except the early pages are allocated at first. This is to ensure we don't mistakenly allocate a page that should not be available (e.g. will be mapped below).
2. Build an initial memory map. This is also a bit rpi4-specific. It maps:

- Device tree DTB
- Kernel text, data and BSS
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
