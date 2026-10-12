//! A loadable library that deliberately does not export `eiviz_pro_get_api`.

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_empty_plugin_present() -> u32 {
    1
}
