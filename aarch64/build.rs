// Relink when xtask's linker script changes.  Cargo only sees it as a linker
// argument, so it wouldn't otherwise know the kernel depends on it.  xtask
// rewrites the script only when its contents change.
fn main() {
    println!("cargo:rerun-if-env-changed=R9_LINKER_SCRIPT");
    if let Ok(script) = std::env::var("R9_LINKER_SCRIPT") {
        println!("cargo:rerun-if-changed={script}");
    }
}
