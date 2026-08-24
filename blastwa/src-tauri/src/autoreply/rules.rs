// auto-reply rule engine: keyword match + watcher task
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchType {
    Like,
    StartWith,
    EndWith,
    Contains,
}

impl MatchType {
    pub fn matches(&self, message: &str, keyword: &str) -> bool {
        match self {
            MatchType::Like => message == keyword,
            MatchType::StartWith => message.starts_with(keyword),
            MatchType::EndWith => message.ends_with(keyword),
            MatchType::Contains => message.contains(keyword),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub match_type: MatchType,
    pub keyword: String,
    #[serde(default)]
    pub reply_message: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// first matching enabled rule wins — mirrors original behavior
pub fn match_rule<'a>(message: &str, rules: &'a [Rule]) -> Option<&'a Rule> {
    rules.iter().find(|r| r.enabled && r.match_type.matches(message, &r.keyword))
}

pub fn save_rules(rules: &[Rule], path: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    Ok(std::fs::write(path, serde_json::to_string_pretty(rules)?)?)
}

pub fn load_rules(path: &std::path::Path) -> anyhow::Result<Vec<Rule>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(t: MatchType, kw: &str) -> Rule {
        Rule {
            name: "t".into(),
            match_type: t,
            keyword: kw.into(),
            reply_message: Some("reply".into()),
            enabled: true,
        }
    }

    #[test]
    fn contains_matches_substring() {
        assert!(match_rule("ada promo?", &[rule(MatchType::Contains, "promo")]).is_some());
        assert!(match_rule("tidak ada", &[rule(MatchType::Contains, "promo")]).is_none());
    }

    #[test]
    fn like_is_exact_only() {
        assert!(match_rule("halo bos", &[rule(MatchType::Like, "halo bos")]).is_some());
        assert!(match_rule("halo sob", &[rule(MatchType::Like, "halo bos")]).is_none());
    }

    #[test]
    fn start_and_end_anchors() {
        assert!(match_rule("halo bos", &[rule(MatchType::StartWith, "halo")]).is_some());
        assert!(!match_rule("eh halo bos", &[rule(MatchType::StartWith, "halo")]).is_some());
        assert!(match_rule("oke bos", &[rule(MatchType::EndWith, "bos")]).is_some());
    }

    #[test]
    fn first_match_wins_and_disabled_skipped() {
        let mut disabled = rule(MatchType::Contains, "promo");
        disabled.enabled = false;
        let active = rule(MatchType::Contains, "diskon");
        let rules = [disabled, active];
        let m = match_rule("promo diskon hari ini", &rules).unwrap();
        assert_eq!(m.keyword, "diskon");
    }

    #[test]
    fn rules_roundtrip_json() {
        let path = std::env::temp_dir().join("blastwa_test_rules.json");
        save_rules(&[rule(MatchType::Contains, "x")], &path).unwrap();
        let back = load_rules(&path).unwrap();
        assert_eq!(back.len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
