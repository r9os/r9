//! sleep's logic, from Plan 9's sleep.c, apart from its system calls, so that
//! it can be tested on the host.

#![cfg_attr(not(test), no_std)]

use libc::strtol;

/// Parse `arg` as sleep.c's first argument: the whole seconds, from strtol
/// with base 0, and the fraction in milliseconds, which is read without
/// floating point because it is useful to be able to run sleep when
/// bootstrapping a machine.  After the '.' one digit is a tenth of a second,
/// two are a hundredth, and more than three are cut to three.  The fraction
/// is None when the argument has none to sleep: no '.', or a fraction that
/// reads as 0 or less.
pub fn parse(arg: &[u8]) -> (i64, Option<i64>) {
    let (n, p) = strtol(arg, 0);
    let f = match arg.get(p) {
        Some(&b'.') => {
            let (f, q) = strtol(&arg[p + 1..], 10);
            if f > 0 {
                Some(match q {
                    // sleep.c's case 0, unreachable, as f > 0 needs a digit.
                    0 => f,
                    1 => f * 100,
                    2 => f * 10,
                    _ => strtol(&arg[p + 1..p + 4], 10).0,
                })
            } else {
                None
            }
        }
        _ => None,
    };
    (n, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_seconds() {
        assert_eq!(parse(b"1"), (1, None));
        assert_eq!(parse(b"123"), (123, None));
        assert_eq!(parse(b"0"), (0, None));
        assert_eq!(parse(b"-5"), (-5, None), "negative");
        assert_eq!(parse(b"0x10"), (16, None), "hex, base 0");
        assert_eq!(parse(b"010"), (8, None), "octal, base 0");
        assert_eq!(parse(b" 5"), (5, None), "leading whitespace");
        assert_eq!(parse(b"1 x"), (1, None), "stops at the first bad byte");
        assert_eq!(parse(b"0x1.5"), (1, Some(500)), "hex whole, fraction after");
        assert_eq!(parse(b""), (0, None), "no digits");
        assert_eq!(parse(b"x"), (0, None), "no digits");
        assert_eq!(parse(b"99999999999999999999"), (i64::MAX, None), "clamped");
    }

    #[test]
    fn fraction() {
        // One digit is a tenth of a second.
        assert_eq!(parse(b"1.5"), (1, Some(500)));
        assert_eq!(parse(b".5"), (0, Some(500)), "no whole part");
        assert_eq!(parse(b"0.5"), (0, Some(500)));
        // Two are a hundredth.
        assert_eq!(parse(b"1.25"), (1, Some(250)));
        assert_eq!(parse(b".25"), (0, Some(250)));
        // Three are milliseconds, and more than three are cut to three.
        assert_eq!(parse(b"1.234"), (1, Some(234)));
        assert_eq!(parse(b"1.2345"), (1, Some(234)));
        assert_eq!(parse(b".999999"), (0, Some(999)));
        assert_eq!(parse(b"1. 5"), (1, Some(50)), "the count includes the space");
    }

    #[test]
    fn fraction_not_slept() {
        assert_eq!(parse(b"1.0"), (1, None), "reads as 0");
        assert_eq!(parse(b"1."), (1, None), "no digits");
        assert_eq!(parse(b"1.x"), (1, None), "no digits");
        assert_eq!(parse(b"1.-5"), (1, None), "reads as less than 0");
    }

    #[test]
    fn cut_to_three() {
        // sleep.c truncates to the first three digits, and still sleeps the
        // result, so this is 0 milliseconds, not nothing.
        assert_eq!(parse(b"1.000999"), (1, Some(0)), "leading zeros");
    }
}
