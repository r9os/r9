//! factor's logic, from Plan 9's factor.c, apart from its output, so that
//! it can be tested on the host.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use libc::{modf, sqrt};

/// factor.c's wheel: the gaps between the numbers coprime to 2*3*5*7 = 210.
/// There are 48, and they sum to 210.
const WHEEL: [f64; 48] = [
    2.0, 10.0, 2.0, 4.0, 2.0, 4.0, 6.0, 2.0, 6.0, 4.0, 2.0, 4.0, 6.0, 6.0, 2.0, 6.0, 4.0, 2.0, 6.0,
    4.0, 6.0, 8.0, 4.0, 2.0, 4.0, 2.0, 4.0, 8.0, 6.0, 4.0, 6.0, 2.0, 4.0, 6.0, 2.0, 6.0, 6.0, 4.0,
    2.0, 4.0, 6.0, 2.0, 6.0, 4.0, 2.0, 4.0, 2.0, 10.0,
];

/// The prime factors of `n`, in the order factor.c prints them: the 2s, 3s,
/// 5s and 7s, then the wheel's candidates, and n itself when it is left over
/// and greater than 1.  factor.c prints n and returns on 0, before dividing;
/// on a negative or infinite n its wheel would loop forever, where sqrt is
/// NaN or d > s never breaks, so the port stops too, with no factors.
pub fn factor(mut n: f64) -> Vec<f64> {
    let mut fs = Vec::new();
    if !n.is_finite() || n <= 0.0 {
        return fs;
    }
    let mut quot = 0.0;
    let mut s = sqrt(n) + 1.0;
    for d in [2.0, 3.0, 5.0, 7.0] {
        while modf(n / d, &mut quot) == 0.0 {
            fs.push(d);
            n = quot;
            s = sqrt(n) + 1.0;
        }
    }
    let mut d = 1.0;
    let mut i = 1;
    loop {
        d += WHEEL[i];
        while modf(n / d, &mut quot) == 0.0 {
            fs.push(d);
            n = quot;
            s = sqrt(n) + 1.0;
        }
        i += 1;
        if i >= WHEEL.len() {
            i = 0;
            if d > s {
                break;
            }
        }
    }
    if n > 1.0 {
        fs.push(n);
    }
    fs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prime() {
        assert_eq!(factor(199.0), vec![199.0], "inside the wheel's first pass");
        assert_eq!(factor(211.0), vec![211.0], "past 209, found by the n > 1");
    }

    #[test]
    fn a_square_of_a_prime() {
        assert_eq!(factor(121.0), vec![11.0, 11.0]);
        assert_eq!(factor(44521.0), vec![211.0, 211.0], "211 squared; the wheel's second pass");
    }

    #[test]
    fn powers_before_the_wheel() {
        assert_eq!(factor(1024.0), vec![2.0; 10]);
        assert_eq!(factor(2187.0), vec![3.0; 7]);
        assert_eq!(factor(78125.0), vec![5.0; 7]);
        assert_eq!(factor(117649.0), vec![7.0; 6]);
    }

    #[test]
    fn one_and_zero() {
        assert_eq!(factor(1.0), Vec::new());
        assert_eq!(factor(0.0), Vec::new(), "factor.c returns on 0");
    }

    #[test]
    fn mixed() {
        assert_eq!(factor(858.0), vec![2.0, 3.0, 11.0, 13.0], "2*3*11*13");
        assert_eq!(factor(422.0), vec![2.0, 211.0], "2*211, the 211 after the wheel breaks");
    }

    #[test]
    fn below_two_fifty_three() {
        // The smoke test's scale: the largest product of two primes below 2 to
        // the 53, exact as a double; a wheel that skipped a candidate would
        // leave the product unbroken.
        assert_eq!(factor(9007195909437503.0), vec![94906247.0, 94906249.0]);
    }
}
