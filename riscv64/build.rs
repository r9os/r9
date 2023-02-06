fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(board = "virt")]
    println!("cargo:rustc-link-arg=-Triscv64/src/board/virt/kernel.ld");

    #[cfg(board = "allwinner")]
    println!("cargo:rustc-link-arg=-Triscv64/src/board/allwinner/kernel.ld");

    Ok(())
}
