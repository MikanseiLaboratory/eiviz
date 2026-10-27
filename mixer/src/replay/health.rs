//! Replay health with hysteresis. Transitions are the only events.

use super::{
    ISSUE_CPU_ENCODE, ISSUE_FRAMES_DROPPED, ISSUE_GPU_READBACK, ISSUE_PRO_UNAVAILABLE,
    ISSUE_STOPPED_BY_POLICY, ISSUE_STORAGE_FULL, ISSUE_STORAGE_LOW, ISSUE_STORAGE_WRITE,
    SEVERITY_CRITICAL, SEVERITY_OK, SEVERITY_WARNING, STOP_BUFFER_FULL, STOP_IO, STOP_STORAGE_FULL,
};

#[derive(Clone, Debug)]
pub struct HealthSample {
    pub encode_queue_ratio: f32,
    pub encode_ms_avg: f32,
    pub frame_period_ms: f32,
    pub dropped_encode_recent: u32,
    pub write_buffer_ratio: f32,
    pub write_latency_p95_ms: u32,
    pub dropped_write_recent: u32,
    pub dropped_readback_recent: u32,
    pub remaining_secs: u32,
    pub warn_secs: u32,
    pub alert_secs: u32,
    pub critical_secs: u32,
    pub storage_full: bool,
    pub stop_reason: u32,
    pub pro_unavailable: bool,
    /// True only once a store has measured the drive's free space. An unset
    /// remaining time is 0, which would otherwise look like the disk is already
    /// full, so this stays false until there is a figure to compare.
    pub storage_tracked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct HealthMemory {
    encode_hot_since: Option<u64>,
    encode_cool_since: Option<u64>,
    write_hot_since: Option<u64>,
    write_cool_since: Option<u64>,
    pub issues: u32,
    pub severity: u32,
    pub low_tier: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthView {
    pub issues: u32,
    pub severity: u32,
    pub changed: bool,
    pub low_tier: u32,
}

pub fn step(memory: &mut HealthMemory, sample: &HealthSample, now_ms: u64) -> HealthView {
    let mut issues = 0u32;
    let mut severity = SEVERITY_OK;
    let encode_slow =
        sample.frame_period_ms > 0.0 && sample.encode_ms_avg > sample.frame_period_ms * 0.8;
    if sample.encode_queue_ratio >= 0.5 || encode_slow {
        memory.encode_hot_since.get_or_insert(now_ms);
        memory.encode_cool_since = None;
    } else if sample.encode_queue_ratio < 0.25 {
        memory.encode_cool_since.get_or_insert(now_ms);
        memory.encode_hot_since = None;
    } else {
        memory.encode_hot_since = None;
        memory.encode_cool_since = None;
    }
    let encode_hot = memory
        .encode_hot_since
        .is_some_and(|started| now_ms.saturating_sub(started) >= 500)
        || encode_slow;
    let encode_critical = sample.encode_queue_ratio >= 0.9 || sample.dropped_encode_recent > 0;
    let encode_clear = memory
        .encode_cool_since
        .is_some_and(|started| now_ms.saturating_sub(started) >= 2_000)
        && sample.dropped_encode_recent == 0;
    // The latch survives a sample that is merely not-yet-clear, so a warning does not
    // flicker on and off while the encoder is still catching up.
    let encode_hot =
        encode_critical || encode_hot || (memory.issues & ISSUE_CPU_ENCODE != 0 && !encode_clear);
    if encode_hot {
        issues |= ISSUE_CPU_ENCODE;
        severity = severity.max(if encode_critical {
            SEVERITY_CRITICAL
        } else {
            SEVERITY_WARNING
        });
    }

    // A buffered writer normally holds a few megabytes, so the buffer being non-empty
    // says nothing. What says something is a buffer that is nearly full, a write that
    // takes longer than a quarter of a second, or writes being dropped.
    let write_trouble = sample.dropped_write_recent > 0
        || sample.write_latency_p95_ms > 250
        || sample.write_buffer_ratio >= 0.9;
    if write_trouble {
        memory.write_hot_since.get_or_insert(now_ms);
        memory.write_cool_since = None;
    } else {
        memory.write_cool_since.get_or_insert(now_ms);
        memory.write_hot_since = None;
    }
    let write_hot = memory
        .write_hot_since
        .is_some_and(|started| now_ms.saturating_sub(started) >= 1_000)
        || sample.write_latency_p95_ms > 250;
    let write_critical = sample.write_buffer_ratio >= 0.98 || sample.dropped_write_recent > 0;
    if write_critical || write_hot {
        issues |= ISSUE_STORAGE_WRITE;
        severity = severity.max(if write_critical {
            SEVERITY_CRITICAL
        } else {
            SEVERITY_WARNING
        });
    }

    if sample.dropped_readback_recent > 0 {
        issues |= ISSUE_GPU_READBACK;
        severity = severity.max(if sample.dropped_readback_recent >= 30 {
            SEVERITY_CRITICAL
        } else {
            SEVERITY_WARNING
        });
    }
    let mut low_tier = 0u32;
    if sample.storage_tracked && sample.remaining_secs < sample.critical_secs {
        issues |= ISSUE_STORAGE_LOW;
        severity = severity.max(SEVERITY_CRITICAL);
        low_tier = 3;
    } else if sample.storage_tracked && sample.remaining_secs < sample.alert_secs {
        issues |= ISSUE_STORAGE_LOW;
        severity = severity.max(SEVERITY_WARNING);
        low_tier = 2;
    } else if sample.storage_tracked && sample.remaining_secs < sample.warn_secs {
        issues |= ISSUE_STORAGE_LOW;
        severity = severity.max(SEVERITY_WARNING);
        low_tier = 1;
    }
    if sample.storage_tracked && sample.storage_full {
        issues |= ISSUE_STORAGE_FULL;
        severity = severity.max(SEVERITY_CRITICAL);
    }
    let dropped =
        sample.dropped_encode_recent + sample.dropped_write_recent + sample.dropped_readback_recent;
    if dropped > 0 {
        issues |= ISSUE_FRAMES_DROPPED;
        severity = severity.max(SEVERITY_WARNING);
    }
    // The encoder refusing frames is a different problem from the disk refusing them, and the
    // two need different answers: one means too much camera for the machine, the other means a
    // slow drive. Reporting only "frames dropped" left the operator to guess.
    if sample.dropped_encode_recent > 0 {
        issues |= ISSUE_CPU_ENCODE;
        severity = severity.max(if sample.dropped_encode_recent >= 30 {
            SEVERITY_CRITICAL
        } else {
            SEVERITY_WARNING
        });
    }
    if sample.stop_reason == STOP_BUFFER_FULL {
        issues |= ISSUE_STOPPED_BY_POLICY;
        severity = severity.max(SEVERITY_CRITICAL);
    }
    if sample.pro_unavailable {
        issues |= ISSUE_PRO_UNAVAILABLE;
        severity = severity.max(SEVERITY_CRITICAL);
    }
    // A disk that ran out of room stops the recording. Its reported free space is
    // zero, which is also what an unreadable drive reports, so the stop reason is
    // the dependable signal for a full disk.
    if sample.stop_reason == STOP_STORAGE_FULL {
        issues |= ISSUE_STORAGE_FULL;
        severity = severity.max(SEVERITY_CRITICAL);
    }
    if sample.stop_reason == STOP_IO {
        issues |= ISSUE_STORAGE_WRITE;
        severity = severity.max(SEVERITY_CRITICAL);
    }
    let changed =
        issues != memory.issues || severity != memory.severity || low_tier != memory.low_tier;
    memory.issues = issues;
    memory.severity = severity;
    memory.low_tier = low_tier;
    HealthView {
        issues,
        severity,
        changed,
        low_tier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HealthSample {
        HealthSample {
            encode_queue_ratio: 0.0,
            encode_ms_avg: 1.0,
            frame_period_ms: 16.7,
            dropped_encode_recent: 0,
            write_buffer_ratio: 0.0,
            write_latency_p95_ms: 0,
            dropped_write_recent: 0,
            dropped_readback_recent: 0,
            remaining_secs: 10_000,
            warn_secs: 600,
            alert_secs: 300,
            critical_secs: 60,
            storage_full: false,
            stop_reason: 0,
            pro_unavailable: false,
            storage_tracked: true,
        }
    }

    #[test]
    fn encode_warning_needs_half_a_second_and_clears_after_two() {
        let mut memory = HealthMemory::default();
        let mut hot = sample();
        hot.encode_queue_ratio = 0.6;
        assert!(!step(&mut memory, &hot, 0).changed || memory.issues & ISSUE_CPU_ENCODE == 0);
        let mid = step(&mut memory, &hot, 400);
        assert_eq!(mid.issues & ISSUE_CPU_ENCODE, 0);
        let on = step(&mut memory, &hot, 500);
        assert_ne!(on.issues & ISSUE_CPU_ENCODE, 0);
        assert_eq!(on.severity, SEVERITY_WARNING);
        let mut cool = sample();
        cool.encode_queue_ratio = 0.1;
        assert_ne!(step(&mut memory, &cool, 600).issues & ISSUE_CPU_ENCODE, 0);
        let cleared = step(&mut memory, &cool, 2_600);
        assert_eq!(cleared.issues & ISSUE_CPU_ENCODE, 0);
        assert!(cleared.changed);
    }

    #[test]
    fn remaining_time_warns_only_under_the_operators_threshold() {
        let mut memory = HealthMemory::default();
        let mut above = sample();
        above.warn_secs = 90;
        above.alert_secs = 40;
        above.critical_secs = 10;
        above.remaining_secs = 100;
        assert_eq!(step(&mut memory, &above, 0).issues & ISSUE_STORAGE_LOW, 0);
        let mut under = above;
        under.remaining_secs = 80;
        assert_ne!(step(&mut memory, &under, 1).issues & ISSUE_STORAGE_LOW, 0);
        let mut defaults = sample();
        defaults.remaining_secs = 500;
        let mut fresh = HealthMemory::default();
        assert_ne!(step(&mut fresh, &defaults, 0).issues & ISSUE_STORAGE_LOW, 0);
        defaults.remaining_secs = 700;
        fresh = HealthMemory::default();
        assert_eq!(step(&mut fresh, &defaults, 0).issues & ISSUE_STORAGE_LOW, 0);
    }

    #[test]
    fn encode_drops_are_critical_immediately() {
        let mut memory = HealthMemory::default();
        let mut hot = sample();
        hot.dropped_encode_recent = 2;
        let view = step(&mut memory, &hot, 0);
        assert_eq!(view.severity, SEVERITY_CRITICAL);
        assert_ne!(view.issues & ISSUE_CPU_ENCODE, 0);
        assert_ne!(view.issues & ISSUE_FRAMES_DROPPED, 0);
    }

    #[test]
    fn storage_low_notifies_again_when_the_tier_changes() {
        let mut memory = HealthMemory::default();
        let mut low = sample();
        low.remaining_secs = 500;
        let warn = step(&mut memory, &low, 0);
        assert_eq!(warn.low_tier, 1);
        assert!(warn.changed);
        low.remaining_secs = 200;
        let alert = step(&mut memory, &low, 10);
        assert_eq!(alert.low_tier, 2);
        assert!(alert.changed);
        assert_eq!(alert.severity, SEVERITY_WARNING);
        low.remaining_secs = 30;
        let critical = step(&mut memory, &low, 20);
        assert_eq!(critical.severity, SEVERITY_CRITICAL);
    }

    /// A store that has not measured the drive, or cannot read it, reports zero
    /// free bytes. That is not the same as a full disk, and it must not raise an
    /// alarm the moment recording starts.
    #[test]
    fn an_unmeasured_drive_is_not_reported_as_low() {
        let mut memory = HealthMemory::default();
        let untracked = HealthSample {
            remaining_secs: 0,
            storage_tracked: false,
            ..sample()
        };
        let view = step(&mut memory, &untracked, 0);
        assert_eq!(view.issues & ISSUE_STORAGE_LOW, 0);
        assert_eq!(view.severity, SEVERITY_OK);
    }

    /// The stop reason names the failure when the free-space figure cannot.
    #[test]
    fn a_stop_names_why_the_recording_ended() {
        let mut memory = HealthMemory::default();
        let full = HealthSample {
            stop_reason: STOP_STORAGE_FULL,
            ..sample()
        };
        let view = step(&mut memory, &full, 0);
        assert_ne!(view.issues & ISSUE_STORAGE_FULL, 0);
        assert_eq!(view.severity, SEVERITY_CRITICAL);

        let mut memory = HealthMemory::default();
        let io = HealthSample {
            stop_reason: STOP_IO,
            ..sample()
        };
        let view = step(&mut memory, &io, 0);
        assert_ne!(view.issues & ISSUE_STORAGE_WRITE, 0);
        assert_eq!(view.severity, SEVERITY_CRITICAL);

        let mut memory = HealthMemory::default();
        let policy = HealthSample {
            stop_reason: STOP_BUFFER_FULL,
            ..sample()
        };
        let view = step(&mut memory, &policy, 0);
        assert_ne!(view.issues & ISSUE_STOPPED_BY_POLICY, 0);
        assert_eq!(view.severity, SEVERITY_CRITICAL);
    }
}
