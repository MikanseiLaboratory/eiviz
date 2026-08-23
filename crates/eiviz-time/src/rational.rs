use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum RationalError {
    #[error("denominator must not be zero")]
    ZeroDenominator,
}

/// Reduced signed rational number `num / den` with `den > 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    num: i64,
    den: u64,
}

impl Rational {
    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub const ONE: Self = Self { num: 1, den: 1 };

    pub fn new(num: i64, den: i64) -> Result<Self, RationalError> {
        if den == 0 {
            return Err(RationalError::ZeroDenominator);
        }
        Ok(Self::reduced(num, den))
    }

    pub const fn from_integer(value: i64) -> Self {
        Self { num: value, den: 1 }
    }

    fn reduced(num: i64, den: i64) -> Self {
        let neg = (num < 0) ^ (den < 0);
        let n = num.unsigned_abs();
        let d = den.unsigned_abs();
        let g = gcd_u64(n, d);
        let mut num = (n / g) as i64;
        if neg {
            num = -num;
        }
        Self { num, den: d / g }
    }

    pub const fn numerator(self) -> i64 {
        self.num
    }

    pub const fn denominator(self) -> u64 {
        self.den
    }

    pub fn recip(self) -> Result<Self, RationalError> {
        if self.num == 0 {
            return Err(RationalError::ZeroDenominator);
        }
        Self::new(self.den as i64, self.num)
    }

    pub fn wrapping_add(self, other: Self) -> Self {
        let den = lcm_u64(self.den, other.den);
        let a = (den / self.den) as i128 * self.num as i128;
        let b = (den / other.den) as i128 * other.num as i128;
        let num = (a + b) as i64;
        Self::reduced(num, den as i64)
    }

    pub fn saturating_mul(self, other: Self) -> Self {
        let num = (self.num as i128) * (other.num as i128);
        let den = (self.den as i128) * (other.den as i128);
        let num = num.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        let den = den.clamp(1, i64::MAX as i128) as i64;
        Self::reduced(num, den)
    }

    pub fn cmp_ratio(self, other: Self) -> std::cmp::Ordering {
        let left = self.num as i128 * other.den as i128;
        let right = other.num as i128 * self.den as i128;
        left.cmp(&right)
    }

    /// Exact `floor(self * factor)` for non-negative factor.
    pub fn mul_floor_u64(self, factor: u64) -> Result<u64, RationalError> {
        if self.num < 0 {
            return Ok(0);
        }
        let n = (self.num as u128)
            .checked_mul(factor as u128)
            .ok_or(RationalError::ZeroDenominator)?;
        Ok((n / self.den as u128) as u64)
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp_ratio(*other)
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

fn lcm_u64(a: u64, b: u64) -> u64 {
    a / gcd_u64(a, b) * b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_and_compares() {
        let a = Rational::new(4, 8).unwrap();
        let b = Rational::new(1, 2).unwrap();
        assert_eq!(a, b);
        assert!(Rational::new(60000, 1001).unwrap() > Rational::from_integer(59));
        assert!(Rational::new(60000, 1001).unwrap() < Rational::from_integer(60));
    }
}
