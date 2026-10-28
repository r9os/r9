//! Number parsing, after Plan 9's strtol and atof: both read the longest
//! prefix of the bytes that is a number, and stop at the first byte that
//! isn't.

/// Convert the longest prefix of `nptr` to an `i64` in `base`, as C's
/// strtol: it skips leading whitespace, takes an optional sign, and in base
/// 0 reads a `0x` or `0X` prefix as hex and a lone leading `0` as octal.
/// Return the value, clamped to i64's range on overflow, and where parsing
/// stopped: the first byte that isn't a digit of the number, or the end.
/// No digits, or a base below 1 or above 36 other than 0, returns 0 and
/// stops where it started, so the sign isn't consumed.
pub fn strtol(nptr: &[u8], mut base: i32) -> (i64, usize) {
    let mut p = 0;
    while let Some(c) = nptr.get(p) {
        if !matches!(c, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r' | b'\x0b') {
            break;
        }
        p += 1;
    }
    let mut neg = false;
    match nptr.get(p) {
        Some(&b'-') => {
            neg = true;
            p += 1;
        }
        Some(&b'+') => p += 1,
        _ => {}
    }
    if base == 0 {
        base = 10;
        if nptr.get(p) == Some(&b'0') {
            base = 8;
            if matches!(nptr.get(p + 1), Some(c) if *c == b'x' || *c == b'X') {
                p += 2;
                base = 16;
            }
        }
    } else if base == 16 {
        if nptr.get(p) == Some(&b'0')
            && matches!(nptr.get(p + 1), Some(c) if *c == b'x' || *c == b'X')
        {
            p += 2;
        }
    } else if !(1..=36).contains(&base) {
        return (0, 0);
    }

    let mut n: i64 = 0;
    let mut ndig = 0;
    let mut ovfl = false;
    for c in nptr.iter().skip(p) {
        let v = match c {
            b'0'..=b'9' => (c - b'0') as i64,
            b'a'..=b'z' => (c - b'a') as i64 + 10,
            b'A'..=b'Z' => (c - b'A') as i64 + 10,
            _ => break,
        };
        if v >= base as i64 {
            break;
        }
        // Plan 9's strtol clamps on overflow, so n mustn't wrap: once it
        // can't grow, keep it and remember the overflow.
        if let Some(nn) = n.checked_mul(base as i64).and_then(|t| t.checked_add(v)) {
            n = nn;
        } else {
            ovfl = true;
        }
        ndig += 1;
        p += 1;
    }
    if ndig == 0 {
        return (0, 0);
    }
    let value = if ovfl {
        if neg { i64::MIN } else { i64::MAX }
    } else if neg {
        -n
    } else {
        n
    };
    (value, p)
}

