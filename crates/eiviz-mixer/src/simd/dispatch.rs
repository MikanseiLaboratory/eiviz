//! Runtime SIMD path selection. Hardware flags are detected once.

use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdPath {
    Scalar,
    Sse2,
    Avx2,
    Neon,
}

impl SimdPath {
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                return Self::Avx2;
            }
            if is_x86_feature_detected!("sse2") {
                return Self::Sse2;
            }
            Self::Scalar
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::Neon
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::Scalar
        }
    }

    pub fn has_ssse3(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            matches!(self, Self::Avx2) || is_x86_feature_detected!("ssse3")
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = self;
            false
        }
    }
}

impl fmt::Display for SimdPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Scalar => "scalar",
            Self::Sse2 => "sse2",
            Self::Avx2 => "avx2",
            Self::Neon => "neon",
        })
    }
}

pub fn path() -> SimdPath {
    use std::sync::OnceLock;
    static PATH: OnceLock<SimdPath> = OnceLock::new();
    *PATH.get_or_init(SimdPath::detect)
}
