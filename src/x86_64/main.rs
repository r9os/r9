use crate::println;

#[no_mangle]
pub extern "C" fn main9() {
    println!();
    println!("r9 from the Internet");
    println!("looping now");
    #[allow(clippy::empty_loop)]
    loop {}
}
