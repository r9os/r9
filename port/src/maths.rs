/// Round down by a power of 2
pub const fn round_down2_usize(n: usize, step: usize) -> usize {
    assert!(step.is_power_of_two());
    n & !(step - 1)
}

/// Round down by a power of 2
pub const fn round_down2_u64(n: u64, step: u64) -> u64 {
    assert!(step.is_power_of_two());
    n & !(step - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_down2() {
        assert_eq!(round_down2_usize(0, 16), 0);
        assert_eq!(round_down2_usize(6, 16), 0);
        assert_eq!(round_down2_usize(16, 16), 16);
        assert_eq!(round_down2_usize(17, 16), 16);
        assert_eq!(round_down2_usize(8193, 4096), 8192);
    }
}
