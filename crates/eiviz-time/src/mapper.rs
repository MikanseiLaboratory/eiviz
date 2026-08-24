use crate::{ClockDomain, MediaTime, Rational, TimeError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

const PARTS_PER_BILLION: i128 = 1_000_000_000;

/// An integer tick value in an explicit clock domain and timebase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockTimestamp {
    pub domain: ClockDomain,
    pub ticks: i64,
    pub timebase: Rational,
}

impl ClockTimestamp {
    pub fn new(domain: ClockDomain, ticks: i64, timebase: Rational) -> Result<Self, TimeError> {
        if timebase.numerator() <= 0 {
            return Err(TimeError::InvalidTimebase);
        }
        Ok(Self {
            domain,
            ticks,
            timebase,
        })
    }

    pub fn from_media(domain: ClockDomain, time: MediaTime) -> Result<Self, TimeError> {
        Self::new(domain, time.ticks(), time.timebase())
    }

    pub fn nanoseconds(domain: ClockDomain, nanos: u64) -> Result<Self, TimeError> {
        Self::new(
            domain,
            i64::try_from(nanos).map_err(|_| TimeError::Overflow)?,
            Rational::new(1, 1_000_000_000).expect("constant timebase"),
        )
    }
}

/// A simultaneous observation of two clocks. `target` is normally monotonic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockObservation {
    pub source: ClockTimestamp,
    pub target: ClockTimestamp,
    pub discontinuity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClockLockState {
    Unlocked,
    Acquiring,
    Locked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationStatus {
    Accepted,
    Locked,
    Duplicate,
    Reset,
}

/// Bounds for a mapper. Tick thresholds are expressed in target-domain ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockMapperConfig {
    pub window_size: usize,
    pub min_lock_observations: usize,
    pub max_drift_ppm: u32,
    pub jump_threshold_ticks: i64,
    pub max_offset_step_ticks: i64,
    pub filter_denominator: u32,
    /// Source counter modulus. For example, `Some(1 << 32)` unwraps RTP-style counters.
    pub source_wrap_ticks: Option<u64>,
}

impl Default for ClockMapperConfig {
    fn default() -> Self {
        Self {
            window_size: 16,
            min_lock_observations: 4,
            max_drift_ppm: 2_000,
            jump_threshold_ticks: 100_000_000,
            max_offset_step_ticks: 1_000_000,
            filter_denominator: 4,
            source_wrap_ticks: None,
        }
    }
}

