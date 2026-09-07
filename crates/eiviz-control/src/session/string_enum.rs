//! Session / Remote JSON string enums.
//!
//! Session files write PascalCase (`Follow`). Windows Remote mutations use
//! `JsonStringEnumConverter(CamelCase)` (`follow`). Accept both by matching
//! variant names without ASCII case.

macro_rules! session_string_enum {
    (
        $(#[$enum_attr:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_attr:meta])*
                $variant:ident
            ),+ $(,)?
        }
    ) => {
        $(#[$enum_attr])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $vis enum $name {
            $(
                $(#[$variant_attr])*
                $variant,
            )+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant),)+
                }
            }

            pub fn from_str_loose(raw: &str) -> Option<Self> {
                $(
                    if raw.eq_ignore_ascii_case(stringify!($variant)) {
                        return Some(Self::$variant);
                    }
                )+
                None
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
                Self::from_str_loose(&raw).ok_or_else(|| {
                    serde::de::Error::unknown_variant(&raw, &[$(stringify!($variant)),+])
                })
            }
        }
    };
}

pub(crate) use session_string_enum;

#[cfg(test)]
mod tests {
    use crate::session::{
        AudioDeviceKind, AudioLinkMode, BandwidthSave, InputKind, MixSource, MultiviewTemplate,
        MvSlotKind, OutputSourceKind, OutputTransport, Renderer, SwitcherSceneFilter,
        VideoPlayWhen,
    };

    fn assert_pair<T>(pascal: &str, camel: &str, expected: T)
    where
        T: serde::de::DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug,
    {
        let parsed: T = serde_json::from_value(serde_json::Value::String(camel.into()))
            .unwrap_or_else(|error| panic!("{camel}: {error}"));
        assert_eq!(parsed, expected);
        let again: T = serde_json::from_value(serde_json::Value::String(pascal.into()))
            .unwrap_or_else(|error| panic!("{pascal}: {error}"));
        assert_eq!(again, expected);
        let written = serde_json::to_value(&expected).expect("serialize");
        assert_eq!(written, serde_json::Value::String(pascal.into()));
    }

    #[test]
    fn remote_camel_case_and_session_pascal_case_roundtrip() {
        assert_pair("Follow", "follow", AudioLinkMode::Follow);
        assert_pair("Independent", "independent", AudioLinkMode::Independent);
        assert_pair("All", "all", SwitcherSceneFilter::All);
        assert_pair("Include", "include", SwitcherSceneFilter::Include);
        assert_pair("Exclude", "exclude", SwitcherSceneFilter::Exclude);
        assert_pair("MuPreview", "muPreview", MixSource::MuPreview);
        assert_pair("MuProgram", "muProgram", MixSource::MuProgram);
        assert_pair(
            "SessionMultiview",
            "sessionMultiview",
            MixSource::SessionMultiview,
        );
        assert_pair("MuPreview", "muPreview", OutputSourceKind::MuPreview);
        assert_pair("MuProgram", "muProgram", OutputSourceKind::MuProgram);
        assert_pair("Multiview", "multiview", OutputSourceKind::Multiview);
        assert_pair("Omt", "omt", OutputTransport::Omt);
        assert_pair("Ndi", "ndi", OutputTransport::Ndi);
        assert_pair("DeckLink", "deckLink", OutputTransport::DeckLink);
        assert_pair("Wasapi", "wasapi", AudioDeviceKind::Wasapi);
        assert_pair("CoreAudio", "coreAudio", AudioDeviceKind::CoreAudio);
        assert_pair("Auto", "auto", Renderer::Auto);
        assert_pair("Dx12", "dx12", Renderer::Dx12);
        assert_pair("Vulkan", "vulkan", Renderer::Vulkan);
        assert_pair("Metal", "metal", Renderer::Metal);
        assert_pair("OnActive", "onActive", VideoPlayWhen::OnActive);
        assert_pair(
            "NotOnPreviewOrProgram",
            "notOnPreviewOrProgram",
            BandwidthSave::NotOnPreviewOrProgram,
        );
        assert_pair("MuPreview", "muPreview", MvSlotKind::MuPreview);
        assert_pair(
            "PreviewProgram8",
            "previewProgram8",
            MultiviewTemplate::PreviewProgram8,
        );
        assert_pair("Grid2x2", "grid2x2", MultiviewTemplate::Grid2x2);
        assert_pair("OMT", "omt", InputKind::OMT);
        assert_pair("NDI", "ndi", InputKind::NDI);
        assert_pair("UVC", "uvc", InputKind::UVC);
        assert_pair("Bars", "bars", InputKind::Bars);
    }

    #[test]
    fn unknown_audio_link_is_rejected() {
        let err = serde_json::from_str::<AudioLinkMode>("\"following\"").unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("following")
                && message.contains("Follow")
                && message.contains("Independent"),
            "{message}"
        );
    }

    #[test]
    fn output_transport_accepts_legacy_uppercase_aliases() {
        let omt: OutputTransport = serde_json::from_str("\"OMT\"").unwrap();
        let ndi: OutputTransport = serde_json::from_str("\"NDI\"").unwrap();
        assert_eq!(omt, OutputTransport::Omt);
        assert_eq!(ndi, OutputTransport::Ndi);
    }
}
