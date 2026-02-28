use serde::{Deserialize, Serialize};

use crate::models::context::ContextEntryRow;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SectionStatus {
    Strong,
    Moderate,
    Weak,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionHealth {
    pub section: String,
    pub score: f64,
    pub entry_count: usize,
    pub missing_quantification: usize,
    pub status: SectionStatus,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessReport {
    pub overall_score: f64,
    pub sections: Vec<SectionHealth>,
    pub total_entries: usize,
    pub missing_sections: Vec<String>,
}

const SECTION_WEIGHTS: &[(&str, f64)] = &[
    ("experience", 0.35),
    ("education", 0.15),
    ("skill", 0.15),
    ("project", 0.15),
    ("publication", 0.05),
    ("open_source", 0.05),
    ("certification", 0.05),
    ("award", 0.03),
    ("extracurricular", 0.02),
];

pub fn compute_completeness_report(entries: &[ContextEntryRow]) -> CompletenessReport {
    let total_entries = entries.len();
    let mut section_healths = Vec::new();
    let mut weighted_score_sum = 0.0;
    let mut missing_sections = Vec::new();

    for (section_key, weight) in SECTION_WEIGHTS {
        let section_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == *section_key)
            .collect();

        let entry_count = section_entries.len();

        if entry_count == 0 {
            missing_sections.push(section_key.to_string());
            section_healths.push(SectionHealth {
                section: section_key.to_string(),
                score: 0.0,
                entry_count: 0,
                missing_quantification: 0,
                status: SectionStatus::Missing,
                recommendations: vec![format!(
                    "Add at least one {} entry to strengthen your context",
                    section_key
                )],
            });
            continue;
        }

        let missing_quantification = section_entries
            .iter()
            .filter(|e| e.impact_score < 0.5)
            .count();

        let section_score: f64 = {
            let sum: f64 = section_entries
                .iter()
                .map(|e| e.recency_score * e.impact_score)
                .sum();
            (sum / entry_count as f64).clamp(0.0, 1.0)
        };

        let status = match section_score {
            s if s >= 0.8 => SectionStatus::Strong,
            s if s >= 0.5 => SectionStatus::Moderate,
            s if s >= 0.2 => SectionStatus::Weak,
            _ => SectionStatus::Missing,
        };

        let mut recommendations = Vec::new();
        if missing_quantification > 0 {
            recommendations.push(format!(
                "{} {} entries lack quantified metrics — add numbers or [LOW_METRICS] markers",
                missing_quantification, section_key
            ));
        }
        if entry_count < 2 && *section_key == "experience" {
            recommendations
                .push("Add more experience entries to build a complete picture".to_string());
        }

        weighted_score_sum += section_score * weight;
        section_healths.push(SectionHealth {
            section: section_key.to_string(),
            score: section_score,
            entry_count,
            missing_quantification,
            status,
            recommendations,
        });
    }

    let total_weight: f64 = SECTION_WEIGHTS.iter().map(|(_, w)| w).sum();
    let overall_score = if total_weight > 0.0 {
        (weighted_score_sum / total_weight).clamp(0.0, 1.0)
    } else {
        0.0
    };

    CompletenessReport {
        overall_score,
        sections: section_healths,
        total_entries,
        missing_sections,
    }
}
