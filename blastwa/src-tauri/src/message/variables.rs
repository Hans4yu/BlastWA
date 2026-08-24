// template variable substitution + js escaping.
// placeholders mirror the original app exactly: [[fullname]] etc.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactRow {
    pub number: String,
    pub fullname: String,
    pub firstname: String,
    pub lastname: String,
    pub var1: String,
    pub var2: String,
    pub var3: String,
    pub var4: String,
    pub var5: String,
}

impl ContactRow {
    pub fn from_fullname(number: &str, fullname: &str) -> Self {
        let mut c = ContactRow {
            number: number.to_string(),
            fullname: fullname.to_string(),
            ..Default::default()
        };
        let mut parts = fullname.splitn(2, ' ');
        c.firstname = parts.next().unwrap_or("").to_string();
        c.lastname = parts.next().unwrap_or("").to_string();
        c
    }

    /// wa id like 6281234567890@c.us
    pub fn wa_id(&self) -> String {
        format!("{}@c.us", self.number)
    }
}

pub fn apply_variables(template: &str, contact: &ContactRow) -> String {
    let random_tag: u32 = rand::Rng::gen_range(&mut rand::thread_rng(), 1000..9999);
    template
        .replace("[[fullname]]", &contact.fullname)
        .replace("[[firstname]]", &contact.firstname)
        .replace("[[lastname]]", &contact.lastname)
        .replace("[[VAR1]]", &contact.var1)
        .replace("[[VAR2]]", &contact.var2)
        .replace("[[VAR3]]", &contact.var3)
        .replace("[[VAR4]]", &contact.var4)
        .replace("[[VAR5]]", &contact.var5)
        .replace("[[randomtag]]", &random_tag.to_string())
}

/// escape a rust string so it can be safely dropped inside single-quoted js
pub fn js_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ContactRow {
        ContactRow {
            number: "628123456789".into(),
            fullname: "Budi Santoso".into(),
            firstname: "Budi".into(),
            lastname: "Santoso".into(),
            var1: "Promo".into(),
            ..Default::default()
        }
    }

    #[test]
    fn all_placeholders_replaced() {
        let t = "[[firstname]] [[lastname]] aka [[fullname]] v=[[VAR1]] tag=[[randomtag]]";
        let out = apply_variables(t, &sample());
        assert!(out.starts_with("Budi Santoso aka Budi Santoso v=Promo tag="));
    }

    #[test]
    fn missing_vars_become_empty() {
        let mut c = sample();
        c.var3 = String::new();
        assert_eq!(apply_variables("X[[VAR3]]Y", &c), "XY");
    }

    #[test]
    fn random_tag_is_numeric() {
        let out = apply_variables("[[randomtag]]", &sample());
        assert_eq!(out.len(), 4);
        assert!(out.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn js_escape_quotes_and_newlines() {
        assert_eq!(js_escape("it's\nfine"), "it\\'s\\nfine");
        assert_eq!(js_escape("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn split_names() {
        let c = ContactRow::from_fullname("62", "Budi Santoso Jaya");
        assert_eq!(c.firstname, "Budi");
        assert_eq!(c.lastname, "Santoso Jaya");
    }
}
