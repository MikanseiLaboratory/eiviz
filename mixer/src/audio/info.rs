pub const AUDIO_DIR_RENDER: u32 = 0;
pub const AUDIO_DIR_CAPTURE: u32 = 1;
pub const AUDIO_DIR_BOTH: u32 = 2;
#[allow(dead_code)]
pub const AUDIO_CAP_DEFAULT: u32 = 1;
pub const AUDIO_CAP_LOOPBACK: u32 = 2;
pub const CAPTURE_MODE_MIC: u32 = 0;
pub const CAPTURE_MODE_ENDPOINT_LOOPBACK: u32 = 1;
pub const CAPTURE_MODE_PROCESS_LOOPBACK: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioDeviceInfo {
    pub kind: u32,
    pub channels: u32,
    pub id: [u8; 256],
    pub name: [u8; 256],
    pub direction: u32,
    pub caps: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioBusInfo {
    pub id: u64,
    pub role: u32,
    pub device_kind: u32,
    pub map_left: i32,
    pub map_right: i32,
    pub bit: u32,
    pub name: [u8; 64],
    pub device_id: [u8; 256],
}
