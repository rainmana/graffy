//! Lexicographically sortable identifiers (ULID) for every durable object.

use std::fmt;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub String);

        impl $name {
            /// Generate a fresh, time-sortable id.
            pub fn generate() -> Self {
                Self(format!(concat!($prefix, "_{}"), ulid::Ulid::new()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// A graph definition — durable, shareable.
    GraphId,
    "gph"
);
id_type!(
    /// One execution of a graph.
    RunId,
    "run"
);
id_type!(
    /// A coordination session (may span many runs).
    SessionId,
    "ses"
);
id_type!(
    /// An Information Unit in the MCW ledger.
    IuId,
    "iu"
);
id_type!(
    /// An evidence artifact backing a claim.
    EvidenceId,
    "evd"
);

#[cfg(test)]
mod tests {
    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = super::RunId::generate();
        let b = super::RunId::generate();
        assert!(a.0.starts_with("run_"));
        assert_ne!(a, b);
    }
}
