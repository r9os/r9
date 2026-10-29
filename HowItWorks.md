# How r9 Works

**Please update with any material code change!**

This document summarises the flow of each implementation as it currently stands. We do this because r9 is a hobby project that is dipped in and out of, with months between dips. In that context, it's really hard to remember how it works and why. A document like this also serves as a useful introduction to the project.

## aarch64

### Early initialisation (Assembly)

Execution begins in `l.S` at the `start` label and is focused on Raspberry 4 ATM. Supporting rpi5 should be straightforward, but anything beyond that would get complex. That would be a strong argument to move some of this code to rust, although that code would probably need to be segrated fromt he rest of the rust code.

The code in `l.S` exists to bring us to a state where we can run kernel code in the OS level (EL1). It defines no constants: `main.rs` passes them in as `global_asm!` operands, with the register values built from `aarch64_cpu`'s fields. In order to get there, we run through a number of steps:

1. Park every core except core 0 (`dnr`).
2. Drop to EL1 from EL2, enabling AArch64 and the FPU for EL1 on the way, and making sure EL2 doesn't trap the FPU (`CPTR_EL2`). It also sets the generic timer's EL2 controls, which reset to UNKNOWN values: EL1 may read the physical counter (`CNTHCTL_EL2`), and virtual time is physical time (`CNTVOFF_EL2 = 0`). QEMU's `-kernel` enters at EL2, and the Pi's firmware is expected to. r9 doesn't support entering at EL3, so a core that does parks.
3. Set all of `SCTLR_EL1` to a known value, with the MMU and caches off, since most of it resets to UNKNOWN values. Do the same for the generic timer's EL1 controls: EL0 can't read the counters or timers (`CNTKCTL_EL1 = 0`), and the virtual timer is disabled until something arms it (`CNTV_CTL_EL0 = 0`). Then invalidate the data cache over the BSS, and clear it. Everything written before the MMU is on is in the BSS: the boot stack, the early page table area (`earlyvm_pagetables` in `kernel.ld`) and the zeroed statics. Those writes go straight to memory, but lines left from before r9 started could be evicted over them, or hide them once the caches are on, so the invalidate comes before any write.
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
5. Initialise the [console](#devcons), ensuring we can use `port::println` to write to the UART. Then register the generic timer's virtual counter with `port::clock`, which panics if firmware left its frequency at 0, now that the panic can be seen.
6. Set up the mailbox (rpi-specific, but allows us to extract more information from the hardware, and interact directly in certain ways).
7. Write out some useful system information. Set up the interrupt controller, the GIC-400 (`gic.rs`), with every interrupt disabled, as Linux does. Write out the kernel page tables.
8. Continue on a newly allocated kernel stack (`on_kernel_stack`): unlike the boot stack, it has an unmapped guard below it, so an overflow faults and is reported. Then run the GIC's boot tests, which take the timer's interrupt with and without a handler of their own. Start the clock (`clock::start`): the virtual timer's interrupt goes to the timer queue, and the HZ clock starts. Unmask IRQs, and run the other boot tests in `boottest.rs`, panicking if one fails: that the counter runs and never goes backwards, that `ticks` advances at HZ, that a one-shot timer fires once at its deadline, `kmap`, switching between kernel contexts with `swtch`, releasing an address space, and a [process](#processes) for each of a set of small assembled test programs and `user/elftest`. These make syscalls, write to the console and exit, and the tests check each one's exit status and that everything it allocated is freed.
9. Run the user programs: [sleep](#userspace) with argv `["sleep", "1"]` and [factor](#userspace) with argv `["factor", "9007195909437503"]` — the largest product of two primes below 2 to the 53, so it's exact as a double, and factor divides it for as long as it can — both readied before either runs, so sleep sleeps its second while factor divides, and each exit status is printed as it's collected: `sleep: exited` and `factor: exited` if they succeeded. Then run [echo](#userspace) as a process, with argv `["echo", "hello", "world"]`, and print how it exited: `echo: exited` if it succeeded.

### Exceptions

`trap.S` holds the exception vector table. Every entry pushes a `TrapFrame` (all general purpose registers, plus `ESR_EL1`, `ELR_EL1`, `FAR_EL1`, `SP_EL0` and `SPSR_EL1`) onto the current stack, calls `trap` in `trap.rs`, then leaves through `trap_return`. `trap_return` takes the return state from the frame rather than the live registers, so a handler can change where we return to, or what's returned in x0.

- Exceptions from EL1 are taken on the interrupted kernel stack.
- Exceptions from EL0 are taken on whatever `SP_EL1` was when we entered EL0. `to_user` enters EL0 by building a `TrapFrame` at the top of the process's kernel stack and jumping to `trap_return`, which leaves `SP_EL1` at the top of that stack. A process's first kernel context starts below that frame's space, as Plan 9 keeps the top of each kernel stack for its `Ureg`.
- After reserving the frame, each entry checks that `sp` is still inside its stack, as Linux arm64'"'"'s `kernel_ventry` does. Every EL1 stack is the upper half of a block aligned to twice its size: the kernel stacks'"'"' slots, the boot stack, and the overflow stack. So a valid `sp` has bit 14 set, and one that has run into the guard below has it clear. On an overflow, the entry switches to the overflow stack (`trap.S`), and `stack_overflow_unsafe` prints the frame and panics with the overflowed `sp`, rather than faulting again forever.

The kernel is built softfloat (see `lib/aarch64-unknown-none-elf.json`), so it never uses the FP/SIMD registers, and exception entry doesn't need to save them: while a process is in the kernel, they still hold its own. `schedule` saves them (q0-q31, FPCR and FPSR) into the process when it switches back, and loads the next process's before switching to it, through `ProcArch::fp_save` and `fp_restore`. A new process's are zero, so one process can't read another's.

IRQs are unmasked at EL1 from `main9_rest` on, once the clock has started; until then only the GIC's boot tests unmask them, briefly. They're unmasked at EL0 too. FIQs, SErrors and debug exceptions stay masked everywhere, until we have a use for them. Code that mustn't be interrupted will run under `port::irq::masked(|| …)`, or hold a lock taken with `Lock::ilock`, which masks IRQs until it's released, as Plan 9's `ilock` does. Code in `port` isn't generic over the architecture, so the architecture registers its masking functions with `port::irq::init` (aarch64 masks IRQs and FIQs with DAIF, in `trap::init`), and until it does, masking does nothing. `port::irq` also tracks whether this CPU is handling an interrupt, and `kmap` panics if it is. The page allocator and the heap panic too, since their locks don't mask IRQs, so a handler mustn't take them. `schedule` masks IRQs across its switch into a process, and restores them when it switches back, since a process switches back from inside an exception. `halt()` and the panic handler mask IRQs first (`mask_interrupts`).

A syscall is `svc #0`, with the number in x8 and the arguments in x0–x5, and the result comes back in x0, as on Linux. The `svc` immediate is ignored. Every other register is preserved, since `trap_return` restores the whole frame. The numbers are Plan 9's, in `sys/src/lib.rs` (the `r9-sys` crate, after Plan 9's `sys.h`), which the kernel and the user library share, and `port::syscall::syscall` dispatches them. So far there's `SYSR1`, which returns 0, `EXITS`, `SLEEP`, which yields for zero or negative milliseconds and otherwise sleeps at least one tick, `BRK_`, `ERRSTR`, and the file calls described below. A syscall that fails returns -1, and its error string is kept in the process (`Proc::errstr`, `ERRMAX` bytes), as Plan 9's `syserrstr`; a syscall that succeeds leaves it alone. `ERRSTR` swaps that string with the caller's buffer, as Plan 9's `errstr` does. An unknown number kills the process with the status `sys: bad sys call`. An exit status is a byte string of at most `ERRMAX - 1` bytes, and empty means success, as in Plan 9.

Any other synchronous exception from EL0 kills the process, rather than the kernel: `trap` calls `pexit` with Plan 9's text for faults and breakpoints (`bcm/trap.c`), and its `sys: trap:` form for the rest. An abort exits with `sys: trap: fault read va=0x…`, or `write`. `BRK` exits with `sys: breakpoint`. Anything else, such as an undefined instruction or a trapped `wfi`, exits with `sys: trap: <what> pc=0x…`. Only synchronous exceptions (and SErrors) write `ESR_EL1`, so an IRQ from EL0 isn't mistaken for a fault.

An IRQ from EL1 (`el1h_64_irq`) or 64-bit EL0 (`el0t_64_irq`) goes to `gic::irq`, between `port::irq::enter_interrupt` and `exit_interrupt`. As Plan 9's `irq`, it acknowledges the interrupt (`GICC_IAR`), calls the handler registered for it with `gic::attach`, and ends it (`GICC_EOIR`), one interrupt per exception. A handler runs with IRQs masked, and must quieten a level-triggered source, such as the timer, before it returns, or it's taken again at once. An interrupt with no handler is disabled, then reported with `iprint`. Any other exception, a FIQ or one from EL1 included, prints the `TrapFrame` and panics.

A process reaches devices through files, as in Plan 9: `OPEN`, `CLOSE`, `PREAD`, `PWRITE`, `DUP` and `SEEK` work on its file descriptors, each an open channel (`port::chan::Chan`) on a device (`port::dev::Dev`). A name is either a device's own, a `#` and its letter, such as `#c/cons`, or one in `/dev`, which stands for `#c`, as Plan 9's namespace binds it; there's no `bind` yet, so that's the only directory outside `#`, and every process sees it. As in Plan 9, doubled slashes and `.` elements are skipped, and a name ending in `/` must be a directory; `..` isn't supported yet. `newproc` opens `#c/cons` on fds 0, 1 and 2, which works whatever `/dev` is. The console device's files are `cons`, the console's output; `time`, which reads as Plan 9's, with the seconds and nanoseconds since the clock's counter started (on a Pi 4, power-on, before the firmware ran) until a write of seconds sets it, then the clock's count and frequency; and `pid`, the reader's pid. Devices aren't generic over the architecture, so the architecture registers its current process with `port::proc::init_current`, as it does its clock. A `pread` at an explicit offset still advances the channel's offset by the bytes read, as Plan 9's `read` does; a `pwrite` at one doesn't, as Plan 9's `write` doesn't. A write is copied through a 256-byte stack buffer, so the console lock is never held while touching user memory. Console output is polled, so a long write holds the CPU until the UART has sent it all.

### Kernel context switching

`swtch(save, next)` in `swtch.rs` saves the current kernel context's callee-saved registers (x19-x29, the return address and `sp`) into the `save` `Label`, loads them from `next`, and returns into wherever `next` last called `swtch`. `Label::new_context` builds a label that starts a never-returning function on a fresh stack. This is only for switching between kernel contexts at EL1; entering EL0 is done with `to_user`.

### Processes

A process is a `port::dat::Proc<A>`, and its lifecycle is in `port/src/proc.rs`, following Plan 9's `port/proc.c`. `A` is the architecture, which implements two traits from `port::dat`: `MMU`, for the process's address space (its `Pmmu`), and `ProcArch`, for its kernel stack, kernel context, the per-CPU `up` and scheduler context, and the run queue. On aarch64 that's `Arm64` in `proc.rs`, and the per-CPU state is `Mach` in `mach.rs`, found through `TPIDR_EL1`.

- `newproc(start)` returns a boxed process with an address space, a kernel stack, and a context that starts in `proc_start`.
- `ready(p)` puts it on the run queue: first in, first out, linked through the processes themselves, so queueing never allocates.
- `schedule()` runs in the caller's context, which becomes the scheduler context, until the queue is empty. For each process it takes, it gives it a quantum of 100 ms, switches to its address space and then to it, with IRQs masked across the switch. When the process switches back, `schedule` re-queues it if it yielded (`yield_now`), or reaps it if it exited. It returns each exit's pid and status. Every switch goes through the scheduler context, so a process is reaped off its own stack, and re-queued only once its context is saved. `run(p)` readies `p`, schedules, and returns `p`'s status.
- `proc_start` calls `start`, as Plan 9's `kproc` runs a function in a new process. `start` loads the program and enters EL0 with `enter_user` (`proc.rs`), which calls `to_user` on the process's kernel stack.
- `pexit(status)` records the exit status and switches back to the scheduler context, which reaps the process: its address space is released, every page mapped in it is freed, and its kernel stack is freed.
- `sleep(r, cond)` waits on a `Rendez` until `cond` holds, as Plan 9's does: with IRQs masked throughout, it checks `cond` with `r` locked, and if it's false, records the process on `r`, marks it `Sleeping`, and switches to the scheduler, which hands the process to `r` before unmasking IRQs. `wakeup(r)`, from any context, takes the sleeper and readies it, without allocating. With nothing ready but a process asleep, the scheduler waits with `wfi` (`ProcArch::idle`), with IRQs masked so a wakeup between its check and the wait isn't lost: a pending IRQ ends `wfi` anyway, and the scheduler then unmasks to take it.
- `tsleep(r, cond, d)` is `sleep` with a timeout, as Plan 9's: the process's own `Timer` fires after `d`, and its `twakeup` clears the process's `trend`, its pointer to `r`, which makes the sleep's condition hold; `cond` holds it too. `SLEEP` is `tsleep` on the process's own `sleep` `Rendez`, with a condition that never holds, so only its timeout wakes it. The process's timer is removed before the process is reaped, since a process can't exit while it's in `tsleep`.
- A process is preempted on its way back to EL0, from a syscall or an IRQ, once its quantum has run out and another process is ready (`preempt_due`). `trap` calls `yield_now` after the IRQ's EOI, since a GICv2 holds the CPU interface's running priority until then. The kernel itself isn't preempted.

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
- `user/sleep` is Plan 9's `sleep.c`, ported line by line. Its argument parsing (`parse`) is a library, tested on the host.
- `user/factor` is Plan 9's `factor.c`, ported line by line, for the arguments path. Its factorization (`factor`) is a library, tested on the host.
- `user/elftest` is the ELF loader's test program: its data spans two pages, and its bss runs onto another.

User programs use FP/SIMD freely: EL0 may (`CPACR_EL1.FPEN`). Each process has its own FP/SIMD registers (q0-q31, FPCR, FPSR), zero in a new process, and the scheduler saves and restores them at every switch, so they survive a process being preempted.

xtask builds the user programs before anything that embeds them, and passes their directory to those compiles in `R9_USER_BIN`. The kernel and `port`'s tests embed them with `include_bytes!(concat!(env!("R9_USER_BIN"), "/elftest"))`, so compiling either without xtask fails, rather than embedding a stale program. For an editor's rust-analyzer, set `R9_USER_BIN` to the directory a `cargo xtask build --arch aarch64` leaves them in, `target/aarch64-unknown-none/debug`, as an absolute path. The kernel's build steps exclude the user crates, which aren't for its target spec or linker script.

The linker script works the same way. xtask fills in `kernel.ld`'s placeholders from the config, writes the result into `target/`, but only when it has changed, and passes its path in `R9_LINKER_SCRIPT`. Each kernel crate's `build.rs` tells cargo to relink when that file changes. Cargo only sees the script as a linker argument, so it wouldn't otherwise know the kernel depends on it.

`cargo xtask smoke --arch aarch64` boots r9 under QEMU and checks that the user programs run: the output must have factor's argument, its factors, a blank line, `factor: exited` and `sleep: exited`, and `hello world` and `echo: exited` after, all with no panic. The kernel doesn't exit QEMU, so the smoke test stops it once echo has exited or the kernel has panicked, or after 60 seconds.

With `--semihosting`, which CI uses, the kernel is built with the `semihosting` feature, and makes QEMU exit through Arm semihosting (`aarch64/src/semihosting.rs`): with status 0 once sleep, factor and echo all exit successfully, and 1 if any fails or the kernel panics. The smoke test waits for QEMU to exit, and its status must be 0 too, so a failure shows up straight away rather than as a timeout. Semihosting is an undefined instruction on hardware without a debugger, so that image is for QEMU only, and a normal build contains no semihosting call.

## port

`port` is where all the shared subsytems are defined.

### clock
The monotonic clock, after Plan 9's `fastticks`: a counter that counts up at a fixed frequency. Code in `port` isn't generic over the architecture, so the architecture registers its functions with `port::clock::init`, as it does for `port::irq`, and until it does there's no clock. aarch64 registers the generic timer's virtual counter (`CNTVCT_EL0`), read after an `isb`, so it isn't read ahead of earlier instructions, and its frequency (`CNTFRQ_EL0`, 62.5 MHz on QEMU's raspi4b). `to_counts` converts a `Duration` to a count, rounding up so a deadline is never early, and `to_duration` converts back, rounding down. Both are exact, through `u128`.

