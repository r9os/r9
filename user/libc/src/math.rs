//! The floating point the commands need, after Plan 9's sqrt and modf.
//!
//! Plan 9 uses the AArch64 instructions fsqrt and frintz.  On the host, where
//! the tests run, the same instructions come from core's NEON and SSE2
//! intrinsics; frintz has no stable intrinsic, so modf clears the mantissa
//! bits below the integer part, which rounds toward zero as frintz does.

/// The square root of `x`, as Plan 9's sqrt: the hardware instruction.
/// A negative x is NaN, as in C.
pub fn sqrt(x: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        use core::arch::aarch64;
        // SAFETY: vdupq_n_f64, vsqrtq_f64 and vgetq_lane_f64 take and return
        // well-formed doubles; fsqrt is defined for every input, NaN and
        // infinities included.
        unsafe {
            let v = aarch64::vdupq_n_f64(x);
            aarch64::vgetq_lane_f64(aarch64::vsqrtq_f64(v), 0)
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64;
        // SSE2 is part of x86-64.
        // SAFETY: _mm_set1_pd, _mm_sqrt_sd and _mm_cvtsd_f64 take and return
        // well-formed vectors; sqrtsd is defined for every input, NaN and
        // infinities included.
        unsafe {
            let a = x86_64::_mm_set1_pd(x);
            x86_64::_mm_cvtsd_f64(x86_64::_mm_sqrt_sd(a, a))
        }
    }
}

/// `x`'s integer part, as the frintz truncation: round toward zero.
fn trunc(x: f64) -> f64 {
    let b = x.to_bits();
    let e = (b >> 52) & 0x7ff;
    if e == 0 || e < 1023 {
        // Subnormal, or a normal below 1: every bit is below the point, so
        // the integer part is ±0.
        return f64::from_bits(b & 0x8000_0000_0000_0000);
    }
    if e >= 1023 + 52 {
        // 2 to the 53 or more, or infinite or NaN: the lowest mantissa bit
        // is at position 0 or above, so there's no fraction to clear.
        return x;
    }
    // |x| is between 1 and 2 to the 53: the lowest 52 - e + 1023 mantissa
    // bits are below the point; clear them.
    let drop = 1075 - e;
    f64::from_bits(b & !((1u64 << drop) - 1))
}

/// Split `x` into its fraction and integer part, as C's modf: store the
/// integer part in `intpart` and return the fraction.  Both keep x's sign.
pub fn modf(x: f64, intpart: &mut f64) -> f64 {
    let i = trunc(x);
    *intpart = i;
    x - i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqrt_matches_std() {
        for x in [0.0, 1.0, 2.0, 3.25, 1e30, 1e-30, f64::MAX, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(sqrt(x).to_bits(), f64::sqrt(x).to_bits(), "{x}");
        }
        assert!(sqrt(f64::NAN).is_nan());
        assert!(sqrt(-4.0).is_nan());
    }

    #[test]
    fn trunc_rounds_toward_zero() {
        assert_eq!(trunc(0.0), 0.0);
        assert_eq!(trunc(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(trunc(1.5), 1.0);
        assert_eq!(trunc(-1.5), -1.0);
        assert_eq!(trunc(0.5), 0.0);
        assert_eq!(trunc(-0.5).to_bits(), (-0.0f64).to_bits());
        assert_eq!(trunc(3.0), 3.0);
        assert_eq!(trunc(-3.0), -3.0);
        assert_eq!(trunc(1e20), 1e20);
        assert_eq!(trunc(f64::INFINITY), f64::INFINITY);
        assert!(trunc(f64::NAN).is_nan());
        // The boundary where a fraction still exists: below 2 to the 53 the
        // ulp is 0.5, at 2 to the 53 it's 1.
        assert_eq!(trunc(4503599627370496.5), 4503599627370496.0);
        assert_eq!(trunc(-4503599627370496.5), -4503599627370496.0);
        assert_eq!(trunc(9007199254740992.0), 9007199254740992.0);
    }

    #[test]
    fn modf_splits() {
        let mut i;
        i = 0.0;
        assert_eq!(modf(1.5, &mut i), 0.5);
        assert_eq!(i, 1.0);
        i = 0.0;
        assert_eq!(modf(-1.5, &mut i), -0.5);
        assert_eq!(i, -1.0);
        i = 0.0;
        assert_eq!(modf(0.5, &mut i), 0.5);
        assert_eq!(i, 0.0);
        i = 0.0;
        assert_eq!(modf(3.0, &mut i), 0.0);
        assert_eq!(i, 3.0);
        i = 0.0;
        assert_eq!(modf(1e20 + 5.0, &mut i), 0.0);
        assert_eq!(i, 1e20);
    }
}
