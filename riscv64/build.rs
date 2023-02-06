fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "virt")]
    println!("cargo:rustc-link-arg=-Triscv64/src/board/virt/kernel.ld");

    #[cfg(feature = "allwinner")]
    println!("cargo:rustc-link-arg=-Triscv64/src/board/allwinner/kernel.ld");

    Ok(())
}
