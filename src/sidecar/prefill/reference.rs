use nexusweave::replay::Choice;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Rule {
    WordSequenceV1 { aliases: Vec<String> },
}

pub(super) struct Reference {
    terms: Vec<Vec<String>>,
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Reference {
    pub fn parse(raw: &Value) -> Result<Self, &'static str> {
        let Rule::WordSequenceV1 { aliases } = serde_json::from_value(raw.clone())
            .map_err(|_| "Expected a word_sequence_v1 reference with text aliases.")?;
        if aliases.is_empty() || aliases.len() > 64 || aliases.iter().any(|s| s.len() > 1024) {
            return Err("Use 1 to 64 reference aliases of at most 1024 bytes each.");
        }
        let terms: Vec<_> = aliases.iter().map(|a| tokens(a)).collect();
        if terms.iter().any(Vec::is_empty) {
            return Err("Each reference alias must contain a word or number.");
        }
        Ok(Self { terms })
    }

    pub fn score(&self, choice: &Choice) -> Option<bool> {
        if choice.kind != "model_output" {
            return None;
        }
        let answer = tokens(choice.outcome.as_str()?);
        Some(
            self.terms
                .iter()
                .any(|alias| answer.windows(alias.len()).any(|part| part == alias)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checks_whole_word_sequences_in_the_full_text_output() {
        let reference =
            Reference::parse(&json!({"kind":"word_sequence_v1","aliases":["Café au lait","9"]}))
                .unwrap();
        let mut choice = Choice {
            kind: "model_output".into(),
            label: "Answer".into(),
            outcome: json!(format!("{}CAFÉ, au lait!", "words ".repeat(100))),
        };
        assert_eq!(reference.score(&choice), Some(true));
        choice.outcome = json!("19 café_au_lait");
        assert_eq!(reference.score(&choice), Some(false));
        choice.outcome = json!(9);
        assert_eq!(reference.score(&choice), None);
        choice.kind = "tool_call".into();
        choice.outcome = json!("9");
        assert_eq!(reference.score(&choice), None);
    }

    #[test]
    fn rejects_unusable_or_unbounded_rules() {
        for aliases in [
            json!([]),
            json!(["..."]),
            json!(["x".repeat(1025)]),
            json!(vec!["a"; 65]),
        ] {
            assert!(
                Reference::parse(&json!({"kind":"word_sequence_v1","aliases":aliases})).is_err()
            );
        }
    }
}
