# How r9 Works

**Please update with any material code change!**

This document summarises the flow of each implementation as it currently stands. We do this because r9 is a hobby project that is dipped in and out of, with months between dips. In that context, it's really hard to remember how it works and why. A document like this also serves as a useful introduction to the project.

## aarch64

### Early initialisation (Assembly)

Execution begins in `l.S` at the `start` label and is focused on Raspberry 4 ATM. Supporting rpi5 should be straightforward, but anything beyond that would get complex. That would be a strong argument to move some of this code to rust, although that code would probably need to be segrated fromt he rest of the rust code.

The code in `l.S` exists to bring us to a state where we can run kernel code in the OS level (EL1). It defines no constants: `main.rs` passes them in as `global_asm!` operands, with the register values built from `aarch64_cpu`'s fields. In order to get there, we run through a number of steps:

1. Park every core except core 0 (`dnr`).
2. Drop to EL1 from EL2, enabling AArch64 and the FPU for EL1 on the way, and making sure EL2 doesn't trap the FPU (`CPTR_EL2`). QEMU's `-kernel` enters at EL2, and the Pi's firmware is expected to. r9 doesn't support entering at EL3, so a core that does parks.
3. Set all of `SCTLR_EL1` to a known value, with the MMU and caches off, since most of it resets to UNKNOWN values. Then invalidate the data cache over the BSS, and clear it. Everything written before the MMU is on is in the BSS: the boot stack, the early page table area (`earlyvm_pagetables` in `kernel.ld`) and the zeroed statics. Those writes go straight to memory, but lines left from before r9 started could be evicted over them, or hide them once the caches are on, so the invalidate comes before any write.
4. Set up the initial stack (`KSTACKSIZE`, 16KiB), defined in BSS at the bottom of the file, using its physical address since the MMU is still off.
5. Call `pre_mmu_init` in `pre_mmu/mod.rs`, which does the rest of the work before the MMU is on. First it sets up the miniuart for some kind of debugging (`init_early_uart_rpi4` in `pre_mmu/util.rs`). This is a bare bones set up - a rust driver will take over later. It allows us to do something similar to printf-style debugging - writing a single character to the UART at specific points. Once the UART is initialised, we output a single '.' so we at least know we've gotten that far.
6. Then `pre_mmu_init` calls `init_vm` in `pre_mmu/vminit.rs`, which builds the initial page tables from the early page table area while the MMU is still off. It maps the kernel image, the DTB and the kmap region's tables at KZERO + their physical addresses (`ttbr1_el1`), with the recursive entry 511 in the kernel root. It also identity maps the first 1GiB (`ttbr0_el1`), so the code in `l.S` can keep running once the MMU is enabled and then jump into the higher half. It sets `TCR_EL1` and `MAIR_EL1` too.
7. Enable the MMU and caches, by writing all of `SCTLR_EL1` again, jump to the higher half, and move the stack to its virtual address.
8. Jump into the rust code, entrypoint `main.rs::main9`, passing the DTB's physical address.

### Initialisation (Rust)

We're now in rust, in `main9`, and at a high level we:

