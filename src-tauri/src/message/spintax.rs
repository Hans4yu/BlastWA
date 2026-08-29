// recursive {a|b|c} expander. nesting handled by always reducing the
// innermost group first, so "{a|{b|c}}" resolves correctly.
use rand::Rng;
use regex::Regex;

pub fn spin(text: &str) -> String {
    let re = Regex::new(r"\{([^{}]*)\}").expect("static regex");
    let mut out = text.to_string();
    let mut guard = 0;
    while re.is_match(&out) {
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                let choices: Vec<&str> = caps[1].split('|').collect();
                let mut rng = rand::thread_rng();
                choices[rng.gen_range(0..choices.len())].to_string()
            })
            .to_string();
        guard += 1;
        if guard > 100 {
            // pathological input protection, better safe than infinite
            break;
        }
    }
    out
}

pub fn preview_spins(text: &str, count: usize) -> Vec<String> {
    (0..count).map(|_| spin(text)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_two_groups() {
        for _ in 0..50 {
            let s = spin("{Hello|Hi} {world|there}");
            assert!(
                s == "Hello world"
                    || s == "Hello there"
                    || s == "Hi world"
                    || s == "Hi there",
                "unexpected: {s}"
            );
        }
    }

    #[test]
    fn nested_expands() {
        let s = spin("{a|{b|c}}");
        assert!(s == "a" || s == "b" || s == "c", "unexpected: {s}");
    }

    #[test]
    fn no_braces_unchanged() {
        assert_eq!(spin("plain message"), "plain message");
    }

    #[test]
    fn single_choice() {
        assert_eq!(spin("{only_one}"), "only_one");
    }

    #[test]
    fn no_panic_100_iterations() {
        let t = "{x|y|z} middle {1|2|3} tail";
        for _ in 0..100 {
            let _ = spin(t);
        }
    }
}
