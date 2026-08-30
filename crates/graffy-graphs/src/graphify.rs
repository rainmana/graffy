//! Graphification — the founding invariant made operational: a skill or raw
//! prompt is never executed as text. It is compiled into a durable graph
//! whose shape supplies what raw execution never can: grounding before
//! drafting, verification before responding, budgets, and a journal.
//!
//! v1 ships **auto-adopt** — the low-oversight mode: the skill's own words
//! become the system knowledge of an `apply` node inside the verified
//! conversation floor (`intake → ground → apply → verify → respond`, with
//! the guarded revise loop). Guided and collaborative modes arrive with the
//! TUI conversion flows; the mode enum exists now so specs and CLI stay
//! stable when they do.

use graffy_core::spec::{EdgeSpec, GraphMeta, GraphSpec, NodeSpec, PolicySpec};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphifyError {
    #[error("input is empty — nothing to graphify")]
    Empty,
}

/// User-involvement modes from the founding design. Only Auto executes in
/// v1; the others are named so CLIs and specs need no breaking change later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Existing patterns absorb the skill with little oversight.
    Auto,
    /// The user checkpoints outcomes (TUI flow — not yet implemented).
    Guided,
    /// Node-by-node co-design (TUI flow — not yet implemented).
    Collaborative,
}

impl Mode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "guided" => Some(Self::Guided),
            "collaborative" => Some(Self::Collaborative),
            _ => None,
        }
    }
}

/// A parsed skill document (SKILL.md convention: optional `---` frontmatter
/// with `name:` / `description:`, then the body as usage knowledge).
#[derive(Debug, Clone)]
pub struct SkillDoc {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Parse a SKILL.md-style document. Hand-rolled on purpose: the convention
/// is a handful of `key: value` lines between `---` fences, and pulling a
/// YAML engine in for that would be dependency theater.
pub fn parse_skill_md(markdown: &str, fallback_name: &str) -> Result<SkillDoc, GraphifyError> {
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        return Err(GraphifyError::Empty);
    }

    let mut name = String::new();
    let mut description = String::new();
    let mut body = trimmed.to_owned();

    if let Some(rest) = trimmed.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        {
            let frontmatter = &rest[..end];
            body = rest[end + 4..].trim().to_owned();
            for line in frontmatter.lines() {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let value = value.trim().trim_matches('"').trim_matches('\'');
                match key.trim() {
                    "name" => name = value.to_owned(),
                    "description" => description = value.to_owned(),
                    _ => {}
                }
            }
        }
    }
    if name.is_empty() {
        // Fall back to the first `# Heading`, then to the file name.
        name = body
            .lines()
            .find_map(|l| l.strip_prefix("# ").map(str::to_owned))
            .unwrap_or_else(|| fallback_name.to_owned());
    }
    if body.is_empty() {
        return Err(GraphifyError::Empty);
    }
    Ok(SkillDoc {
        name,
        description,
        body,
    })
}

fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_owned()
}

fn param(value: &str) -> toml::Value {
    toml::Value::String(value.to_owned())
}

fn conversation_floor(
    graph_id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    apply_system: String,
) -> GraphSpec {
    let mut apply_params = toml::Table::new();
    apply_params.insert("system".into(), param(&apply_system));

    GraphSpec {
        graph: GraphMeta {
            id: graph_id,
            name,
            version: "0.1.0".into(),
            description,
            license: Some("GPL-3.0-or-later".into()),
            authors: vec!["graphified by graffy (auto-adopt)".into()],
            tags,
        },
        nodes: vec![
            NodeSpec {
                id: "intake".into(),
                kind: "intake".into(),
                description: "Decompose the request into Information Units.".into(),
                model_tier: None,
                params: toml::Table::new(),
            },
            NodeSpec {
                id: "ground".into(),
                kind: "research".into(),
                description: "Collect evidence the skill's claims will need.".into(),
                model_tier: None,
                params: toml::Table::new(),
            },
            NodeSpec {
                id: "apply".into(),
                kind: "model".into(),
                description: "Execute the imported knowledge inside the governed floor.".into(),
                model_tier: Some("balanced".into()),
                params: apply_params,
            },
            NodeSpec {
                id: "verify".into(),
                kind: "verify".into(),
                description: "Peer-review gate; failures escalate up the routing ladder.".into(),
                model_tier: Some("balanced".into()),
                params: toml::Table::new(),
            },
            NodeSpec {
                id: "respond".into(),
                kind: "respond".into(),
                description: "Deliver the verified result.".into(),
                model_tier: None,
                params: toml::Table::new(),
            },
        ],
        edges: vec![
            EdgeSpec {
                from: "intake".into(),
                to: "ground".into(),
                when: None,
            },
            EdgeSpec {
                from: "ground".into(),
                to: "apply".into(),
                when: None,
            },
            EdgeSpec {
                from: "apply".into(),
                to: "verify".into(),
                when: None,
            },
            EdgeSpec {
                from: "verify".into(),
                to: "respond".into(),
                when: Some("verdict == 'pass'".into()),
            },
            EdgeSpec {
                from: "verify".into(),
                to: "apply".into(),
                when: Some("verdict == 'revise'".into()),
            },
        ],
        policy: PolicySpec::default(),
    }
}

