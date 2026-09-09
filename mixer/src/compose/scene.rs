use std::sync::Arc;

use crate::abi::OverlayDesc;

pub(crate) struct SceneGpu {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) packed: Option<wgpu::Texture>,
    pub(crate) packed_view: Option<wgpu::TextureView>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) layers: Arc<[OverlayDesc]>,
    pub(crate) labels: Arc<[String]>,
    pub(crate) label_size: f32,
    pub(crate) label_percent: bool,
    pub(crate) label_top: bool,
}
