//! Append-only run journal (ADR-0003) — Phase 1 milestone M1.
//!
//! On disk: length-delimited `graffy.journal.v1.RunEvent` protobuf frames.
//! A journal file is a self-contained, portable record of everything a run
//! did: node transitions, model + routing calls, budgets, approvals, and the
//! full MCW stream (IUs, failure signals, repairs, H/R/D/M, evidence).
//!
//! Writers assign a strictly increasing `seq` per run and flush every frame —
//! a crash loses at most the frame being written. Readers replay by folding
//! the stream; `summarize` is the reference fold used by the CLI and tests.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use graffy_proto::prost::Message;

use crate::error::JournalError;

pub use graffy_proto::journal::v1 as wire;
use graffy_proto::journal::v1::run_event::Event;

/// Appends length-delimited `RunEvent` frames to a journal file.
pub struct JournalWriter {
    out: BufWriter<File>,
    path: PathBuf,
    run_id: String,
    seq: u64,
}

impl JournalWriter {
    /// Create (truncate) a journal at `path` for the given run.
    pub fn create(path: &Path, run_id: &str) -> Result<Self, JournalError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            out: BufWriter::new(file),
            path: path.to_path_buf(),
            run_id: run_id.to_owned(),
            seq: 0,
        })
    }

    /// Append one event; returns the assigned sequence number.
    pub fn append(&mut self, event: Event) -> Result<u64, JournalError> {
        self.seq += 1;
        let frame = wire::RunEvent {
            run_id: self.run_id.clone(),
            seq: self.seq,
            at: Some(crate::exec::now_ts()),
            event: Some(event),
        };
        let bytes = frame.encode_length_delimited_to_vec();
        self.out.write_all(&bytes)?;
        self.out.flush()?;
        Ok(self.seq)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

/// Reads a journal back into memory.
pub struct JournalReader;

impl JournalReader {
    /// Decode every frame in the file, in order.
    pub fn read_all(path: &Path) -> Result<Vec<wire::RunEvent>, JournalError> {
        let data = std::fs::read(path)?;
        let mut cursor = data.as_slice();
        let mut events = Vec::new();
        while !cursor.is_empty() {
            events.push(wire::RunEvent::decode_length_delimited(&mut cursor)?);
        }
        Ok(events)
    }
}

/// The reference fold over a journal stream.
#[derive(Debug, Default)]
pub struct RunSummary {
    pub run_id: String,
    pub graph_name: String,
    pub event_count: usize,
    pub status: Option<wire::RunStatus>,
    /// Final observed state per node id.
    pub node_states: BTreeMap<String, wire::NodeState>,
    pub iu_count: usize,
    pub evidence_count: usize,
    pub failure_signal_count: usize,
    pub repair_count: usize,
    pub model_calls: usize,
    pub routing_decisions: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_usd: f64,
}

/// Fold a journal stream into a [`RunSummary`].
pub fn summarize(events: &[wire::RunEvent]) -> RunSummary {
    let mut s = RunSummary {
        event_count: events.len(),
        ..Default::default()
    };
    for frame in events {
        if s.run_id.is_empty() {
            s.run_id = frame.run_id.clone();
        }
        match &frame.event {
            Some(Event::RunStarted(m)) => s.graph_name = m.graph_name.clone(),
            Some(Event::NodeTransition(t)) => {
                s.node_states
                    .insert(t.node_id.clone(), t.to.try_into().unwrap_or_default());
            }
            Some(Event::ModelCall(c)) => {
                s.model_calls += 1;
                s.total_input_tokens += c.input_tokens;
                s.total_output_tokens += c.output_tokens;
                s.total_usd += c.cost_usd;
            }
            Some(Event::RoutingDecision(_)) => s.routing_decisions += 1,
            Some(Event::IuRecorded(_)) => s.iu_count += 1,
            Some(Event::EvidenceRecorded(_)) => s.evidence_count += 1,
            Some(Event::FailureRaised(_)) => s.failure_signal_count += 1,
            Some(Event::RepairExecuted(_)) => s.repair_count += 1,
            Some(Event::RunFinished(f)) => {
                s.status = Some(f.status.try_into().unwrap_or_default());
            }
            _ => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_journal_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "graffy-journal-test-{tag}-{}",
            ulid::Ulid::generate()
        ))
    }

    #[test]
    fn frames_roundtrip_in_order() {
        let path = temp_journal_path("roundtrip");
        let mut w = JournalWriter::create(&path, "run_TEST").unwrap();
        w.append(Event::RunStarted(wire::RunManifest {
            run_id: "run_TEST".into(),
            graph_name: "t".into(),
            ..Default::default()
        }))
        .unwrap();
        w.append(Event::NodeTransition(wire::NodeTransition {
            node_id: "a".into(),
            from: wire::NodeState::Queued as i32,
            to: wire::NodeState::Running as i32,
            ..Default::default()
        }))
        .unwrap();
        w.append(Event::RunFinished(wire::RunFinished {
            status: wire::RunStatus::Succeeded as i32,
            ..Default::default()
        }))
        .unwrap();

        let events = JournalReader::read_all(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let summary = summarize(&events);
        assert_eq!(summary.status, Some(wire::RunStatus::Succeeded));
        assert_eq!(
            summary.node_states.get("a"),
            Some(&wire::NodeState::Running)
        );
        std::fs::remove_file(&path).ok();
    }
}
