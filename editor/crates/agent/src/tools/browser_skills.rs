use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainSkill {
    pub site: String,
    pub name: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

fn skills_dir() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("No config directory")?;
    let skills_path = config_dir.join("tau").join("domain-skills");
    std::fs::create_dir_all(&skills_path)?;
    Ok(skills_path)
}

fn normalize_site(site: &str) -> String {
    extract_site_from_url(site).unwrap_or_else(|| site.to_string())
}

pub fn get_skills_for_site(site: &str) -> Result<Vec<DomainSkill>> {
    let skills_path = skills_dir()?;
    let site_dir = skills_path.join(normalize_site(site));

    if !site_dir.exists() {
        return Ok(vec![]);
    }

    let mut skills = Vec::new();
    for entry in std::fs::read_dir(&site_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let content = std::fs::read_to_string(&path)?;
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            skills.push(DomainSkill {
                site: site.to_string(),
                name,
                content,
                created_at: String::new(),
                updated_at: String::new(),
            });
        }
    }

    Ok(skills)
}

pub fn save_skill(site: &str, name: &str, content: &str) -> Result<()> {
    let skills_path = skills_dir()?;
    let site_dir = skills_path.join(normalize_site(site));
    std::fs::create_dir_all(&site_dir)?;

    let skill_path = site_dir.join(format!("{}.md", name));
    std::fs::write(&skill_path, content)?;

    Ok(())
}

pub fn list_sites() -> Result<Vec<String>> {
    let skills_path = skills_dir()?;

    if !skills_path.exists() {
        return Ok(vec![]);
    }

    let mut sites = Vec::new();
    for entry in std::fs::read_dir(&skills_path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(site_name) = path.file_name().and_then(|s| s.to_str()) {
                sites.push(site_name.to_string());
            }
        }
    }

    Ok(sites)
}

fn extract_site_from_url(url: &str) -> Option<String> {
    let without_scheme = url.split("://").nth(1)?;
    let host = without_scheme.split(['/', '?', '#']).next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    host.split('.').next().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_site_from_url() {
        assert_eq!(extract_site_from_url("https://github.com/login"), Some("github".into()));
        assert_eq!(extract_site_from_url("https://www.google.com"), Some("google".into()));
        assert_eq!(extract_site_from_url("https://docs.rs/tokio"), Some("docs".into()));
    }
}
