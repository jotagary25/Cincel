//! Order statistics used by every measurement (p50, p90, p95, max).

/// Summary of a sample set, in the unit of the samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub max: f64,
}

/// Percentile with linear interpolation between closest ranks (the
/// "inclusive" definition: `p = 0` is the minimum and `p = 100` the maximum).
/// Returns `None` for an empty set; NaN samples are ignored.
#[cfg(test)]
pub fn percentile(samples: &[f64], p: f64) -> Option<f64> {
    let mut sorted: Vec<f64> = samples.iter().copied().filter(|v| !v.is_nan()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    Some(percentile_sorted(&sorted, p))
}

fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    let p = p.clamp(0.0, 100.0);
    let rank = p / 100.0 * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    let fraction = rank - low as f64;
    sorted[low] + (sorted[high] - sorted[low]) * fraction
}

/// p50/p90/p95/max of a sample set, `None` when it is empty.
pub fn summarize(samples: &[f64]) -> Option<Summary> {
    let mut sorted: Vec<f64> = samples.iter().copied().filter(|v| !v.is_nan()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    Some(Summary {
        count: sorted.len(),
        p50: percentile_sorted(&sorted, 50.0),
        p90: percentile_sorted(&sorted, 90.0),
        p95: percentile_sorted(&sorted, 95.0),
        max: sorted[sorted.len() - 1],
    })
}

/// Rounds to two decimals, the precision every JSON line carries.
pub fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
