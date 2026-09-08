//! Offline readiness algebra over caller-reviewed evidence, not NLP entailment.
use std::collections::BTreeSet;

#[derive(Debug, PartialEq, Eq)]
pub enum Readiness {
    NeedsMoreEvidence,
    ConflictingEvidence,
    SupportedInvestigation,
}

/// Caller owns the assertion meaning and review of supporting/refuting sources.
pub struct Premise {
    pub id: String,
    pub missing_fact: Option<String>,
    pub supports: BTreeSet<String>,
    pub refutes: BTreeSet<String>,
}

/// No evidence is unknown, not false. Refutation takes precedence over missing
/// evidence. A successful result never authorizes domain execution.
pub fn assess(
    premises: &[Premise],
    reviewed_sources: &BTreeSet<String>,
) -> Result<Readiness, String> {
    let mut ids = BTreeSet::new();
    let mut missing = premises.is_empty();
    let mut refuted = false;
    for p in premises {
        if p.id.trim().is_empty() || !ids.insert(&p.id) {
            return Err("premise identity missing or duplicated".into());
        }
        if !p.supports.is_subset(reviewed_sources) || !p.refutes.is_subset(reviewed_sources) {
            return Err("evidence has not been admitted by caller review".into());
        }
        if !p.supports.is_disjoint(&p.refutes) {
            return Err(
                "split a source into reviewed spans before assigning opposing roles".into(),
            );
        }
        if p.missing_fact
            .as_deref()
            .is_some_and(|s| s.trim().is_empty())
        {
            return Err("missing fact must identify the absent fact".into());
        }
        if p.missing_fact.is_some() && (!p.supports.is_empty() || !p.refutes.is_empty()) {
            return Err("an atomic premise cannot be both missing and evidenced".into());
        }
        if p.missing_fact.is_none() && p.supports.is_empty() && p.refutes.is_empty() {
            return Err("missing premise must identify the absent fact".into());
        }
        missing |= p.missing_fact.is_some();
        refuted |= !p.refutes.is_empty();
    }
    Ok(if refuted {
        Readiness::ConflictingEvidence
    } else if missing {
        Readiness::NeedsMoreEvidence
    } else {
        Readiness::SupportedInvestigation
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn premise(id: &str, yes: &[&str], no: &[&str]) -> Premise {
        Premise {
            id: id.into(),
            missing_fact: (yes.is_empty() && no.is_empty()).then(|| format!("missing {id}")),
            supports: yes.iter().map(|s| s.to_string()).collect(),
            refutes: no.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn sources() -> BTreeSet<String> {
        ["a".into(), "b".into()].into()
    }

    #[test]
    fn missing_approval_or_zero_retrieval_never_becomes_refutation() {
        assert_eq!(
            assess(&[], &sources()).unwrap(),
            Readiness::NeedsMoreEvidence
        );
        assert_eq!(
            assess(&[premise("approved_master", &[], &[])], &sources()).unwrap(),
            Readiness::NeedsMoreEvidence
        );
        assert_eq!(
            assess(
                &[
                    premise("candidate_exists", &["a"], &[]),
                    premise("approval", &[], &[])
                ],
                &sources()
            )
            .unwrap(),
            Readiness::NeedsMoreEvidence
        );
    }
    #[test]
    fn explicit_counterevidence_survives_missing_facts_and_order() {
        for p in [
            vec![
                premise("codes_removed", &[], &["a"]),
                premise("cutoff", &[], &[]),
            ],
            vec![
                premise("cutoff", &[], &[]),
                premise("codes_removed", &[], &["a"]),
            ],
        ] {
            assert_eq!(
                assess(&p, &sources()).unwrap(),
                Readiness::ConflictingEvidence
            );
        }
        assert_eq!(
            assess(&[premise("claim", &["a"], &["b"])], &sources()).unwrap(),
            Readiness::ConflictingEvidence
        );
    }
    #[test]
    fn adding_support_cannot_erase_refutation() {
        assert_eq!(
            assess(&[premise("investigation", &["a"], &[])], &sources()).unwrap(),
            Readiness::SupportedInvestigation
        );
        assert_eq!(
            assess(&[premise("investigation", &["a"], &["b"])], &sources()).unwrap(),
            Readiness::ConflictingEvidence
        );
    }
    #[test]
    fn unreviewed_and_ambiguous_witnesses_are_not_admitted() {
        for p in [
            vec![premise("x", &[], &["unreviewed"])],
            vec![premise("x", &["a"], &["a"])],
            vec![premise("x", &["a"], &[]), premise("x", &["b"], &[])],
            vec![Premise {
                id: "x".into(),
                missing_fact: Some(" ".into()),
                supports: BTreeSet::new(),
                refutes: BTreeSet::new(),
            }],
        ] {
            assert!(assess(&p, &sources()).is_err());
        }
    }
}
