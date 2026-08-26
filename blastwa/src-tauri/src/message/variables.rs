// template variable substitution + js escaping.
// placeholders mirror the original app exactly: [[fullname]] etc.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactRow {
    pub number: String,
    pub fullname: String,
    pub firstname: String,
    pub middlename: String,
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
        // first word is the first name, last word is the family name, and
        // everything between is the middle name. a two-word name therefore
        // has no middle name, and a single word has neither middle nor last.
        let words: Vec<&str> = fullname.split_whitespace().collect();
        match words.len() {
            0 => {}
            1 => c.firstname = words[0].to_string(),
            2 => {
                c.firstname = words[0].to_string();
                c.lastname = words[1].to_string();
            }
            _ => {
                c.firstname = words[0].to_string();
                c.middlename = words[1..words.len() - 1].join(" ");
                c.lastname = words[words.len() - 1].to_string();
            }
        }
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
        .replace("[[middlename]]", &contact.middlename)
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
    fn split_names_three_parts_fills_middle() {
        let c = ContactRow::from_fullname("62", "Muhammad Rizqy Hidayah");
        assert_eq!(c.firstname, "Muhammad");
        assert_eq!(c.middlename, "Rizqy");
        assert_eq!(c.lastname, "Hidayah");
    }

    #[test]
    fn split_names_two_parts_has_no_middle() {
        let c = ContactRow::from_fullname("62", "Dwi Anggoro");
        assert_eq!(c.firstname, "Dwi");
        assert_eq!(c.middlename, "");
        assert_eq!(c.lastname, "Anggoro");
    }

    #[test]
    fn split_names_four_parts_joins_middle() {
        let c = ContactRow::from_fullname("62", "Haidar Labib Izza Kif");
        assert_eq!(c.firstname, "Haidar");
        assert_eq!(c.middlename, "Labib Izza");
        assert_eq!(c.lastname, "Kif");
    }

    #[test]
    fn middlename_placeholder_replaced() {
        let c = ContactRow::from_fullname("62", "Muhammad Rizqy Hidayah");
        assert_eq!(
            apply_variables("[[firstname]]|[[middlename]]|[[lastname]]", &c),
            "Muhammad|Rizqy|Hidayah"
        );
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
        assert_eq!(c.middlename, "Santoso");
        assert_eq!(c.lastname, "Jaya");
    }
}
