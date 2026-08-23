use crate::rational::Rational;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Frames per second as a reduced positive ratio `num / den`.
/// NTSC 59.94 is `60000 / 1001`, never a float.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    num: u32,
    den: u32,
}

pub const NTSC_5994: FrameRate = FrameRate {
    num: 60000,
    den: 1001,
};
pub const RATE_60: FrameRate = FrameRate { num: 60, den: 1 };
pub const RATE_30: FrameRate = FrameRate { num: 30, den: 1 };
pub const RATE_25: FrameRate = FrameRate { num: 25, den: 1 };
pub const RATE_24: FrameRate = FrameRate { num: 24, den: 1 };
pub const PAL_50: FrameRate = FrameRate { num: 50, den: 1 };

impl FrameRate {
    pub fn new(num: u32, den: u32) -> Result<Self, crate::RationalError> {
        if num == 0 || den == 0 {
            return Err(crate::RationalError::ZeroDenominator);
        }
        let g = gcd_u32(num, den);
        Ok(Self {
            num: num / g,
            den: den / g,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.num
    }

    pub const fn denominator(self) -> u32 {
        self.den
    }

    pub fn as_rational(self) -> Rational {
        Rational::new(self.num as i64, self.den as i64).expect("non-zero")
    }

    /// Frame duration as a rational number of seconds (`den / num`).
    pub fn frame_duration(self) -> Rational {
        Rational::new(self.den as i64, self.num as i64).expect("non-zero")
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{} fps", self.num)
        } else {
            write!(f, "{}/{} fps", self.num, self.den)
        }
    }
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_is_exact_ratio() {
        assert_eq!(NTSC_5994.numerator(), 60000);
        assert_eq!(NTSC_5994.denominator(), 1001);
        let r = FrameRate::new(120000, 2002).unwrap();
        assert_eq!(r, NTSC_5994);
    }
}
