use eiviz_media::Capability;

/// Official DeckLink SDK is dynamically loaded at runtime when present.
pub fn probe() -> Capability {
    Capability {
        id: "decklink".into(),
        available: false,
        detail: "Desktop Video runtime not detected".into(),
    }
}

pub fn schedule_timescale() -> (u32, u32) {
    // duration, timescale for 59.94: 1001 / 60000
    (1001, 60000)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ntsc_schedule_is_rational() {
        assert_eq!(super::schedule_timescale(), (1001, 60000));
    }
}
