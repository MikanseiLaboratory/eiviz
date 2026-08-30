#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioDeviceInfo {
    pub kind: u32,
    pub channels: u32,
    pub id: [u8; 256],
    pub name: [u8; 256],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioBusInfo {
    pub id: u64,
    pub role: u32,
    pub device_kind: u32,
    pub map_left: i32,
    pub map_right: i32,
    pub exclusive: u32,
    pub bit: u32,
    pub name: [u8; 64],
    pub device_id: [u8; 256],
}
