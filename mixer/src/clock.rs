use std::time::{Duration, Instant};

/// OMT / NDI media timestamps use 100 ns ticks.
pub const TICKS_PER_SEC: i64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rate {
    pub num: u32,
    pub den: u32,
}

impl Rate {
    pub fn new(num: u32, den: u32) -> Result<Self, &'static str> {
        if num == 0 || den == 0 {
            return Err("frame rate numerator and denominator must be non-zero");
        }
        if num > 240_000 || den > 100_000 {
            return Err("frame rate is outside the supported range");
        }
        let rate = Self { num, den }.reduced();
        if rate.interval_ticks() <= 0 {
            return Err("frame period is zero");
        }
        Ok(rate)
    }

    pub fn or_default(num: u32, den: u32) -> Self {
        Self::new(num, den).unwrap_or(Self {
            num: 60_000,
            den: 1_001,
        })
    }

    pub fn reduced(self) -> Self {
        let g = gcd(self.num, self.den);
        Self {
            num: self.num / g,
            den: self.den / g,
        }
    }

    pub fn interval_ticks(self) -> i64 {
        (i64::from(self.den)).saturating_mul(TICKS_PER_SEC) / i64::from(self.num.max(1))
    }

    pub fn pts(self, idx: u64) -> i64 {
        let num = i128::from(self.num.max(1));
        let den = i128::from(self.den);
        ((i128::from(idx) * i128::from(TICKS_PER_SEC) * den) / num) as i64
    }

    pub fn deadline(self, epoch: Instant, idx: u64) -> Instant {
        epoch + duration_from_ticks(self.pts(idx))
    }

    pub fn frames_elapsed(self, elapsed: Duration) -> u64 {
        let ticks = (elapsed.as_nanos() / 100) as i64;
        if ticks <= 0 {
            return 0;
        }
        let num = i128::from(self.num);
        let den_ticks = i128::from(self.den.max(1)) * i128::from(TICKS_PER_SEC);
        let mut idx = ((i128::from(ticks) * num) / den_ticks) as u64;
        while self.pts(idx.saturating_add(1)) <= ticks {
            idx = idx.saturating_add(1);
        }
        idx
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den.max(1))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SharedMediaClock {
    epoch: Instant,
}

impl SharedMediaClock {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    pub fn at(epoch: Instant) -> Self {
        Self { epoch }
    }

    pub fn epoch(self) -> Instant {
        self.epoch
    }

    pub fn deadline(self, rate: Rate, idx: u64) -> Instant {
        rate.deadline(self.epoch, idx)
    }

    pub fn frames_elapsed(self, rate: Rate, now: Instant) -> u64 {
        rate.frames_elapsed(now.saturating_duration_since(self.epoch))
    }

    pub fn pts_for_index(self, rate: Rate, idx: u64) -> i64 {
        rate.pts(idx)
    }

    pub fn now_pts(self, now: Instant) -> i64 {
        let nanos = now.saturating_duration_since(self.epoch).as_nanos();
        (nanos / 100) as i64
    }

    pub fn audio_deadline(self, samples: u64, sample_rate: u32) -> Instant {
        self.epoch + audio_duration(samples, sample_rate)
    }

    pub fn audio_pts(self, samples: u64, sample_rate: u32) -> i64 {
        audio_pts(samples, sample_rate)
    }
}

impl Default for SharedMediaClock {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlayoutCursor {
    pub rate: Rate,
    pub idx: u64,
}

impl PlayoutCursor {
    pub fn new(rate: Rate) -> Self {
        Self { rate, idx: 0 }
    }

    pub fn set_rate(&mut self, clock: SharedMediaClock, now: Instant, rate: Rate) {
        if self.rate == rate {
            return;
        }
        let last_ts = if self.idx == 0 {
            -1
        } else {
            self.rate.pts(self.idx.saturating_sub(1))
        };
        self.rate = rate;
        let wall = clock.frames_elapsed(rate, now);
        if last_ts < 0 {
            self.idx = wall;
            return;
        }
        let min_idx = (last_ts / rate.interval_ticks().max(1)) as u64 + 1;
        self.idx = wall.max(min_idx);
    }

    /// Returns the playout PTS when a slot is due. Late slots are skipped;
    /// the cursor never bursts catch-up sends.
    pub fn due(&mut self, clock: SharedMediaClock, now: Instant) -> Option<i64> {
        self.due_with_skip(clock, now).map(|(pts, _)| pts)
    }