1. Point `TPIDR_EL1` at this CPU's `Mach` (`mach.rs`), then set up the exception handlers early so that we get a bit of feedback if things go wrong.
2. Replace the boot identity map in `ttbr0_el1` with the empty user root, so low addresses, including null, fault. The kernel's own page tables were built before the MMU was enabled (see above).
3. Parse the device tree, enabling more informed initialisation.
4. Set up the page allocator, with every page not used by the kernel image or the DTB.
5. Initialise the [console](#devcons), ensuring we can use `port::println` to write to the UART.
6. Set up the mailbox (rpi-specific, but allows us to extract more information from the hardware, and interact directly in certain ways).
7. Write out some useful system information, and the kernel page tables.
8. Run the boot tests in `boottest.rs`, panicking if one fails: `kmap`, switching between kernel contexts with `swtch`, releasing an address space, and a [process](#processes) for each of a set of small assembled test programs and `user/elftest`. These make syscalls, write to the console and exit, and the tests check each one's exit status and that everything it allocated is freed.
9. Run [echo](#userspace) as a process, with argv `["echo", "hello", "world"]`, and print how it exited: `echo: exited` if it succeeded.

### Exceptions

`trap.S` holds the exception vector table. Every entry pushes a `TrapFrame` (all general purpose registers, plus `ESR_EL1`, `ELR_EL1`, `FAR_EL1`, `SP_EL0` and `SPSR_EL1`) onto the current stack, calls `trap` in `trap.rs`, then leaves through `trap_return`. `trap_return` takes the return state from the frame rather than the live registers, so a handler can change where we return to, or what's returned in x0.

- Exceptions from EL1 are taken on the interrupted kernel stack.
- Exceptions from EL0 are taken on whatever `SP_EL1` was when we entered EL0. `to_user` enters EL0 by building a `TrapFrame` at the top of the process's kernel stack and jumping to `trap_return`, which leaves `SP_EL1` at the top of that stack. A process's first kernel context starts below that frame's space, as Plan 9 keeps the top of each kernel stack for its `Ureg`.

The kernel is built softfloat (see `lib/aarch64-unknown-none-elf.json`), so it never uses the FP/SIMD registers and exception entry doesn't need to save them. `to_user` zeroes the user FP/SIMD state, so one process can't read another's. It will need saving once processes are switched between.

IRQs, FIQs, SErrors and debug exceptions are masked in both EL1 and EL0 until we have a use for them.

A syscall is `svc #0`, with the number in x8 and the arguments in x0–x5, and the result comes back in x0, as on Linux. The `svc` immediate is ignored. Every other register is preserved, since `trap_return` restores the whole frame. The numbers are Plan 9's, in `sys/src/lib.rs` (the `r9-sys` crate, after Plan 9's `sys.h`), which the kernel and the user library share, and `port::syscall::syscall` dispatches them. So far there's `SYSR1`, which returns 0, `EXITS`, `BRK_`, `ERRSTR` and `PWRITE`. A syscall that fails returns -1, and its error string is kept in the process (`Proc::errstr`, `ERRMAX` bytes), as Plan 9's `syserrstr`; a syscall that succeeds leaves it alone. `ERRSTR` swaps that string with the caller's buffer, as Plan 9's `errstr` does. An unknown number kills the process with the status `sys: bad sys call`. An exit status is a byte string of at most `ERRMAX - 1` bytes, and empty means success, as in Plan 9.

There are no file descriptors yet: `pwrite` to fd 1 or 2 writes to the console, ignoring the offset, as Plan 9's `conswrite` does, and any other fd fails. The buffer is copied through a 256-byte stack buffer, so the console lock is never held while touching user memory. Console output is polled, with IRQs masked, so a long write holds the CPU until the UART has sent it all.

### Kernel context switching

`swtch(save, next)` in `swtch.rs` saves the current kernel context's callee-saved registers (x19-x29, the return address and `sp`) into the `save` `Label`, loads them from `next`, and returns into wherever `next` last called `swtch`. `Label::new_context` builds a label that starts a never-returning function on a fresh stack. This is only for switching between kernel contexts at EL1; entering EL0 is done with `to_user`.

### Processes

A process is a `port::dat::Proc<A>`, and its lifecycle is in `port/src/proc.rs`, following Plan 9's `port/proc.c`. `A` is the architecture, which implements two traits from `port::dat`: `MMU`, for the process's address space (its `Pmmu`), and `ProcArch`, for its kernel stack, kernel context, and the per-CPU `up` and scheduler context. On aarch64 that's `Arm64` in `proc.rs`, and the per-CPU state is `Mach` in `mach.rs`, found through `TPIDR_EL1`.

- `newproc(start)` allocates an address space and a kernel stack, and a context that starts in `proc_start`.
- `run(p)` makes `p` the current process (`up`), and switches to it from the scheduler context. There's no scheduler yet, so a process runs until it exits.
- `proc_start` switches to the process's address space and calls `start`, as Plan 9's `kproc` runs a function in a new process. `start` loads the program and enters EL0 with `enter_user` (`proc.rs`), which calls `to_user` on the process's kernel stack.
- `pexit(status)` records the exit status and switches back to the scheduler context, where `run` reaps the process: its address space is released, every page mapped in it is freed, and its kernel stack is freed.

### User Address Space

User addresses are translated through `ttbr0_el1`. The layout is defined in `param.rs`:

- `UTZERO` (0x10000): start of user text. The pages below are unmapped to catch null pointers.
- The bss follows the text, and `brk_` grows it towards the stack. The assembled test programs in `boottest.rs` start with an empty bss at the page after their text.
- `USTKTOP` (2^47): top of the user stack, which is `USTKSIZE` (64KiB), mapped up front, with nothing mapped below it.
- `UADDRTOP` (0x0000_ff80_0000_0000): end of the user address space. Root entry 511 above it recursively maps the user page table. `map_to` refuses addresses in the recursive range, or outside the half of the address space its table translates.

No user page is ever both writable and executable: `port` maps user pages with `MMU::put` and a `Perm`, which is either `Text` (read-only, executable) or `Data` (read-write, never executable). AArch64 has no "kernel read-write, user read-only" permission, so the kernel fills user pages through [`kmap`](#virtual-address-map), never through their user address. `port` reaches a page through `MMU::kmap(pa, f)`, which maps it only while `f` runs. `port::segment::map_zeroed` zeroes each page this way before mapping it, so a process never sees another's data, and if it fails partway it unmaps and frees what it mapped. `MMU::unput` unmaps a page and hands it back, invalidating its TLB entry.

`brk_` follows Plan 9's `ibrk`: it moves the end of the process's bss, rounded up to a page. Growing maps zeroed pages straight away, as there's no demand paging. Shrinking unmaps and frees them. It fails without changing anything if it would grow into the stack or the unmapped guard page below it, or needs more pages than are free. Text is written through kmap but executed through its user address, so `put` with `Text` first cleans the data cache and invalidates the instruction cache (`vm::sync_icache`), as Plan 9's `putmmu` does for text pages, so the instructions fetched are the ones written. A process owns every page mapped in its address space, until there are segments.

Programs are static aarch64 ELF executables. `port::elf` validates an image without allocating, treating it as untrusted: every read is bounds-checked, and each loadable segment must be text (R+X), rodata (R) or data (R+W), never writable and executable, in ascending order with no two sharing a page. `port::exec::load` then checks the layout, before mapping anything: segments lie between `UTZERO` and the guard page below the stack, with at most one data segment, at the top. It maps each segment page by page, zeroed, with its file bytes copied in through `kmap`, and sets the bss to run from the end of the data's file bytes, as Plan 9's exec does. A failure partway isn't undone: the caller exits the process. `port::exec::exec` loads a program this way, maps its stack, and pushes its arguments: from `USTKTOP` down, the argument strings, each NUL-terminated, then the argv array of pointers to them, ending in a null pointer, with `sp` 16-byte aligned below it, as AAPCS64 needs. The arguments may use at most a quarter of the stack, as in Linux. It returns the registers the program starts with, which `to_user` loads: the pc, `sp`, and x0 = argc and x1 = argv. There's no `Tos`, environment or auxiliary vector. The boot tests run [`user/elftest`](#userspace) through it.

Syscalls reach user memory through `port/src/uaccess.rs`, after Plan 9's `validaddr`. `validate_user_range` checks that a range is below `UADDRTOP`, then checks each page with `MMU::user_page_ok`. On aarch64 that's `AT S1E0R` or `AT S1E0W` (`vm::user_page_ok`), which asks the MMU whether EL0 could make the access. `copy_from_user` and `copy_str_from_user` then use the user address directly. That's only correct while nothing can change the mapping between the check and the access: one CPU, no preemption, one thread per process, and PAN off. A bad address kills the process with the status `sys: bad address in syscall`.

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
0xffff_ff00_0020_0000  +--------------------------------------------------+ KSTACKBASE
                       | kernel stacks: 64 slots of 32KiB, each a 16KiB   |
                       | unmapped guard below a 16KiB stack          RW   |
                       +--------------------------------------------------+
                       |                       ...                        |
0xffff_ff80_0000_0000  +--------------------------------------------------+
                       | root entry 511: recursive map of the kernel      |
                       | tables                                           |
0xffff_ffff_ffff_ffff  +--------------------------------------------------+
```

- There's no direct map of RAM. `paddr(va)` in `kmem.rs` converts only for what's mapped at KZERO + its physical address: the kernel image and the DTB.
- Releasing an address space (`vm::release`) first switches to the empty one, which flushes the TLB, then frees its page tables through kmap, since they're no longer active. Each page mapped in it is passed to the caller, which frees it if it owns it: not every page mapped in an address space belongs to it.
- Kernel stacks (`kstack.rs`) come from their own region. Each stack has an unmapped guard below it, so an overflow faults instead of corrupting memory. The kernel heap is a static in the kernel image's BSS.
- After the MMU is on, page tables are only ever edited while they're active: `map_phys_range` takes no table, and always edits the active root of its half through the recursive window. To map into a user address space, switch to it first. When there's no user address space, `ttbr0_el1` holds an empty root, which has only its recursive entry. Mapping user pages then fails with `NoAddressSpace`, and every other user address faults, including null.
- Page tables are reached through the recursive entries. Any other physical page, such as a user page, is reached through `kmap` (`kmap.rs`, as in Plan 9). `kmap(pa)` maps the page in one of the current CPU's slots and returns a guard. Dropping the guard unmaps it and invalidates that address in the TLB. The tables for the region are built before the MMU is on, so `kmap` never allocates. It's for thread context only.
- The kernel image sections are 2MiB aligned by `kernel.ld`, so they can have their own permissions using 2MiB blocks.
- Before the switch to the higher half, `ttbr0_el1` holds an identity map of the first 1GiB, so `l.S` keeps running when the MMU is turned on. It's replaced by the user page table.

VM initialisation proceeds as follows:

1. **Before the MMU is on,** `init_vm` (`pre_mmu/vminit.rs`) builds the initial page tables. It takes table pages from the 32 pages reserved as `earlyvm_pagetables` in `kernel.ld`, with its own small allocator, `EarlyPageAllocator`. It maps:
   - the kernel image at KZERO + its physical address: text (including boottext) read-only and executable, rodata read-only, data and BSS read-write, all with 2MiB blocks
   - the DTB, read-only
   - the tables for the kmap and kernel stack regions, with no pages mapped
   - an identity map of the first 1GiB in `ttbr0_el1`, so `l.S` keeps running when the MMU is turned on

   It also sets `TCR_EL1` and `MAIR_EL1`. MMIO isn't mapped here: each driver maps its registers later, at KZERO + 16TiB (`deviceutil.rs`).
2. **At the start of `main9`,** the empty user root replaces the identity map in `ttbr0_el1`.
3. **`main9` then sets up the page allocator,** `port/src/pagealloc.rs`, which is shared by every architecture. It's a bitmap allocator (`port/src/bitmapalloc.rs`) whose bitmaps are a static: 32 bitmaps of 4KiB, one bit per 4KiB page, so it can track up to 4GiB. Every page starts marked as allocated. `init_page_allocator` then frees the first DTB memory region, except the ranges already in use: the kernel image (including the early page tables in its BSS) and the DTB. From here on, page tables, kmap and drivers all allocate pages from it.

To document:

- The innards of page table mapping.

Future Improvements:

- The page allocator only uses the first DTB memory region.
- Replace the page allocator with something a little more sophisticated with a higher upper bound.
- If the page allocator made use of something like a free list, we could add the early pages to that free list, and avoid the weird hack of marking everything as allocated up front and having to later mark as unallocated.

## x86_64

## riscv64

## userspace

User programs live in `user/`, and are built for aarch64 only, by xtask, for the stock `aarch64-unknown-none` target. They're static (`-C relocation-model=static`), laid out by `user/lib/user.ld`, with 4KiB pages (`-z max-page-size=4096`). The linker script puts text at `UTZERO`, then rodata, then data and bss, each starting on a page, and `end` after the bss.

- `user/libc` (the `r9-libc` package) is the user library, after Plan 9's libc. `_start` is entered like a call, with x0 = argc and x1 = argv, and calls the program's `main(argc, argv)`, then `exits("main")` if it returns, as Plan 9's `_main` does. It has the system calls (`exits`, `pwrite` and `write`, `brk_`, `brk` and `sbrk`, from `end`, and `errstr`), Plan 9's `rerrstr`, which reads the error string without changing it, and `Errstr`, which formats it as Plan 9's `%r`. It also has a global allocator over `sbrk` that never frees, `fprint` and `print!`, and a panic handler that prints the message and exits with `panic`. On any target but r9 it's empty, so a crate that uses it can test its library on the host.
- `user/echo` is Plan 9's `echo.c`, ported line by line. Its logic (`split`, `size`, `fill`) is a library, tested on the host.
- `user/elftest` is the ELF loader's test program: its data spans two pages, and its bss runs onto another.

User programs use FP/SIMD freely: EL0 may (`CPACR_EL1.FPEN`), and `to_user` zeroes the state, which isn't saved, as only one process runs at a time.

xtask builds the user programs before anything that embeds them, and passes their directory to those compiles in `R9_USER_BIN`. The kernel and `port`'s tests embed them with `include_bytes!(concat!(env!("R9_USER_BIN"), "/elftest"))`, so compiling either without xtask fails, rather than embedding a stale program. For an editor's rust-analyzer, set `R9_USER_BIN` to the directory a `cargo xtask build --arch aarch64` leaves them in, `target/aarch64-unknown-none/debug`, as an absolute path. The kernel's build steps exclude the user crates, which aren't for its target spec or linker script.

`cargo xtask smoke --arch aarch64` boots r9 under QEMU and checks that echo runs: the output must have the lines `hello world` and `echo: exited`, and no panic. The kernel doesn't exit QEMU, so the smoke test stops it once echo has exited or the kernel has panicked, or after 60 seconds.

With `--semihosting`, which CI uses, the kernel is built with the `semihosting` feature, and makes QEMU exit through Arm semihosting (`aarch64/src/semihosting.rs`): with status 0 once echo exits successfully, and 1 if echo fails or the kernel panics. The smoke test waits for QEMU to exit, and its status must be 0 too, so a failure shows up straight away rather than as a timeout. Semihosting is an undefined instruction on hardware without a debugger, so that image is for QEMU only, and a normal build contains no semihosting call.

## port

`port` is where all the shared subsytems are defined.

### devcons

### fdt

### mcslock
