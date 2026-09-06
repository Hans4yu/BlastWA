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
    /// case-insensitive: senders type "PROMO", "Promo", "promo" freely, and a
    /// keyword that only fires on one casing silently drops the rest
    pub fn matches(&self, message: &str, keyword: &str) -> bool {
        let msg = message.to_lowercase();
        let kw = keyword.to_lowercase();
        match self {
            MatchType::Like => msg == kw,
            MatchType::StartWith => msg.starts_with(&kw),
            MatchType::EndWith => msg.ends_with(&kw),
            MatchType::Contains => msg.contains(&kw),
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
    /// rules are enabled unless explicitly disabled: a bool with plain
    /// `#[serde(default)]` deserializes to false, which silently disabled
    /// every rule whose json predates this field
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// rule usable by the watcher: enabled, with a keyword AND a reply to send.
/// an empty keyword is catastrophic — `"".contains("")` is true, so the rule
/// would answer EVERY incoming message.
impl Rule {
    pub fn is_armed(&self) -> bool {
        self.enabled
            && !self.keyword.trim().is_empty()
            && !self.reply_message.as_deref().unwrap_or("").trim().is_empty()
    }
}

/// first matching enabled rule wins — mirrors original behavior.
/// the empty-keyword guard lives here (not just in save) because a legacy
/// file could still hand us one; a reply is NOT required to match — the
/// watcher skips reply-less rules at send time.
pub fn match_rule<'a>(message: &str, rules: &'a [Rule]) -> Option<&'a Rule> {
    rules.iter().find(|r| {
        r.enabled
            && !r.keyword.trim().is_empty()
            && r.match_type.matches(message, &r.keyword)
    })
}

/// persist rules, dropping half-written rows (no keyword or no reply text).
/// the watcher must never load a rule that would reply with an empty
/// message or match everything. returns the number of rules actually saved
/// so the UI can surface how many rows were skipped.
pub fn save_rules(rules: &[Rule], path: &std::path::Path) -> anyhow::Result<usize> {
    let armed: Vec<&Rule> = rules.iter().filter(|r| r.is_armed()).collect();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::config::settings::atomic_write(
        path,
        serde_json::to_string_pretty(&armed)?.as_bytes(),
    )?;
    Ok(armed.len())
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
    fn disabled_rule_never_matches() {
        let mut r = rule(MatchType::Contains, "promo");
        r.enabled = false;
        assert!(match_rule("ada promo?", &[r]).is_none());
    }

    #[test]
    fn enabled_defaults_to_true_when_absent_from_json() {
        // a rule file written before `enabled` existed must not load as a
        // silently disabled rule
        let parsed: Vec<Rule> = serde_json::from_str(
            r#"[{"name":"legacy","match_type":"Contains","keyword":"promo"}]"#,
        )
        .expect("legacy rule json should parse");
        assert!(parsed[0].enabled);
        assert!(match_rule("ada promo?", &parsed).is_some());
    }

    #[test]
    fn explicit_false_still_disables() {
        let parsed: Vec<Rule> = serde_json::from_str(
            r#"[{"name":"off","match_type":"Contains","keyword":"promo","enabled":false}]"#,
        )
        .unwrap();
        assert!(!parsed[0].enabled);
        assert!(match_rule("ada promo?", &parsed).is_none());
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
    fn keyword_matching_is_case_insensitive() {
        assert!(match_rule("PROMO pgi bos", &[rule(MatchType::Contains, "promo")]).is_some());
        assert!(match_rule("halo BOS", &[rule(MatchType::Like, "Halo Bos")]).is_some());
        assert!(match_rule("MENU utama", &[rule(MatchType::StartWith, "menu")]).is_some());
        assert!(match_rule("mulai dari MENU", &[rule(MatchType::StartWith, "menu")]).is_none());
    }

    #[test]
    fn empty_keyword_never_matches_anything() {
        // contains("") is true for every message; an empty keyword must be
        // inert or the rule would auto-reply to the entire inbox
        assert!(match_rule("anything", &[rule(MatchType::Contains, "")]).is_none());
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
        let saved = save_rules(&[rule(MatchType::Contains, "x")], &path).unwrap();
        assert_eq!(saved, 1);
        let back = load_rules(&path).unwrap();
        assert_eq!(back.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_drops_rows_without_keyword_or_reply() {
        let path = std::env::temp_dir().join("blastwa_test_rules_filter.json");
        let mut no_reply = rule(MatchType::Contains, "hi");
        no_reply.reply_message = None;
        let no_keyword = rule(MatchType::Contains, "");
        let saved = save_rules(&[no_reply, no_keyword, rule(MatchType::Contains, "ok")], &path).unwrap();
        assert_eq!(saved, 1, "only the fully armed rule may be persisted");
        let back = load_rules(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].keyword, "ok");
        let _ = std::fs::remove_file(&path);
    }
}
