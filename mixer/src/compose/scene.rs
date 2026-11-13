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
    pub(crate) camera: crate::abi::EivizSceneCamera,
    pub(crate) labels: Arc<[String]>,
    pub(crate) label_size: f32,
    pub(crate) label_percent: bool,
    pub(crate) label_top: bool,
    /// `gpu_epoch` when the texture was last composed. Static scenes whose layers
    /// have not changed can reuse the texture instead of redrawing every frame.
    pub(crate) cache_epoch: u64,
}
