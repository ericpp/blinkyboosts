use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct SatTracker {
    total: i64,
    by_source: HashMap<String, i64>,
    cycle_total: i64,
}

impl SatTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, source: &str, sats: i64) -> i64 {
        self.total += sats;
        *self.by_source.entry(source.to_string()).or_insert(0) += sats;
        self.total
    }

    /// Check which thresholds are crossed by this boost
    pub fn get_thresholds_to_trigger(
        &mut self,
        boost_amount: i64,
        all_thresholds: &[i64],
        max_threshold: i64
    ) -> Vec<i64> {
        let old_cycle = self.cycle_total;
        let new_cycle = old_cycle + boost_amount;
        
        let mut triggered = Vec::new();

        // Handle max threshold crossing with wraparound
        if new_cycle >= max_threshold {
            triggered.push(max_threshold);
            self.cycle_total = new_cycle - max_threshold;
            
            // After reset, check if other thresholds are met
            for &threshold in all_thresholds {
                if threshold != max_threshold && self.cycle_total >= threshold {
                    triggered.push(threshold);
                }
            }
        } else {
            self.cycle_total = new_cycle;
            
            // Check normal threshold crossings
            for &threshold in all_thresholds {
                if old_cycle < threshold && new_cycle >= threshold {
                    triggered.push(threshold);
                }
            }
        }

        triggered.sort_unstable();
        triggered
    }

    /// Sync cycle position based on total (call after loading historical data)
    pub fn sync_trigger_state(&mut self, max_threshold: i64) {
        self.cycle_total = self.total % max_threshold;
    }
}