### timer
Timers, as Plan 9's `portclock.c`: one queue, sorted by deadline, with the clock's timer armed for the earliest (`port::clock`'s `set_timer`, which aarch64 implements with the virtual timer's `CNTV_CVAL_EL0` and `CNTV_CTL_EL0`). A `Timer` is `Relative` (fire once) or `Periodic`, with a callback `fn(*mut ())` that runs in interrupt context, its argument set by the timer's owner, as Plan 9's `ta`. `timer::add` queues one, moving it if it's already queued, and `timer::remove` unqueues it. `timer::add_unowned` queues one its caller doesn't own for the program's life, like a process's, and the caller must remove it before it's freed, as Plan 9's `tsleep` does with the process's own timer. `timer::interrupt`, called from the clock's interrupt, fires each timer that's due with the queue's lock dropped around its callback, and re-queues a periodic one at its last deadline plus its period, so it doesn't drift. `timer::start` starts the HZ clock, a 100 Hz periodic timer whose callback advances `ticks`. There's one queue, for the one CPU; its timers are mostly statics, and a process's `tsleep` adds its own.

### devcons
The console has two ways to print, as Plan 9's does. `print` and `println` serialize their output with `PRINTLOCK`, taken with IRQs unmasked, so they're for thread context only, and panic in an interrupt handler. `iprint` and `iprintln` are for interrupt handlers, panics and exceptions: they mask IRQs, skip `PRINTLOCK`, and wait about 100 ms for `IPRINTLOCK`, as Plan 9's `iprintcanlock` does, before printing anyway (without a [clock](#clock), they try once), so a CPU that stopped part way through printing can't stop another reporting. They mask IRQs for their whole output, about 87 µs a character at 115200 baud, so a handler should print at most a line. Both reach the UART through its own lock, taken with IRQs masked, held only to copy the reference, and always taken last.

### fdt

### mcslock
