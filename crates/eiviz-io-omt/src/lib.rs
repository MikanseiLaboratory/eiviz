use eiviz_media::Capability;

pub fn probe() -> Capability {
    Capability {
        id: "omt".into(),
        available: false,
        detail: "OMT adapter compiled; native libomt not linked in this build".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_capability() {
        assert_eq!(super::probe().id, "omt");
        assert!(!super::probe().available);
    }
}
