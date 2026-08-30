//! Generated protobuf types for graffy (ADR-0003, ADR-0004).
//!
//! Source of truth lives at `src/protos/*.proto` in the repo root:
//! * [`mcw`] — Meta-Context Window framework constructs (Information Units,
//!   failure modes, repair operations, H/R/D/M observables, evidence).
//! * [`journal`] — the append-only, replayable run journal.

/// Meta-Context Window framework types (`graffy.mcw.v1`).
pub mod mcw {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/graffy.mcw.v1.rs"));
    }
}

/// Run journal types (`graffy.journal.v1`).
pub mod journal {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/graffy.journal.v1.rs"));
    }
}

pub use prost;
pub use prost_types;

/// Proto packages compiled into this build.
pub const PROTO_PACKAGES: &[&str] = &["graffy.mcw.v1", "graffy.journal.v1"];

#[cfg(test)]
mod tests {
    use prost::Message;

    #[test]
    fn iu_roundtrips_through_wire_format() {
        let iu = crate::mcw::v1::InformationUnit {
            id: "iu_TEST".to_owned(),
            kind: crate::mcw::v1::IuKind::Constraint as i32,
            payload_text: "answers must cite evidence artifacts".to_owned(),
            salience: Some(0.9),
            ..Default::default()
        };
        let bytes = iu.encode_to_vec();
        let decoded = crate::mcw::v1::InformationUnit::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded, iu);
    }

    #[test]
    fn journal_event_carries_mcw_payloads() {
        let event = crate::journal::v1::RunEvent {
            run_id: "run_TEST".to_owned(),
            seq: 1,
            event: Some(crate::journal::v1::run_event::Event::FailureRaised(
                crate::mcw::v1::FailureSignal {
                    id: "fs_TEST".to_owned(),
                    mode: crate::mcw::v1::FailureMode::Overcompression as i32,
                    confidence: Some(0.8),
                    early_signal: "summary dropped the L1 falsification condition".to_owned(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let bytes = event.encode_to_vec();
        let decoded = crate::journal::v1::RunEvent::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded, event);
    }
}