    /// Same as [`Self::due`], plus the number of skipped late slots.
    pub fn due_with_skip(&mut self, clock: SharedMediaClock, now: Instant) -> Option<(i64, u64)> {
        let ideal = clock.frames_elapsed(self.rate, now);
        let skipped = ideal.saturating_sub(self.idx);
        if ideal > self.idx {
            self.idx = ideal;
        }
        if now < clock.deadline(self.rate, self.idx) {
            return None;
        }
        let pts = self.rate.pts(self.idx);
        self.idx = self.idx.saturating_add(1);
        Some((pts, skipped))
    }

    pub fn next_deadline(&self, clock: SharedMediaClock) -> Instant {
        clock.deadline(self.rate, self.idx)
    }
}

pub fn duration_from_ticks(ticks: i64) -> Duration {
    Duration::from_nanos((ticks.max(0) as u64).saturating_mul(100))
}

pub fn audio_pts(samples: u64, sample_rate: u32) -> i64 {
    let rate = i64::from(sample_rate.max(1));
    (samples as i64).saturating_mul(TICKS_PER_SEC) / rate
}

pub fn audio_duration(samples: u64, sample_rate: u32) -> Duration {
    duration_from_ticks(audio_pts(samples, sample_rate))
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(num: u32, den: u32) -> i64 {
        Rate::new(num, den).unwrap().interval_ticks()
    }

    #[test]
    fn omt_tick_interval_matches_common_rates() {
        assert_eq!(interval(30, 1), 333_333);
        assert_eq!(interval(60, 1), 166_666);
        assert_eq!(interval(30_000, 1_001), 333_666);
        assert_eq!(interval(60_000, 1_001), 166_833);
        assert_eq!(interval(50, 1), 200_000);
        assert_eq!(interval(25, 1), 400_000);
        assert_eq!(interval(24, 1), 416_666);
        assert_eq!(interval(24_000, 1_001), 417_083);
        assert_eq!(interval(120, 1), 83_333);
        assert_eq!(interval(120_000, 1_001), 83_416);
    }

    #[test]
    fn rejects_zero_and_empty_period() {
        assert!(Rate::new(0, 1).is_err());
        assert!(Rate::new(1, 0).is_err());
        assert!(Rate::new(u32::MAX, 1).is_err());
    }

    #[test]
    fn frames_elapsed_tracks_omt_tick_timeline() {
        let rate = Rate::new(30, 1).unwrap();
        let period = duration_from_ticks(rate.pts(1));
        assert_eq!(rate.frames_elapsed(Duration::ZERO), 0);
        assert_eq!(rate.frames_elapsed(period), 1);
        assert_eq!(rate.frames_elapsed(duration_from_ticks(rate.pts(10))), 10);
        assert_eq!(
            rate.frames_elapsed(duration_from_ticks(rate.pts(10)) - Duration::from_nanos(100)),
            9
        );
    }

    #[test]
    fn hour_slot_counts_stay_within_one_frame() {
        let hour = Duration::from_secs(3600);
        let cases = [
            (24_000, 1_001, 86_313),
            (24, 1, 86_400),
            (25, 1, 90_000),
            (30_000, 1_001, 107_892),
            (30, 1, 108_000),
            (50, 1, 180_000),
            (60_000, 1_001, 215_784),
            (60, 1, 216_000),
            (120_000, 1_001, 431_568),
            (120, 1, 432_000),
        ];
        for (num, den, expect) in cases {
            let rate = Rate::new(num, den).unwrap();
            let got = rate.frames_elapsed(hour);
            assert!(
                got.abs_diff(expect) <= 1,
                "{num}/{den} hour slots {got} vs {expect}"
            );
        }
    }

    #[test]
    fn pts_is_monotonic_and_aligned() {
        for (num, den) in [
            (24_000, 1_001),
            (24, 1),
            (25, 1),
            (30_000, 1_001),
            (30, 1),
            (50, 1),
            (60_000, 1_001),
            (60, 1),
            (120_000, 1_001),
            (120, 1),
        ] {
            let rate = Rate::new(num, den).unwrap();
            let mut last = -1i64;
            for idx in 0..1_000 {
                let pts = rate.pts(idx);
                assert!(pts > last, "{num}/{den} idx={idx}");
                last = pts;
            }
        }
    }

    #[test]
    fn playout_does_not_burst_after_hitch() {
        let epoch = Instant::now();
        let clock = SharedMediaClock::at(epoch);
        let rate = Rate::new(50, 1).unwrap();
        let mut cursor = PlayoutCursor::new(rate);
        assert!(cursor.due(clock, epoch).is_some());
        assert!(
            cursor
                .due(clock, epoch + Duration::from_millis(1))
                .is_none()
        );
        let late = epoch + Duration::from_millis(80);
        let (_, skipped) = cursor.due_with_skip(clock, late).expect("late slot");
        assert!(skipped >= 2, "hitch must skip late compose slots");
        assert!(cursor.due(clock, late + Duration::from_millis(1)).is_none());
    }

    #[test]
    fn fifty_from_59_94_keeps_fifty_slots() {
        let epoch = Instant::now();
        let clock = SharedMediaClock::at(epoch);
        let output = Rate::new(50, 1).unwrap();
        let mut cursor = PlayoutCursor::new(output);
        let mut sent = 0u64;
        let end = epoch + Duration::from_secs(1);
        loop {
            let deadline = cursor.next_deadline(clock);
            if deadline >= end {
                break;
            }
            assert!(cursor.due(clock, deadline).is_some());
            sent += 1;
        }
        assert_eq!(sent, 50);
    }

    #[test]
    fn twenty_five_from_59_94_keeps_twenty_five_slots() {
        let epoch = Instant::now();
        let clock = SharedMediaClock::at(epoch);
        let output = Rate::new(25, 1).unwrap();
        let mut cursor = PlayoutCursor::new(output);
        let mut sent = 0u64;
        let end = epoch + Duration::from_secs(1);
        loop {
            let deadline = cursor.next_deadline(clock);
            if deadline >= end {
                break;
            }
            assert!(cursor.due(clock, deadline).is_some());
            sent += 1;
        }
        assert_eq!(sent, 25);
    }

    #[test]
    fn repeat_when_output_faster_than_source() {
        let epoch = Instant::now();
        let clock = SharedMediaClock::at(epoch);
        let source = Rate::new(30_000, 1_001).unwrap();
        let output = Rate::new(60_000, 1_001).unwrap();
        let mut cursor = PlayoutCursor::new(output);
        let mut sent = 0u64;
        let mut unique = 0u64;
        let mut last_src = None;
        let end = epoch + Duration::from_secs(1);
        loop {
            let deadline = cursor.next_deadline(clock);
            if deadline >= end {
                break;
            }
            assert!(cursor.due(clock, deadline).is_some());
            sent += 1;
            let src_idx = source.frames_elapsed(deadline.saturating_duration_since(epoch));
            if last_src != Some(src_idx) {
                unique += 1;
                last_src = Some(src_idx);
            }
        }
        assert!((59..=60).contains(&sent), "sent={sent}");
        assert!(
            unique < sent && (29..=30).contains(&unique),
            "unique={unique} sent={sent}"
        );
    }

    #[test]
    fn ten_minute_slot_counts_stay_within_one_frame() {
        let ten_min = Duration::from_secs(600);
        let cases = [
            (24_000, 1_001, 14_385),
            (24, 1, 14_400),
            (25, 1, 15_000),
            (30_000, 1_001, 17_982),
            (30, 1, 18_000),
            (50, 1, 30_000),
            (60_000, 1_001, 35_964),
            (60, 1, 36_000),
            (120_000, 1_001, 71_928),
            (120, 1, 72_000),
        ];
        for (num, den, expect) in cases {
            let rate = Rate::new(num, den).unwrap();
            let got = rate.frames_elapsed(ten_min);
            assert!(
                got.abs_diff(expect) <= 1,
                "{num}/{den} 10m slots {got} vs {expect}"
            );
        }
    }

    #[test]
    fn fifty_nine_ninety_four_to_fifty_drops_without_burst() {
        let epoch = Instant::now();
        let clock = SharedMediaClock::at(epoch);
        let output = Rate::new(50, 1).unwrap();
        let mut cursor = PlayoutCursor::new(output);
        let mut last_deadline = epoch;
        let end = epoch + Duration::from_secs(2);
        let mut sent = 0u64;
        loop {
            let deadline = cursor.next_deadline(clock);
            if deadline >= end {
                break;
            }
            assert!(deadline >= last_deadline);
            last_deadline = deadline;
            assert!(cursor.due(clock, deadline).is_some());
            sent += 1;
        }
        assert_eq!(sent, 100);
    }

    #[test]
    fn audio_and_video_share_the_same_epoch() {
        let epoch = Instant::now();
        let clock = SharedMediaClock::at(epoch);
        let video = Rate::new(50, 1).unwrap();
        assert_eq!(clock.deadline(video, 50), epoch + Duration::from_secs(1));
        assert_eq!(
            clock.audio_deadline(48_000, 48_000),
            epoch + Duration::from_secs(1)
        );
        assert_eq!(clock.audio_pts(480, 48_000), 100_000);
    }
}
