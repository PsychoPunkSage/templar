//! JD Parser — extracts structured requirements, keywords, and tone from a raw job description.

use serde::{Deserialize, Serialize};

use crate::errors::AppError;
use crate::generation::prompts::{JD_PARSE_PROMPT_TEMPLATE, JD_PARSE_SYSTEM};
use crate::llm_client::LlmClient;

/// Detected tone of a job description. Drives verb selection in generation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum JDTone {
    AggressiveStartup,
    #[default]
    CollaborativeEnterprise,
    ResearchOriented,
    ProductOriented,
}

/// A single requirement extracted from the JD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub text: String,
    pub is_required: bool,
}

/// High-level signals about the role shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleSignals {
    pub is_startup: bool,
    pub is_ic_focused: bool,
    pub is_research: bool,
    pub seniority: String,
}

/// A single keyword from the JD, weighted by position and frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordEntry {
    pub keyword: String,
    pub frequency: u32,
    /// title=1.0, requirements=0.8, responsibilities=0.6, about=0.3
    pub position_weight: f32,
    /// frequency * position_weight
    pub weighted_score: f32,
}

/// Full structured output of JD parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedJD {
    pub hard_requirements: Vec<Requirement>,
    pub soft_signals: Vec<String>,
    pub role_signals: RoleSignals,
    pub keyword_inventory: Vec<KeywordEntry>,
    pub detected_tone: JDTone,
}

/// Parses a job description using the LLM and returns a structured `ParsedJD`.
pub async fn parse_jd(jd_text: &str, llm: &LlmClient) -> Result<ParsedJD, AppError> {
    let prompt = JD_PARSE_PROMPT_TEMPLATE.replace("{jd_text}", jd_text);
    llm.call_json::<ParsedJD>(&prompt, JD_PARSE_SYSTEM)
        .await
        .map_err(|e| AppError::Llm(format!("JD parsing failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // JD fixture: Aggressive startup
    const STARTUP_JD: &str = r#"
        Senior Rust Engineer — Core Infrastructure
        We move fast and own everything end-to-end. You will architect distributed systems,
        spearhead performance initiatives, and drive reliability from zero to production.
        Requirements: 5+ years Rust required, systems programming required, distributed systems expertise required.
        Nice to have: Kubernetes, Kafka experience a plus.
        About Us: Fast-paced Series B startup disrupting fintech infrastructure.
    "#;

    // JD fixture: Collaborative enterprise
    const ENTERPRISE_JD: &str = r#"
        Software Engineer — Platform Team
        Join our collaborative team to contribute to our microservices platform.
        You will partner with product managers and support reliability goals.
        Required: Java, Spring Boot, SQL. Preferred: Kubernetes, CI/CD experience.
        About: Global enterprise with 50,000 employees focused on financial services.
    "#;

    // JD fixture: Research role
    const RESEARCH_JD: &str = r#"
        Research Scientist — ML Systems
        Investigate novel approaches to large language model training efficiency.
        Publish findings in top venues (NeurIPS, ICML). Evaluate proposed architectures.
        Required: PhD in CS/ML, experience with PyTorch. Preferred: Publications at top venues.
        About: Research lab at the frontier of AI.
    "#;

    #[test]
    fn test_jd_tone_serde_aggressive_startup() {
        let json = r#""AggressiveStartup""#;
        let tone: JDTone = serde_json::from_str(json).unwrap();
        assert_eq!(tone, JDTone::AggressiveStartup);
    }

    #[test]
    fn test_jd_tone_serde_collaborative_enterprise() {
        let json = r#""CollaborativeEnterprise""#;
        let tone: JDTone = serde_json::from_str(json).unwrap();
        assert_eq!(tone, JDTone::CollaborativeEnterprise);
    }

    #[test]
    fn test_jd_tone_serde_research_oriented() {
        let json = r#""ResearchOriented""#;
        let tone: JDTone = serde_json::from_str(json).unwrap();
        assert_eq!(tone, JDTone::ResearchOriented);
    }

    #[test]
    fn test_jd_tone_serde_product_oriented() {
        let json = r#""ProductOriented""#;
        let tone: JDTone = serde_json::from_str(json).unwrap();
        assert_eq!(tone, JDTone::ProductOriented);
    }

    #[test]
    fn test_parsed_jd_full_deserializes_correctly() {
        let json = r#"{
            "hard_requirements": [
                {"text": "5+ years Rust", "is_required": true},
                {"text": "Systems programming", "is_required": true}
            ],
            "soft_signals": ["Kubernetes experience", "Kafka"],
            "role_signals": {
                "is_startup": true,
                "is_ic_focused": true,
                "is_research": false,
                "seniority": "senior"
            },
            "keyword_inventory": [
                {
                    "keyword": "Rust",
                    "frequency": 5,
                    "position_weight": 0.8,
                    "weighted_score": 4.0
                },
                {
                    "keyword": "distributed systems",
                    "frequency": 2,
                    "position_weight": 0.6,
                    "weighted_score": 1.2
                }
            ],
            "detected_tone": "AggressiveStartup"
        }"#;

        let parsed: ParsedJD = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.detected_tone, JDTone::AggressiveStartup);
        assert_eq!(parsed.hard_requirements.len(), 2);
        assert!(parsed.hard_requirements[0].is_required);
        assert_eq!(parsed.soft_signals.len(), 2);
        assert_eq!(parsed.keyword_inventory[0].keyword, "Rust");
        assert!((parsed.keyword_inventory[0].weighted_score - 4.0).abs() < f32::EPSILON);
        assert!(parsed.role_signals.is_startup);
        assert_eq!(parsed.role_signals.seniority, "senior");
    }

    #[test]
    fn test_keyword_entry_weighted_score_is_freq_times_weight() {
        let entry = KeywordEntry {
            keyword: "Rust".to_string(),
            frequency: 5,
            position_weight: 0.8,
            weighted_score: 4.0,
        };
        let expected = entry.frequency as f32 * entry.position_weight;
        assert!((entry.weighted_score - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn test_jd_tone_default_is_collaborative() {
        let tone = JDTone::default();
        assert_eq!(tone, JDTone::CollaborativeEnterprise);
    }

    /// Verify fixture JDs are present (used for integration tests with a real LLM).
    #[test]
    fn test_fixtures_are_nonempty() {
        assert!(!STARTUP_JD.trim().is_empty());
        assert!(!ENTERPRISE_JD.trim().is_empty());
        assert!(!RESEARCH_JD.trim().is_empty());
    }
}
