use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::event::{EnvelopeMeta, Event};

const DEFAULT_CAPACITY: usize = 1024;

/// Bounded ring for control events. The compose thread must never wait on this.
#[derive(Debug)]
pub struct EventHub {
    capacity: usize,
    next_sequence: u64,
    dropped: u64,
    events: VecDeque<Event>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(8),
            next_sequence: 1,
            dropped: 0,
            events: VecDeque::new(),
        }
    }

    pub fn next_sequence(&self) -> u64 {
        self.next_sequence.saturating_sub(1)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn meta(&self, session_revision: u64, request_id: &str) -> EnvelopeMeta {
        EnvelopeMeta {
            sequence: self.next_sequence,
            session_revision,
            unix_ms: unix_ms(),
            request_id: request_id.to_string(),
        }
    }

    pub fn publish(&mut self, mut event: Event) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        match &mut event {
            Event::Ready { meta }
            | Event::SessionChanged { meta, .. }
            | Event::LiveChanged { meta, .. }
            | Event::Resource { meta, .. }
            | Event::CommandApplied { meta, .. }
            | Event::TransitionStarted { meta, .. }
            | Event::TransitionCompleted { meta, .. }
            | Event::Lag { meta, .. }
            | Event::Failed { meta, .. }
            | Event::Discovered { meta, .. }
            | Event::Shutdown { meta } => {
                meta.sequence = sequence;
            }
        }
        if self.events.len() >= self.capacity {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(event);
    }

    /// Poll events with `sequence > after`. If the subscriber lagged past the
    /// retained window, a single `Lag` event is prepended.
    pub fn after(&self, after: u64) -> Vec<Event> {
        let oldest = self.events.front().map(|event| event.meta().sequence);
        let mut out = Vec::new();
        if let Some(oldest) = oldest {
            if after + 1 < oldest {
                out.push(Event::Lag {
                    meta: EnvelopeMeta {
                        sequence: after,
                        session_revision: 0,
                        unix_ms: unix_ms(),
                        request_id: String::new(),
                    },
                    missed: oldest.saturating_sub(after + 1),
                });
            }
        }
        out.extend(
            self.events
                .iter()
                .filter(|event| event.meta().sequence > after)
                .cloned(),
        );
        out
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;

    #[test]
    fn lag_is_signaled_when_ring_wraps() {
        let mut hub = EventHub::new(8);
        for _ in 0..20 {
            let meta = hub.meta(1, "r");
            hub.publish(Event::Ready { meta });
        }
        let events = hub.after(0);
        assert!(matches!(events.first(), Some(Event::Lag { missed, .. }) if *missed > 0));
    }
}
