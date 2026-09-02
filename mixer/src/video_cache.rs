//! Decoded-file RAM residency. Full-file VRAM residency is never used.

/// Leave this much physical RAM for the mixer and the rest of the machine.
pub const RAM_RESERVE: u64 = 512 * 1024 * 1024;

pub const PRELOAD_OVERFLOW: &str = "preload-ram-overflow";
pub const PRELOAD_FAILED: &str = "preload-ram-failed";

pub enum PreloadError {
    Overflow,
    Failed,
    Stopped,
}

impl PreloadError {
    pub fn token(self) -> &'static str {
        match self {
            Self::Overflow => PRELOAD_OVERFLOW,
            Self::Failed => PRELOAD_FAILED,
            Self::Stopped => "",
        }
    }
}

pub fn estimated_decoded_ram_bytes(
    width: u32,
    height: u32,
    packed: bool,
    duration_hns: i64,
    fps_num: u32,
    fps_den: u32,
) -> Option<u64> {
    if width == 0 || height == 0 || duration_hns <= 0 {
        return None;
    }
    let bpp = if packed { 2u64 } else { 4 };
    let frame = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(bpp);
    let num = u64::from(fps_num.max(1));
    let den = u64::from(fps_den.max(1));
    let frames = (duration_hns as u64).saturating_mul(num) / den.saturating_mul(10_000_000);
    Some(frames.saturating_mul(frame))
}

pub fn ram_overflows(needed: u64, available: Option<u64>) -> bool {
    let Some(available) = available else {
        return false;
    };
    needed.saturating_add(RAM_RESERVE) > available
}

pub fn available_ram_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        return windows_avail_phys();
    }
    #[cfg(target_os = "macos")]
    {
        return macos_avail_phys();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    None
}

pub fn index_at_or_after(pts: &[i64], seek_hns: i64) -> usize {
    pts.partition_point(|&pts| pts < seek_hns)
}

#[cfg(windows)]
fn windows_avail_phys() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    }
    let mut status = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0 && status.avail_phys > 0).then_some(status.avail_phys)
}

#[cfg(target_os = "macos")]
fn macos_avail_phys() -> Option<u64> {
    let page = sysctl_int(c"hw.pagesize")? as u64;
    let free = sysctl_int(c"vm.page_free_count")? as u64;
    let inactive = sysctl_int(c"vm.page_inactive_count").unwrap_or(0) as u64;
    let bytes = page.saturating_mul(free.saturating_add(inactive));
    (bytes > 0).then_some(bytes)
}

#[cfg(target_os = "macos")]
fn sysctl_int(name: &std::ffi::CStr) -> Option<i64> {
    let mut value = 0i64;
    let mut len = std::mem::size_of::<i64>();
    let rc = unsafe {
        sysctlbyname(
            name.as_ptr(),
            (&raw mut value).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && len > 0).then_some(value)
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn sysctlbyname(
        name: *const std::ffi::c_char,
        oldp: *mut std::ffi::c_void,
        oldlenp: *mut usize,
        newp: *mut std::ffi::c_void,
        newlen: usize,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_long_1080p_bgra_is_huge() {
        let hour = 3_600 * 10_000_000;
        let est = estimated_decoded_ram_bytes(1920, 1080, false, hour, 30, 1).unwrap();
        assert!(est > 8 * 1024 * 1024 * 1024);
    }

    #[test]
    fn estimate_allows_short_packed_stinger() {
        let five_sec = 5 * 10_000_000;
        let est = estimated_decoded_ram_bytes(1920, 1080, true, five_sec, 30, 1).unwrap();
        assert!(est < 1024 * 1024 * 1024);
    }

    #[test]
    fn unknown_duration_has_no_estimate() {
        assert_eq!(
            estimated_decoded_ram_bytes(1920, 1080, false, 0, 30, 1),
            None
        );
    }

    #[test]
    fn overflow_uses_available_ram_not_a_fixed_cap() {
        assert!(!ram_overflows(3 * 1024 * 1024 * 1024, Some(8 * 1024 * 1024 * 1024)));
        assert!(ram_overflows(3 * 1024 * 1024 * 1024, Some(3 * 1024 * 1024 * 1024)));
        assert!(!ram_overflows(u64::MAX, None));
    }

    #[test]
    fn index_at_or_after_seek() {
        let pts = [0, 333_667, 667_334];
        assert_eq!(index_at_or_after(&pts, 0), 0);
        assert_eq!(index_at_or_after(&pts, 333_667), 1);
        assert_eq!(index_at_or_after(&pts, 400_000), 2);
        assert_eq!(index_at_or_after(&pts, 1_000_000), 3);
    }
}
