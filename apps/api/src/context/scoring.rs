#![allow(dead_code)]

use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub recency: f64,
    pub impact: f64,
    pub jd_relevance: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            recency: 0.5,
            impact: 0.3,
            jd_relevance: 0.2,
        }
    }
}

/// Computes recency score with 18-month half-life exponential decay.
/// Returns 1.0 for current positions (end_date = None) and evergreen entries.
pub fn compute_recency_score(
    end_date: Option<NaiveDate>,
    flagged_evergreen: bool,
    half_life_months: f64,
) -> f64 {
    if flagged_evergreen {
        return 1.0;
    }
    let end_date = match end_date {
        Some(d) => d,
        None => return 1.0, // current position
    };
    let now = Utc::now().naive_utc().date();
    let months_since = months_between(end_date, now);
    if months_since <= 0.0 {
        return 1.0;
    }
    (0.5_f64)
        .powf(months_since / half_life_months)
        .clamp(0.0, 1.0)
}

/// Combined relevance score: 0.5*recency + 0.3*impact + 0.2*jd_relevance
pub fn compute_combined_score(
    recency: f64,
    impact: f64,
    jd_relevance: f64,
    weights: &ScoringWeights,
) -> f64 {
    (weights.recency * recency + weights.impact * impact + weights.jd_relevance * jd_relevance)
        .clamp(0.0, 1.0)
}

fn months_between(start: NaiveDate, end: NaiveDate) -> f64 {
    let years = end.year() - start.year();
    let months = end.month() as i32 - start.month() as i32;
    let total = years * 12 + months;
    let day_frac = (end.day() as f64 - start.day() as f64) / 30.0;
    (total as f64 + day_frac).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evergreen_always_one() {
        let old = NaiveDate::from_ymd_opt(2010, 1, 1);
        assert_eq!(compute_recency_score(old, true, 18.0), 1.0);
    }

    #[test]
    fn test_current_job_is_one() {
        assert_eq!(compute_recency_score(None, false, 18.0), 1.0);
    }

    #[test]
    fn test_very_old_entry_low_score() {
        let old = NaiveDate::from_ymd_opt(2010, 1, 1);
        let score = compute_recency_score(old, false, 18.0);
        assert!(score < 0.01, "Score was {score}");
    }

    #[test]
    fn test_combined_score_full() {
        let w = ScoringWeights::default();
        let score = compute_combined_score(1.0, 1.0, 1.0, &w);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_combined_score_partial() {
        let w = ScoringWeights::default();
        // 0.5*0.8 + 0.3*0.6 + 0.2*0.4 = 0.4 + 0.18 + 0.08 = 0.66
        let score = compute_combined_score(0.8, 0.6, 0.4, &w);
        assert!((score - 0.66).abs() < 0.001, "Score was {score}");
    }

    #[test]
    fn test_combined_score_clamped() {
        let w = ScoringWeights {
            recency: 1.0,
            impact: 0.0,
            jd_relevance: 0.0,
        };
        assert_eq!(compute_combined_score(1.5, 0.0, 0.0, &w), 1.0);
    }
}
