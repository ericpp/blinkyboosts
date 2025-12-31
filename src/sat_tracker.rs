use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct SatTracker {
    total: i64,
    by_source: HashMap<String, i64>,
    last_triggered_multiple: HashMap<i64, i64>,
    triggered_once: HashMap<i64, bool>,
}

impl SatTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add sats from a boost/zap and return the new total
    pub fn add(&mut self, source: &str, sats: i64) -> i64 {
        self.total += sats;
        *self.by_source.entry(source.to_string()).or_insert(0) += sats;
        self.total
    }

    /// Get the current total sats
    pub fn get_total(&self) -> i64 {
        self.total
    }

    /// Check if a threshold should trigger based on the new total
    /// If trigger_multiple is true, returns true if the total has crossed a new multiple of the threshold
    /// If trigger_multiple is false, returns true only the first time the threshold is crossed
    /// previous_total: the total before the current boost was added
    /// new_total: the total after the current boost was added
    pub fn should_trigger_threshold(&self, previous_total: i64, new_total: i64, threshold: i64, trigger_multiple: bool) -> bool {
        if new_total < threshold {
            return false;
        }

        if trigger_multiple {
            // Trigger only when we cross from below a multiple threshold to at/above it
            let previous_multiple = previous_total / threshold;
            let new_multiple = new_total / threshold;

            // Check if we crossed a threshold boundary
            if new_multiple > previous_multiple {
                // We crossed a boundary, but only trigger if we haven't already triggered for this multiple
                let last_triggered_multiple = self.last_triggered_multiple.get(&threshold).copied().unwrap_or(0);
                new_multiple > last_triggered_multiple
            } else {
                false
            }
        } else {
            // Trigger only once
            !self.triggered_once.get(&threshold).copied().unwrap_or(false)
        }
    }

    /// Update the last triggered state for a threshold
    pub fn update_last_triggered_threshold(&mut self, threshold: i64, trigger_multiple: bool) {
        if trigger_multiple {
            let current_total = self.total;
            let current_multiple = current_total / threshold;
            self.last_triggered_multiple.insert(threshold, current_multiple);
        } else {
            self.triggered_once.insert(threshold, true);
        }
    }

    /// Initialize trigger state for all thresholds based on current total
    /// This should be called after loading historical boosts to prevent false triggers
    /// thresholds: Vec of (threshold, trigger_multiple) tuples
    pub fn sync_trigger_state(&mut self, thresholds: &[(i64, bool)]) {
        let current_total = self.total;
        for (threshold, trigger_multiple) in thresholds {
            if *trigger_multiple {
                // For multiple triggers, set the last triggered multiple to the current multiple
                let current_multiple = current_total / threshold;
                self.last_triggered_multiple.insert(*threshold, current_multiple);
            } else {
                // For single triggers, mark as triggered if we're already past the threshold
                if current_total >= *threshold {
                    self.triggered_once.insert(*threshold, true);
                }
            }
        }
    }
}