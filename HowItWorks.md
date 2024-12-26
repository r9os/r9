# How r9 Works

**Please update with any material code change!**

This document summarises the flow of each implementation as it currently stands.  We do this because r9 is a hobby project that is dipped in and out of, with months between dips.  In that context, it's really hard to remember how it works and why.  A document like this also serves as a useful introduction to the project.

## aarch64

### Early initialisation (Assembly)

Execution begins in `l.S` at the `start` label and is focused on Raspberry 3 and 4 ATM.  Supporting rpi5 should be straightforward, but anything beyond that would get complex.  That would be a strong argument to move some of this code to rust, although that code would probably need to be segrated fromt he rest of the rust code.

The code in `l.S` exists to bring us to a state where we can run kernel code in the OS level (EL1).  In order to get there, we run through a number of steps:
1. Disable other cores
2. Ensure we're in EL1
3. Set up the miniuart for some kind of debugging.  This is a bare bones set up - a rust driver will take over later.  In this assembly file, this allows us to do something similar to printf-style debugging - writing a single character to the UART at specific points.  These would be temporary, but once we have completed the UART initialisation, we output a single '.' so we at least know we've gotten that far.
4. Set up a set of initial page tables and enable the MMU.  The page tables are defined at the bottom of the file, and maps the rpi4 MMIO, and the kernel.  This is extensively described in the comments.  The page tables are set up with a recursive page table entry.  Tables are configured to identity map and to map in kernel space.  The identiy map ensures the code in l.S can continue to run once the MMU is enabled, and allow it to then jump into the higher half of the address space.
5. Set up the initial stack (size set by STACKSZ - 4KiB), defined to be in BSS at the bottom of the file.
6. Clear the BSS.
7. Jump into the rust code, entrypoint `main.rs::main9`.

### Initialisation (Rust)

We're now in rust, in `main9`, and at a highlevel we:
1. Set up interrupt handlers early so that we get a bit of feedback if things go wrong.
2. Parse the device tree, enabling more informed initialisation.
3. Set up the mailbox (rpi-specific, but allows us to extract more information from the hardware, and interact directly in certain ways).
4. Initialise the [console](#devcons), ensuring we can use `port::println` to write to the UART.
5. Write out some useful system information.
6. Set up [virtual memory](#virtual-memory) in rust.  (After switching we no longer need the pagetables set up in early initialisation.)
7. Write out the page tables.
8. That's it.  Loop.

### Virtual Memory

VM initialisation proceeds as follows:
1. Set up the page allocator.  We use a bitmap allocator defined in `pagealloc.rs`.  This makes use of the `earlypages` reserved in the `kernel.ld` linker script.  The 16KiB reserved here allows us to manage 16\*4096\*8 pages.  Each bit represents 4096KiB, allowing us to map 2GiB of RAM.  The allocator assumes everything except the early pages are allocated at first.  This is to ensure we don't mistakenly allocate a page that should not be available (e.g. will be mapped below).
2. Build an initial memory map.  This is also a bit rpi4-specific.  It maps:
  - Device tree DTB
  - Kernel text, data and BSS
  - MMIO
3. We map each of the above ranges according to the defined entry flags.
4. We now mark all the unallocated pages free.

To document:
- The innards of page table mapping.

Future Improvements:
- The page table management code needs to be able to obtain new page frames while updating page tables.  It's simplest to ensure we have at least X page frames pre-mapped, and available in the page allocator.
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
