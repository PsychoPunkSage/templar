#![allow(dead_code)]

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContributionType {
    SoleAuthor,
    PrimaryContributor,
    TeamMember,
    Reviewer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Education,
    Experience,
    Project,
    Skill,
    Publication,
    OpenSource,
    Award,
    Certification,
    Extracurricular,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceBullet {
    pub text: String,
    pub impact_markers: Vec<String>,
    pub confidence_marker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceEntry {
    pub company: String,
    pub role: String,
    pub date_start: NaiveDate,
    pub date_end: Option<NaiveDate>,
    pub team_size: Option<u32>,
    pub tech_stack: Vec<String>,
    pub contribution_type: ContributionType,
    pub location: Option<String>,
    pub bullets: Vec<ExperienceBullet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EducationEntry {
    pub institution: String,
    pub degree: String,
    pub field: String,
    pub date_start: NaiveDate,
    pub date_end: Option<NaiveDate>,
    pub gpa: Option<f64>,
    pub honors: Vec<String>,
    pub relevant_courses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub description: String,
    pub tech_stack: Vec<String>,
    pub date_start: Option<NaiveDate>,
    pub date_end: Option<NaiveDate>,
    pub url: Option<String>,
    pub contribution_type: ContributionType,
    pub bullets: Vec<ExperienceBullet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub category: String,
    pub items: Vec<String>,
    pub proficiency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicationEntry {
    pub title: String,
    pub venue: String,
    pub date: NaiveDate,
    pub authors: Vec<String>,
    pub url: Option<String>,
    pub contribution_type: ContributionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwardEntry {
    pub title: String,
    pub issuer: String,
    pub date: NaiveDate,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificationEntry {
    pub name: String,
    pub issuer: String,
    pub date_issued: NaiveDate,
    pub date_expires: Option<NaiveDate>,
    pub credential_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenSourceEntry {
    pub project_name: String,
    pub description: String,
    pub url: Option<String>,
    pub contribution_type: ContributionType,
    pub tech_stack: Vec<String>,
    pub bullets: Vec<ExperienceBullet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtracurricularEntry {
    pub organization: String,
    pub role: String,
    pub date_start: NaiveDate,
    pub date_end: Option<NaiveDate>,
    pub bullets: Vec<ExperienceBullet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "entry_type", rename_all = "snake_case")]
pub enum ContextEntryData {
    Education(EducationEntry),
    Experience(ExperienceEntry),
    Project(ProjectEntry),
    Skill(SkillEntry),
    Publication(PublicationEntry),
    OpenSource(OpenSourceEntry),
    Award(AwardEntry),
    Certification(CertificationEntry),
    Extracurricular(ExtracurricularEntry),
}

impl ContextEntryData {
    pub fn entry_type_str(&self) -> &'static str {
        match self {
            ContextEntryData::Education(_) => "education",
            ContextEntryData::Experience(_) => "experience",
            ContextEntryData::Project(_) => "project",
            ContextEntryData::Skill(_) => "skill",
            ContextEntryData::Publication(_) => "publication",
            ContextEntryData::OpenSource(_) => "open_source",
            ContextEntryData::Award(_) => "award",
            ContextEntryData::Certification(_) => "certification",
            ContextEntryData::Extracurricular(_) => "extracurricular",
        }
    }
}