/// Auto-adopt a skill document into a durable graph.
pub fn graphify_skill(doc: &SkillDoc) -> GraphSpec {
    let slug = sanitize(&doc.name);
    let apply_system = format!(
        "You are executing the imported skill '{}' inside a graffy graph. The skill's own \
         instructions follow; apply them to the user's goal, grounding every claim in the \
         evidence gathered upstream, and never assert beyond it.\n\n--- SKILL ---\n{}",
        doc.name, doc.body
    );
    conversation_floor(
        format!("graffy.skill.{slug}"),
        doc.name.clone(),
        if doc.description.is_empty() {
            format!("Graphified skill '{}' (auto-adopt).", doc.name)
        } else {
            doc.description.clone()
        },
        vec!["skill-import".into(), "graphified".into(), "auto".into()],
        apply_system,
    )
}

/// Auto-adopt a raw prompt into a durable graph.
pub fn graphify_prompt(name: &str, prompt_text: &str) -> Result<GraphSpec, GraphifyError> {
    let text = prompt_text.trim();
    if text.is_empty() {
        return Err(GraphifyError::Empty);
    }
    let slug = sanitize(name);
    let apply_system = format!(
        "You are executing an imported prompt inside a graffy graph — never as raw text. \
         The prompt follows; honor its intent for the user's goal while grounding every \
         claim in the evidence gathered upstream.\n\n--- PROMPT ---\n{text}"
    );
    Ok(conversation_floor(
        format!("graffy.prompt.{slug}"),
        name.to_owned(),
        format!("Graphified prompt '{name}' (auto-adopt)."),
        vec!["prompt-import".into(), "graphified".into(), "auto".into()],
        apply_system,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use graffy_core::graph::CompiledGraph;

    const SKILL_WITH_FRONTMATTER: &str = r#"---
name: Incident Postmortem Writer
description: Writes blameless postmortems from incident notes.
---

# Incident Postmortem Writer

Given raw incident notes, produce a blameless postmortem with: timeline,
impact, contributing factors, and concrete follow-ups. Never speculate about
intent; quote the notes for every factual claim.
"#;

    #[test]
    fn skill_md_frontmatter_parses() {
        let doc = parse_skill_md(SKILL_WITH_FRONTMATTER, "fallback").unwrap();
        assert_eq!(doc.name, "Incident Postmortem Writer");
        assert!(doc.description.contains("blameless"));
        assert!(doc.body.contains("Never speculate"));
    }

    #[test]
    fn skill_md_without_frontmatter_falls_back_to_heading_then_filename() {
        let doc = parse_skill_md("# My Heading\n\nDo the thing.", "file-name").unwrap();
        assert_eq!(doc.name, "My Heading");
        let doc2 = parse_skill_md("Just body text.", "file-name").unwrap();
        assert_eq!(doc2.name, "file-name");
        assert!(parse_skill_md("   ", "x").is_err());
    }

    #[test]
    fn graphified_skill_compiles_and_carries_the_skill_text() {
        let doc = parse_skill_md(SKILL_WITH_FRONTMATTER, "fallback").unwrap();
        let spec = graphify_skill(&doc);
        assert_eq!(spec.graph.id, "graffy.skill.incident-postmortem-writer");
        let toml_text = spec.to_toml_string().unwrap();
        let reparsed = graffy_core::spec::GraphSpec::from_toml_str(&toml_text).unwrap();
        CompiledGraph::compile(&reparsed).expect("graphified skill must compile");
        let apply = reparsed.nodes.iter().find(|n| n.id == "apply").unwrap();
        let system = apply.params.get("system").and_then(|v| v.as_str()).unwrap();
        assert!(
            system.contains("Never speculate"),
            "skill text fronts the apply node"
        );
    }

    #[test]
    fn graphified_prompt_compiles() {
        let spec = graphify_prompt("Daily Standup", "Summarize my day as a standup.").unwrap();
        assert_eq!(spec.graph.id, "graffy.prompt.daily-standup");
        let reparsed =
            graffy_core::spec::GraphSpec::from_toml_str(&spec.to_toml_string().unwrap()).unwrap();
        CompiledGraph::compile(&reparsed).expect("graphified prompt must compile");
        assert!(graphify_prompt("x", "   ").is_err());
    }

    #[test]
    fn modes_parse_and_only_auto_is_v1() {
        assert_eq!(Mode::parse("auto"), Some(Mode::Auto));
        assert_eq!(Mode::parse("GUIDED"), Some(Mode::Guided));
        assert_eq!(Mode::parse("collaborative"), Some(Mode::Collaborative));
        assert_eq!(Mode::parse("yolo"), None);
    }
}