/// Convert the longest prefix of `nptr` to an `f64`, as Plan 9's atof: it
/// skips leading whitespace, takes an optional sign, and reads decimal
/// digits around an optional '.', and an optional exponent after an e or E
/// with its own optional sign.  Return 0 if no digit is found.
pub fn atof(nptr: &[u8]) -> f64 {
    let mut p = 0;
    while let Some(c) = nptr.get(p) {
        if !matches!(c, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r' | b'\x0b') {
            break;
        }
        p += 1;
    }
    let mut neg = false;
    match nptr.get(p) {
        Some(&b'-') => {
            neg = true;
            p += 1;
        }
        Some(&b'+') => p += 1,
        _ => {}
    }
    let mut x: f64 = 0.0;
    let mut ndig = 0;
    while let Some(c) = nptr.get(p) {
        if !c.is_ascii_digit() {
            break;
        }
        x = x * 10.0 + (*c - b'0') as f64;
        ndig += 1;
        p += 1;
    }
    if nptr.get(p) == Some(&b'.') {
        p += 1;
        let mut scale = 1.0;
        while let Some(c) = nptr.get(p) {
            if !c.is_ascii_digit() {
                break;
            }
            scale /= 10.0;
            x += (*c - b'0') as f64 * scale;
            ndig += 1;
            p += 1;
        }
    }
    match nptr.get(p) {
        Some(&b'e') | Some(&b'E') => {
            p += 1;
            let mut esign = 1;
            match nptr.get(p) {
                Some(&b'-') => {
                    esign = -1;
                    p += 1;
                }
                Some(&b'+') => p += 1,
                _ => {}
            }
            let mut e: i32 = 0;
            while let Some(c) = nptr.get(p) {
                if !c.is_ascii_digit() {
                    break;
                }
                // Past i32 the exponent only decides between 0 and infinity.
                e = e.saturating_mul(10).saturating_add((c - b'0') as i32);
                p += 1;
            }
            let e = esign * e;
            // Multiplying or dividing by 10 a few hundred times reaches
            // infinity or 0; the loops are bounded, and stop where x can't
            // change.
            if e > 0 {
                let mut i = 0;
                while i < e && x != 0.0 && !x.is_infinite() {
                    x *= 10.0;
                    i += 1;
                }
            } else if e < 0 {
                let mut i = 0;
                while i < -e && x != 0.0 {
                    x /= 10.0;
                    i += 1;
                }
            }
        }
        _ => {}
    }
    if ndig == 0 {
        0.0
    } else if neg {
        -x
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strtol_decimal() {
        assert_eq!(strtol(b"123", 10), (123, 3));
        assert_eq!(strtol(b"0", 10), (0, 1));
        assert_eq!(strtol(b"-123", 10), (-123, 4));
        assert_eq!(strtol(b"+123", 10), (123, 4));
        assert_eq!(strtol(b" \t123", 10), (123, 5));
    }

    #[test]
    fn strtol_base_zero() {
        assert_eq!(strtol(b"0x1f", 0), (31, 4));
        assert_eq!(strtol(b"0X1F", 0), (31, 4));
        assert_eq!(strtol(b"010", 0), (8, 3));
        assert_eq!(strtol(b"0", 0), (0, 1));
        assert_eq!(strtol(b"010", 10), (10, 3));
    }

    #[test]
    fn strtol_base_16() {
        assert_eq!(strtol(b"0xff", 16), (255, 4));
        assert_eq!(strtol(b"FF", 16), (255, 2));
    }

    #[test]
    fn strtol_stops_at_first_bad_character() {
        assert_eq!(strtol(b"12x3", 10), (12, 2));
        assert_eq!(strtol(b"12 3", 10), (12, 2));
        assert_eq!(strtol(b"0x12G4", 0), (18, 4));
        assert_eq!(strtol(b"12z", 16), (18, 2));
    }

    #[test]
    fn strtol_no_digits() {
        assert_eq!(strtol(b"", 10), (0, 0));
        assert_eq!(strtol(b"  ", 10), (0, 0));
        assert_eq!(strtol(b"-x", 10), (0, 0));
        assert_eq!(strtol(b"+", 10), (0, 0));
        assert_eq!(strtol(b"0x", 0), (0, 0));
        assert_eq!(strtol(b"0x", 16), (0, 0));
    }

    #[test]
    fn strtol_bad_base() {
        assert_eq!(strtol(b"123", 0), (123, 3));
        assert_eq!(strtol(b"123", 1), (0, 0));
        assert_eq!(strtol(b"123", 37), (0, 0));
        assert_eq!(strtol(b"123", -1), (0, 0));
    }

    #[test]
    fn strtol_overflow() {
        assert_eq!(strtol(b"99999999999999999999", 10), (i64::MAX, 20));
        assert_eq!(strtol(b"-99999999999999999999", 10), (i64::MIN, 21));
        assert_eq!(strtol(b"9223372036854775807", 10), (i64::MAX, 19));
        assert_eq!(strtol(b"9223372036854775808", 10), (i64::MAX, 19));
        assert_eq!(strtol(b"-9223372036854775808", 10), (i64::MIN, 20));
    }

    #[test]
    fn atof_decimal() {
        assert_eq!(atof(b"3.25"), 3.25);
        assert_eq!(atof(b".5"), 0.5);
        assert_eq!(atof(b"5."), 5.0);
        assert_eq!(atof(b"0"), 0.0);
        assert_eq!(atof(b"-0.5"), -0.5);
        assert_eq!(atof(b" +1.5"), 1.5);
    }

    #[test]
    fn atof_exponent() {
        assert_eq!(atof(b"1e3"), 1000.0);
        assert_eq!(atof(b"2e+3"), 2000.0);
        assert_eq!(atof(b"-2e-2"), -0.02);
        assert_eq!(atof(b"1e400"), f64::INFINITY);
        assert_eq!(atof(b"1e-400"), 0.0);
        assert_eq!(atof(b"-1e-400"), -0.0);
    }

    #[test]
    fn atof_stops_at_first_bad_character() {
        assert_eq!(atof(b"1.5x"), 1.5);
        assert_eq!(atof(b"12abc"), 12.0);
        assert_eq!(atof(b"1e"), 1.0);
        assert_eq!(atof(b"1e+"), 1.0);
        assert_eq!(atof(b"1.2.3"), 1.2);
    }

    #[test]
    fn atof_no_digits() {
        assert_eq!(atof(b""), 0.0);
        assert_eq!(atof(b" "), 0.0);
        assert_eq!(atof(b"-"), 0.0);
        assert_eq!(atof(b"."), 0.0);
        assert_eq!(atof(b"-x"), 0.0);
    }

    #[test]
    fn atof_exact_below_two_fifty_three() {
        // Factor's inputs are below 2 to the 53, where the parse is exact.
        assert_eq!(atof(b"9007199254740992"), 9007199254740992.0);
        assert_eq!(atof(b"12345678901234"), 12345678901234.0);
        assert_eq!(atof(b"70368744177664"), 70368744177664.0);
    }
}