impl ClockMapperConfig {
    fn validate(self) -> Result<Self, TimeError> {
        if self.window_size < 2
            || self.min_lock_observations < 2
            || self.min_lock_observations > self.window_size
            || self.max_drift_ppm == 0
            || self.jump_threshold_ticks <= 0
            || self.max_offset_step_ticks <= 0
            || self.filter_denominator == 0
            || self.source_wrap_ticks == Some(0)
        {
            return Err(TimeError::InvalidMapperConfig);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockMapperDiagnostics {
    pub source_domain: ClockDomain,
    pub target_domain: ClockDomain,
    pub state: ClockLockState,
    pub rate_ppb: i64,
    pub offset_ticks: i64,
    pub last_residual_ticks: i64,
    pub window_observations: usize,
    pub accepted_observations: u64,
    pub duplicate_observations: u64,
    pub bounded_regressions: u64,
    pub discontinuities: u64,
    pub wraps: u64,
}

/// Integer affine mapper with bounded least-squares drift and offset filtering.
///
/// The affine model is evaluated around an anchor to avoid loss of precision:
/// `target = target_anchor + scale(source - source_anchor, rate_ppb) + offset`.
#[derive(Clone, Debug)]
pub struct ClockMapper {
    source_domain: ClockDomain,
    source_timebase: Rational,
    target_domain: ClockDomain,
    target_timebase: Rational,
    config: ClockMapperConfig,
    state: ClockLockState,
    anchor_source: i128,
    anchor_target: i128,
    offset_ticks: i128,
    rate_ppb: i64,
    observations: VecDeque<(i128, i128)>,
    previous_raw_source: Option<i64>,
    previous_unwrapped_source: Option<i128>,
    wrap_epoch: i128,
    accepted: u64,
    duplicates: u64,
    bounded: u64,
    discontinuities: u64,
    wraps: u64,
    last_residual: i128,
}

impl ClockMapper {
    pub fn new(
        source_domain: ClockDomain,
        source_timebase: Rational,
        target_domain: ClockDomain,
        target_timebase: Rational,
        config: ClockMapperConfig,
    ) -> Result<Self, TimeError> {
        if source_domain == target_domain {
            return Err(TimeError::SameClockDomain(source_domain));
        }
        if source_timebase.numerator() <= 0 || target_timebase.numerator() <= 0 {
            return Err(TimeError::InvalidTimebase);
        }
        Ok(Self {
            source_domain,
            source_timebase,
            target_domain,
            target_timebase,
            config: config.validate()?,
            state: ClockLockState::Unlocked,
            anchor_source: 0,
            anchor_target: 0,
            offset_ticks: 0,
            rate_ppb: 0,
            observations: VecDeque::new(),
            previous_raw_source: None,
            previous_unwrapped_source: None,
            wrap_epoch: 0,
            accepted: 0,
            duplicates: 0,
            bounded: 0,
            discontinuities: 0,
            wraps: 0,
            last_residual: 0,
        })
    }

    pub fn exact(source: ClockTimestamp, target: ClockTimestamp) -> Result<Self, TimeError> {
        let mut mapper = Self::new(
            source.domain,
            source.timebase,
            target.domain,
            target.timebase,
            ClockMapperConfig::default(),
        )?;
        mapper.anchor_source = i128::from(source.ticks);
        mapper.anchor_target = i128::from(target.ticks);
        mapper.state = ClockLockState::Locked;
        mapper
            .observations
            .push_back((i128::from(source.ticks), i128::from(target.ticks)));
        mapper.accepted = 1;
        mapper.previous_raw_source = Some(source.ticks);
        mapper.previous_unwrapped_source = Some(i128::from(source.ticks));
        Ok(mapper)
    }

    pub fn observe(
        &mut self,
        observation: ClockObservation,
    ) -> Result<ObservationStatus, TimeError> {
        self.check_timestamp(observation.source, self.source_domain, self.source_timebase)?;
        self.check_timestamp(observation.target, self.target_domain, self.target_timebase)?;
        let source = self.unwrap_observation(observation.source.ticks)?;
        let target = i128::from(observation.target.ticks);

        if observation.discontinuity {
            self.reset_to(source, target);
            return Ok(ObservationStatus::Reset);
        }
        if self.previous_unwrapped_source == Some(source) {
            self.duplicates = self.duplicates.saturating_add(1);
            return Ok(ObservationStatus::Duplicate);
        }
        if self
            .previous_unwrapped_source
            .is_some_and(|previous| source < previous)
        {
            self.reset_to(source, target);
            return Ok(ObservationStatus::Reset);
        }

        if self.state != ClockLockState::Unlocked && self.observations.len() >= 2 {
            let predicted = self.map_unchecked(source)?;
            self.last_residual = target.checked_sub(predicted).ok_or(TimeError::Overflow)?;
            if self.last_residual.unsigned_abs() > self.config.jump_threshold_ticks as u128 {
                self.reset_to(source, target);
                return Ok(ObservationStatus::Reset);
            }
        }

        if self.state == ClockLockState::Unlocked {
            self.anchor_source = source;
            self.anchor_target = target;
            self.state = ClockLockState::Acquiring;
        }
        self.observations.push_back((source, target));
        if self.observations.len() > self.config.window_size {
            self.observations.pop_front();
        }
        self.accepted = self.accepted.saturating_add(1);
        self.previous_unwrapped_source = Some(source);
        self.fit()?;

        if self.observations.len() >= self.config.min_lock_observations {
            self.state = ClockLockState::Locked;
            Ok(ObservationStatus::Locked)
        } else {
            Ok(ObservationStatus::Accepted)
        }
    }

    pub fn map(&self, source: ClockTimestamp) -> Result<ClockTimestamp, TimeError> {
        if self.state != ClockLockState::Locked {
            return Err(TimeError::ClockUnlocked {
                from_domain: self.source_domain,
                to_domain: self.target_domain,
            });
        }
        self.check_timestamp(source, self.source_domain, self.source_timebase)?;
        let source_ticks = self.unwrap_for_mapping(source.ticks)?;
        let target = self.map_unchecked(source_ticks)?;
        ClockTimestamp::new(
            self.target_domain,
            i64::try_from(target).map_err(|_| TimeError::Overflow)?,
            self.target_timebase,
        )
    }

    pub fn map_inverse(&self, target: ClockTimestamp) -> Result<ClockTimestamp, TimeError> {
        if self.state != ClockLockState::Locked {
            return Err(TimeError::ClockUnlocked {
                from_domain: self.target_domain,
                to_domain: self.source_domain,
            });
        }
        self.check_timestamp(target, self.target_domain, self.target_timebase)?;
        let target_delta = i128::from(target.ticks)
            .checked_sub(self.anchor_target)
            .and_then(|value| value.checked_sub(self.offset_ticks))
            .ok_or(TimeError::Overflow)?;
        let source_delta = self.inverse_scale(target_delta)?;
        let source = self
            .anchor_source
            .checked_add(source_delta)
            .ok_or(TimeError::Overflow)?;
        let wrapped = if let Some(modulus) = self.config.source_wrap_ticks {
            source.rem_euclid(i128::from(modulus))
        } else {
            source
        };
        ClockTimestamp::new(
            self.source_domain,
            i64::try_from(wrapped).map_err(|_| TimeError::Overflow)?,
            self.source_timebase,
        )
    }

    pub fn reset(&mut self) {
        self.state = ClockLockState::Unlocked;
        self.observations.clear();
        self.previous_raw_source = None;
        self.previous_unwrapped_source = None;
        self.wrap_epoch = 0;
        self.rate_ppb = 0;
        self.offset_ticks = 0;
        self.last_residual = 0;
        self.discontinuities = self.discontinuities.saturating_add(1);
    }

    /// Re-anchor an exact mapper while preserving reset diagnostics.
    pub fn reanchor_exact(
        &mut self,
        source: ClockTimestamp,
        target: ClockTimestamp,
    ) -> Result<(), TimeError> {
        self.check_timestamp(source, self.source_domain, self.source_timebase)?;
        self.check_timestamp(target, self.target_domain, self.target_timebase)?;
        self.anchor_source = i128::from(source.ticks);
        self.anchor_target = i128::from(target.ticks);
        self.offset_ticks = 0;
        self.rate_ppb = 0;
        self.observations.clear();
        self.observations
            .push_back((self.anchor_source, self.anchor_target));
        self.previous_raw_source = Some(source.ticks);
        self.previous_unwrapped_source = Some(self.anchor_source);
        self.wrap_epoch = 0;
        self.last_residual = 0;
        self.accepted = self.accepted.saturating_add(1);
        self.discontinuities = self.discontinuities.saturating_add(1);
        self.state = ClockLockState::Locked;
        Ok(())
    }

    pub fn state(&self) -> ClockLockState {
        self.state
    }

    pub fn diagnostics(&self) -> ClockMapperDiagnostics {
        ClockMapperDiagnostics {
            source_domain: self.source_domain,
            target_domain: self.target_domain,
            state: self.state,
            rate_ppb: self.rate_ppb,
            offset_ticks: saturating_i64(self.offset_ticks),
            last_residual_ticks: saturating_i64(self.last_residual),
            window_observations: self.observations.len(),
            accepted_observations: self.accepted,
            duplicate_observations: self.duplicates,
            bounded_regressions: self.bounded,
            discontinuities: self.discontinuities,
            wraps: self.wraps,
        }
    }

    fn fit(&mut self) -> Result<(), TimeError> {
        if self.observations.len() < 2 {
            return Ok(());
        }
        let n = self.observations.len() as i128;
        let mut sum_x = 0_i128;
        let mut sum_y = 0_i128;
        let mut sum_xx = 0_i128;
        let mut sum_xy = 0_i128;
        for &(source, target) in &self.observations {
            let x = source
                .checked_sub(self.anchor_source)
                .ok_or(TimeError::Overflow)?;
            let y = target
                .checked_sub(self.anchor_target)
                .ok_or(TimeError::Overflow)?;
            sum_x = sum_x.checked_add(x).ok_or(TimeError::Overflow)?;
            sum_y = sum_y.checked_add(y).ok_or(TimeError::Overflow)?;
            sum_xx = sum_xx
                .checked_add(x.checked_mul(x).ok_or(TimeError::Overflow)?)
                .ok_or(TimeError::Overflow)?;
            sum_xy = sum_xy
                .checked_add(x.checked_mul(y).ok_or(TimeError::Overflow)?)
                .ok_or(TimeError::Overflow)?;
        }
        let covariance = n
            .checked_mul(sum_xy)
            .and_then(|value| value.checked_sub(sum_x.checked_mul(sum_y)?))
            .ok_or(TimeError::Overflow)?;
        let variance = n
            .checked_mul(sum_xx)
            .and_then(|value| value.checked_sub(sum_x.checked_mul(sum_x)?))
            .ok_or(TimeError::Overflow)?;
        if covariance <= 0 || variance <= 0 {
            return Ok(());
        }

        let correction_numerator = covariance
            .checked_mul(i128::from(self.source_timebase.denominator()))
            .and_then(|value| value.checked_mul(i128::from(self.target_timebase.numerator())))
            .and_then(|value| value.checked_mul(PARTS_PER_BILLION))
            .ok_or(TimeError::Overflow)?;
        let correction_denominator = variance
            .checked_mul(i128::from(self.source_timebase.numerator()))
            .and_then(|value| value.checked_mul(i128::from(self.target_timebase.denominator())))
            .ok_or(TimeError::Overflow)?;
        let measured = correction_numerator
            .checked_div(correction_denominator)
            .and_then(|value| value.checked_sub(PARTS_PER_BILLION))
            .ok_or(TimeError::Overflow)?;
        let limit = i128::from(self.config.max_drift_ppm) * 1_000;
        let bounded = measured.clamp(-limit, limit);
        if bounded != measured {
            self.bounded = self.bounded.saturating_add(1);
        }
        let filtered = i128::from(self.rate_ppb)
            + (bounded - i128::from(self.rate_ppb)) / i128::from(self.config.filter_denominator);
        self.rate_ppb = i64::try_from(filtered).map_err(|_| TimeError::Overflow)?;

        let mut residual_sum = 0_i128;
        for &(source, target) in &self.observations {
            let predicted_without_offset = self
                .anchor_target
                .checked_add(
                    self.scale(
                        source
                            .checked_sub(self.anchor_source)
                            .ok_or(TimeError::Overflow)?,
                    )?,
                )
                .ok_or(TimeError::Overflow)?;
            residual_sum = residual_sum
                .checked_add(
                    target
                        .checked_sub(predicted_without_offset)
                        .ok_or(TimeError::Overflow)?,
                )
                .ok_or(TimeError::Overflow)?;
        }
        let measured_offset = residual_sum / n;
        let delta = (measured_offset - self.offset_ticks).clamp(
            -i128::from(self.config.max_offset_step_ticks),
            i128::from(self.config.max_offset_step_ticks),
        );
        self.offset_ticks += delta / i128::from(self.config.filter_denominator);
        Ok(())
    }

    fn map_unchecked(&self, source: i128) -> Result<i128, TimeError> {
        self.anchor_target
            .checked_add(
                self.scale(
                    source
                        .checked_sub(self.anchor_source)
                        .ok_or(TimeError::Overflow)?,
                )?,
            )
            .and_then(|value| value.checked_add(self.offset_ticks))
            .ok_or(TimeError::Overflow)
    }

    fn scale(&self, source_delta: i128) -> Result<i128, TimeError> {
        let numerator = source_delta
            .checked_mul(i128::from(self.source_timebase.numerator()))
            .and_then(|value| value.checked_mul(i128::from(self.target_timebase.denominator())))
            .and_then(|value| value.checked_mul(PARTS_PER_BILLION + i128::from(self.rate_ppb)))
            .ok_or(TimeError::Overflow)?;
        let denominator = i128::from(self.source_timebase.denominator())
            .checked_mul(i128::from(self.target_timebase.numerator()))
            .and_then(|value| value.checked_mul(PARTS_PER_BILLION))
            .ok_or(TimeError::Overflow)?;
        div_floor(numerator, denominator)
    }

    fn inverse_scale(&self, target_delta: i128) -> Result<i128, TimeError> {
        let numerator = target_delta
            .checked_mul(i128::from(self.source_timebase.denominator()))
            .and_then(|value| value.checked_mul(i128::from(self.target_timebase.numerator())))
            .and_then(|value| value.checked_mul(PARTS_PER_BILLION))
            .ok_or(TimeError::Overflow)?;
        let denominator = i128::from(self.source_timebase.numerator())
            .checked_mul(i128::from(self.target_timebase.denominator()))
            .and_then(|value| value.checked_mul(PARTS_PER_BILLION + i128::from(self.rate_ppb)))
            .ok_or(TimeError::Overflow)?;
        div_floor(numerator, denominator)
    }

    fn check_timestamp(
        &self,
        timestamp: ClockTimestamp,
        domain: ClockDomain,
        timebase: Rational,
    ) -> Result<(), TimeError> {
        if timestamp.domain != domain {
            return Err(TimeError::DomainMismatch(timestamp.domain, domain));
        }
        if timestamp.timebase != timebase {
            return Err(TimeError::TimebaseMismatch);
        }
        Ok(())
    }

    fn unwrap_observation(&mut self, raw: i64) -> Result<i128, TimeError> {
        let Some(modulus) = self.config.source_wrap_ticks else {
            self.previous_raw_source = Some(raw);
            return Ok(i128::from(raw));
        };
        if raw < 0 || raw as u64 >= modulus {
            return Err(TimeError::CounterOutsideModulus);
        }
        if let Some(previous) = self.previous_raw_source {
            let backwards = i128::from(previous) - i128::from(raw);
            if backwards > i128::from(modulus / 2) {
                self.wrap_epoch = self
                    .wrap_epoch
                    .checked_add(i128::from(modulus))
                    .ok_or(TimeError::Overflow)?;
                self.wraps = self.wraps.saturating_add(1);
            }
        }
        self.previous_raw_source = Some(raw);
        self.wrap_epoch
            .checked_add(i128::from(raw))
            .ok_or(TimeError::Overflow)
    }

    fn unwrap_for_mapping(&self, raw: i64) -> Result<i128, TimeError> {
        let Some(modulus) = self.config.source_wrap_ticks else {
            return Ok(i128::from(raw));
        };
        if raw < 0 || raw as u64 >= modulus {
            return Err(TimeError::CounterOutsideModulus);
        }
        let candidate = self
            .wrap_epoch
            .checked_add(i128::from(raw))
            .ok_or(TimeError::Overflow)?;
        let Some(previous) = self.previous_unwrapped_source else {
            return Ok(candidate);
        };
        let modulus = i128::from(modulus);
        Ok([candidate - modulus, candidate, candidate + modulus]
            .into_iter()
            .min_by_key(|value| (*value - previous).unsigned_abs())
            .expect("three candidates"))
    }

    fn reset_to(&mut self, source: i128, target: i128) {
        self.state = ClockLockState::Acquiring;
        self.anchor_source = source;
        self.anchor_target = target;
        self.offset_ticks = 0;
        self.rate_ppb = 0;
        self.observations.clear();
        self.observations.push_back((source, target));
        self.previous_unwrapped_source = Some(source);
        self.accepted = self.accepted.saturating_add(1);
        self.discontinuities = self.discontinuities.saturating_add(1);
        self.last_residual = 0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingIslandDiagnostics {
    pub reference_domain: ClockDomain,
    pub state: ClockLockState,
    pub mappers: Vec<ClockMapperDiagnostics>,
    pub external_locks: Vec<(ClockDomain, bool)>,
}

/// A set of clocks correlated through one explicit reference domain.
#[derive(Clone, Debug)]
pub struct TimingIsland {
    reference_domain: ClockDomain,
    mappers: HashMap<ClockDomain, ClockMapper>,
    external_locks: HashMap<ClockDomain, bool>,
}

impl TimingIsland {
    pub fn new(reference_domain: ClockDomain) -> Self {
        Self {
            reference_domain,
            mappers: HashMap::new(),
            external_locks: HashMap::new(),
        }
    }

    pub fn add_mapper(&mut self, mapper: ClockMapper) -> Result<(), TimeError> {
        if mapper.target_domain != self.reference_domain {
            return Err(TimeError::DomainMismatch(
                mapper.target_domain,
                self.reference_domain,
            ));
        }
        self.mappers.insert(mapper.source_domain, mapper);
        Ok(())
    }

    pub fn has_mapper(&self, source: ClockDomain) -> bool {
        self.mappers.contains_key(&source)
    }

    pub fn mapper_state(&self, source: ClockDomain) -> ClockLockState {
        self.mappers
            .get(&source)
            .map_or(ClockLockState::Unlocked, ClockMapper::state)
    }

    pub fn reanchor_exact(
        &mut self,
        source: ClockTimestamp,
        target: ClockTimestamp,
    ) -> Result<(), TimeError> {
        self.mappers
            .get_mut(&source.domain)
            .ok_or(TimeError::MapperMissing {
                from_domain: source.domain,
                to_domain: self.reference_domain,
            })?
            .reanchor_exact(source, target)
    }

    pub fn observe(
        &mut self,
        observation: ClockObservation,
    ) -> Result<ObservationStatus, TimeError> {
        if observation.target.domain != self.reference_domain {
            return Err(TimeError::DomainMismatch(
                observation.target.domain,
                self.reference_domain,
            ));
        }
        let mapper =
            self.mappers
                .get_mut(&observation.source.domain)
                .ok_or(TimeError::MapperMissing {
                    from_domain: observation.source.domain,
                    to_domain: self.reference_domain,
                })?;
        mapper.observe(observation)
    }

    pub fn map(
        &self,
        timestamp: ClockTimestamp,
        target: ClockDomain,
    ) -> Result<ClockTimestamp, TimeError> {
        if timestamp.domain == target {
            return Ok(timestamp);
        }
        let reference = if timestamp.domain == self.reference_domain {
            timestamp
        } else {
            self.mappers
                .get(&timestamp.domain)
                .ok_or(TimeError::MapperMissing {
                    from_domain: timestamp.domain,
                    to_domain: self.reference_domain,
                })?
                .map(timestamp)?
        };
        if target == self.reference_domain {
            return Ok(reference);
        }
        self.mappers
            .get(&target)
            .ok_or(TimeError::MapperMissing {
                from_domain: self.reference_domain,
                to_domain: target,
            })?
            .map_inverse(reference)
    }

    pub fn set_external_lock(&mut self, domain: ClockDomain, locked: bool) {
        self.external_locks.insert(domain, locked);
    }

    pub fn state(&self) -> ClockLockState {
        if self.external_locks.values().any(|locked| !locked)
            || self
                .mappers
                .values()
                .any(|mapper| mapper.state() == ClockLockState::Unlocked)
        {
            ClockLockState::Unlocked
        } else if self
            .mappers
            .values()
            .any(|mapper| mapper.state() == ClockLockState::Acquiring)
        {
            ClockLockState::Acquiring
        } else if self.mappers.is_empty() && self.external_locks.is_empty() {
            ClockLockState::Unlocked
        } else {
            ClockLockState::Locked
        }
    }

    pub fn diagnostics(&self) -> TimingIslandDiagnostics {
        let mut mappers = self
            .mappers
            .values()
            .map(ClockMapper::diagnostics)
            .collect::<Vec<_>>();
        mappers.sort_by_key(|mapper| format!("{:?}", mapper.source_domain));
        let mut external_locks = self
            .external_locks
            .iter()
            .map(|(domain, locked)| (*domain, *locked))
            .collect::<Vec<_>>();
        external_locks.sort_by_key(|(domain, _)| format!("{domain:?}"));
        TimingIslandDiagnostics {
            reference_domain: self.reference_domain,
            state: self.state(),
            mappers,
            external_locks,
        }
    }
}

fn div_floor(numerator: i128, denominator: i128) -> Result<i128, TimeError> {
    if denominator <= 0 {
        return Err(TimeError::InvalidTimebase);
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    Ok(if remainder < 0 {
        quotient.checked_sub(1).ok_or(TimeError::Overflow)?
    } else {
        quotient
    })
}

fn saturating_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb(rate: i64) -> Rational {
        Rational::new(1, rate).unwrap()
    }

    fn point(domain: ClockDomain, ticks: i64, rate: i64) -> ClockTimestamp {
        ClockTimestamp::new(domain, ticks, tb(rate)).unwrap()
    }

    #[test]
    fn exact_affine_mapping_is_integer_and_reversible() {
        let source = point(ClockDomain::SourceMedia, 90_000, 90_000);
        let target = point(ClockDomain::Monotonic, 5_000_000_000, 1_000_000_000);
        let mapper = ClockMapper::exact(source, target).unwrap();
        let mapped = mapper
            .map(point(ClockDomain::SourceMedia, 180_000, 90_000))
            .unwrap();
        assert_eq!(mapped.ticks, 6_000_000_000);
        assert_eq!(mapper.map_inverse(mapped).unwrap().ticks, 180_000);
    }

    #[test]
    fn exact_reanchor_preserves_reset_diagnostics() {
        let mut mapper = ClockMapper::exact(
            point(ClockDomain::SourceMedia, 0, 1_000),
            point(ClockDomain::Monotonic, 10_000, 1_000),
        )
        .unwrap();
        mapper
            .reanchor_exact(
                point(ClockDomain::SourceMedia, 500, 1_000),
                point(ClockDomain::Monotonic, 20_000, 1_000),
            )
            .unwrap();
        assert_eq!(mapper.state(), ClockLockState::Locked);
        assert_eq!(mapper.diagnostics().discontinuities, 1);
        assert_eq!(
            mapper
                .map(point(ClockDomain::SourceMedia, 501, 1_000))
                .unwrap()
                .ticks,
            20_001
        );
    }

    #[test]
    fn bounded_regression_locks_and_tracks_drift() {
        let mut mapper = ClockMapper::new(
            ClockDomain::AudioSample,
            tb(48_000),
            ClockDomain::Monotonic,
            tb(1_000_000_000),
            ClockMapperConfig {
                max_drift_ppm: 500,
                jump_threshold_ticks: 50_000_000,
                ..ClockMapperConfig::default()
            },
        )
        .unwrap();
        // +100 ppm target duration for each nominal second.
        for second in 0..20_i64 {
            mapper
                .observe(ClockObservation {
                    source: point(ClockDomain::AudioSample, second * 48_000, 48_000),
                    target: point(
                        ClockDomain::Monotonic,
                        second * 1_000_100_000,
                        1_000_000_000,
                    ),
                    discontinuity: false,
                })
                .unwrap();
        }
        let diagnostics = mapper.diagnostics();
        assert_eq!(diagnostics.state, ClockLockState::Locked);
        assert!((80_000..=100_000).contains(&diagnostics.rate_ppb));
        let mapped = mapper
            .map(point(ClockDomain::AudioSample, 20 * 48_000, 48_000))
            .unwrap();
        assert!((20_001_500_000..=20_002_500_000).contains(&mapped.ticks));
    }

    #[test]
    fn explicit_jump_resets_lock_and_filter() {
        let mut mapper = ClockMapper::new(
            ClockDomain::SourceMedia,
            tb(1_000),
            ClockDomain::Monotonic,
            tb(1_000_000_000),
            ClockMapperConfig {
                jump_threshold_ticks: 10_000_000,
                ..ClockMapperConfig::default()
            },
        )
        .unwrap();
        for tick in 0..4 {
            mapper
                .observe(ClockObservation {
                    source: point(ClockDomain::SourceMedia, tick, 1_000),
                    target: point(ClockDomain::Monotonic, tick * 1_000_000, 1_000_000_000),
                    discontinuity: false,
                })
                .unwrap();
        }
        assert_eq!(mapper.state(), ClockLockState::Locked);
        assert_eq!(
            mapper
                .observe(ClockObservation {
                    source: point(ClockDomain::SourceMedia, 4, 1_000),
                    target: point(ClockDomain::Monotonic, 1_000_000_000, 1_000_000_000),
                    discontinuity: false,
                })
                .unwrap(),
            ObservationStatus::Reset
        );
        assert_eq!(mapper.state(), ClockLockState::Acquiring);
        assert_eq!(mapper.diagnostics().rate_ppb, 0);
        assert!(matches!(
            mapper.map(point(ClockDomain::SourceMedia, 5, 1_000)),
            Err(TimeError::ClockUnlocked { .. })
        ));
    }

    #[test]
    fn source_counter_wrap_is_unwrapped_without_reset() {
        let mut mapper = ClockMapper::new(
            ClockDomain::DeckLinkStream,
            tb(1_000),
            ClockDomain::Monotonic,
            tb(1_000),
            ClockMapperConfig {
                source_wrap_ticks: Some(256),
                jump_threshold_ticks: 20,
                max_offset_step_ticks: 2,
                ..ClockMapperConfig::default()
            },
        )
        .unwrap();
        for (source, target) in [(253, 1000), (254, 1001), (255, 1002), (0, 1003), (1, 1004)] {
            mapper
                .observe(ClockObservation {
                    source: point(ClockDomain::DeckLinkStream, source, 1_000),
                    target: point(ClockDomain::Monotonic, target, 1_000),
                    discontinuity: false,
                })
                .unwrap();
        }
        assert_eq!(mapper.state(), ClockLockState::Locked);
        assert_eq!(mapper.diagnostics().wraps, 1);
        assert_eq!(
            mapper
                .map(point(ClockDomain::DeckLinkStream, 2, 1_000))
                .unwrap()
                .ticks,
            1005
        );
    }

    #[test]
    fn domains_and_timebases_never_mix_implicitly() {
        let mapper = ClockMapper::exact(
            point(ClockDomain::SourceMedia, 0, 90_000),
            point(ClockDomain::Monotonic, 0, 1_000_000_000),
        )
        .unwrap();
        assert!(matches!(
            mapper.map(point(ClockDomain::AudioSample, 0, 90_000)),
            Err(TimeError::DomainMismatch(..))
        ));
        assert_eq!(
            mapper
                .map(point(ClockDomain::SourceMedia, 90_000, 90_000))
                .unwrap()
                .ticks,
            1_000_000_000
        );
    }

    #[test]
    fn timing_island_maps_between_non_reference_clocks() {
        let mut island = TimingIsland::new(ClockDomain::Monotonic);
        island
            .add_mapper(
                ClockMapper::exact(
                    point(ClockDomain::SourceMedia, 0, 90_000),
                    point(ClockDomain::Monotonic, 1_000_000_000, 1_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        island
            .add_mapper(
                ClockMapper::exact(
                    point(ClockDomain::AudioSample, 0, 48_000),
                    point(ClockDomain::Monotonic, 1_000_000_000, 1_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let audio = island
            .map(
                point(ClockDomain::SourceMedia, 180_000, 90_000),
                ClockDomain::AudioSample,
            )
            .unwrap();
        assert_eq!(audio.ticks, 96_000);
        island.set_external_lock(ClockDomain::DeckLinkGenlock, false);
        assert_eq!(island.state(), ClockLockState::Unlocked);
        island.set_external_lock(ClockDomain::DeckLinkGenlock, true);
        assert_eq!(island.state(), ClockLockState::Locked);
    }
}
