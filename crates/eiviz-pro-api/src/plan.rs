/// Commercial plan. OSS builds and the official free download both use [`Plan::Free`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Plan {
    Free,
    Professional,
    Enterprise,
}

impl Plan {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Professional => "professional",
            Self::Enterprise => "enterprise",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "free" => Some(Self::Free),
            "professional" | "pro" => Some(Self::Professional),
            "enterprise" => Some(Self::Enterprise),
            _ => None,
        }
    }

    /// C ABI / protobuf sentinel. `0` Free, `1` Professional, `2` Enterprise.
    pub fn abi(self) -> u32 {
        match self {
            Self::Free => 0,
            Self::Professional => 1,
            Self::Enterprise => 2,
        }
    }
}

/// `Denied` is not in the plan. `Unlimited` has no numeric cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quota<T> {
    Denied,
    Limited(T),
    Unlimited,
}

impl<T: Copy> Quota<T> {
    /// `0` means denied and [`u32::MAX`] means unlimited. Limited values must
    /// not use either sentinel.
    pub fn abi_count(self) -> u32
    where
        T: Into<u32>,
    {
        match self {
            Self::Denied => 0,
            Self::Unlimited => u32::MAX,
            Self::Limited(value) => value.into(),
        }
    }
}

/// Maximum picture size and frame rate, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoLimit {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

impl VideoLimit {
    pub const fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        Self {
            width,
            height,
            fps_num,
            fps_den,
        }
    }

    pub fn allows(self, width: u32, height: u32, fps_num: u32, fps_den: u32) -> bool {
        if width > self.width || height > self.height || fps_den == 0 || self.fps_den == 0 {
            return false;
        }
        u64::from(fps_num) * u64::from(self.fps_den) <= u64::from(self.fps_num) * u64::from(fps_den)
    }
}

impl Quota<VideoLimit> {
    pub fn allows_video(self, width: u32, height: u32, fps_num: u32, fps_den: u32) -> bool {
        match self {
            Self::Denied => false,
            Self::Unlimited => width > 0 && height > 0 && fps_num > 0 && fps_den > 0,
            Self::Limited(limit) => limit.allows(width, height, fps_num, fps_den),
        }
    }
}

/// What the current plan permits. The mixer enforces this; the Pro module only reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entitlements {
    pub plan: Plan,
    pub mixing_units: Quota<u32>,
    pub decklink_inputs: Quota<u32>,
    pub decklink_outputs: Quota<u32>,
    pub rtmp: Quota<VideoLimit>,
    pub recording: bool,
    pub srt: bool,
    pub hardware_encode: bool,
}

impl Entitlements {
    pub fn free() -> Self {
        Self {
            plan: Plan::Free,
            mixing_units: Quota::Limited(4),
            decklink_inputs: Quota::Denied,
            decklink_outputs: Quota::Denied,
            rtmp: Quota::Limited(VideoLimit::new(1280, 720, 30, 1)),
            recording: false,
            srt: false,
            hardware_encode: false,
        }
    }

    pub fn professional() -> Self {
        Self {
            plan: Plan::Professional,
            mixing_units: Quota::Unlimited,
            decklink_inputs: Quota::Limited(4),
            decklink_outputs: Quota::Limited(1),
            rtmp: Quota::Limited(VideoLimit::new(1920, 1080, 60_000, 1_001)),
            recording: true,
            srt: true,
            hardware_encode: true,
        }
    }

    pub fn enterprise() -> Self {
        Self {
            plan: Plan::Enterprise,
            mixing_units: Quota::Unlimited,
            decklink_inputs: Quota::Unlimited,
            decklink_outputs: Quota::Unlimited,
            rtmp: Quota::Unlimited,
            recording: true,
            srt: true,
            hardware_encode: true,
        }
    }

    pub fn for_plan(plan: Plan) -> Self {
        match plan {
            Plan::Free => Self::free(),
            Plan::Professional => Self::professional(),
            Plan::Enterprise => Self::enterprise(),
        }
    }

    /// `replacing` is true when an existing slot of the same kind is updated in place.
    pub fn admits_count(quota: Quota<u32>, existing: usize, replacing: bool) -> bool {
        if replacing {
            return !matches!(quota, Quota::Denied);
        }
        match quota {
            Quota::Denied => false,
            Quota::Unlimited => true,
            Quota::Limited(limit) => existing < limit as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_rtmp_is_720p30_inclusive() {
        let rtmp = Entitlements::free().rtmp;
        assert!(rtmp.allows_video(1280, 720, 30, 1));
        assert!(rtmp.allows_video(1280, 720, 30_000, 1_001));
        assert!(!rtmp.allows_video(1920, 1080, 30, 1));
        assert!(!rtmp.allows_video(1280, 720, 60, 1));
    }

    #[test]
    fn professional_rtmp_allows_59_94_not_60() {
        let rtmp = Entitlements::professional().rtmp;
        assert!(rtmp.allows_video(1920, 1080, 60_000, 1_001));
        assert!(!rtmp.allows_video(1920, 1080, 60, 1));
    }

    #[test]
    fn count_admission_ignores_in_place_replacement() {
        assert!(Entitlements::admits_count(Quota::Limited(1), 1, true));
        assert!(!Entitlements::admits_count(Quota::Limited(1), 1, false));
        assert!(Entitlements::admits_count(Quota::Limited(4), 3, false));
        assert!(!Entitlements::admits_count(Quota::Denied, 0, true));
    }
}
