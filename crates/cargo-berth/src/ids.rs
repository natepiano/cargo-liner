//! Identifiers and counters used by the reservation ledger.

use std::convert::Infallible;
use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

macro_rules! string_identifier {
    ($name:ident) => {
        #[doc = concat!("An opaque ", stringify!($name), " value.")]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(String);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self { Self(value) }
        }

        impl FromStr for $name {
            type Err = Infallible;

            fn from_str(value: &str) -> Result<Self, Self::Err> { Ok(Self(value.to_owned())) }
        }
    };
}

macro_rules! numeric_identifier {
    ($name:ident, $primitive:ty) => {
        #[doc = concat!("A ", stringify!($name), " counter value.")]
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name($primitive);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }

        impl From<$primitive> for $name {
            fn from(value: $primitive) -> Self { Self(value) }
        }

        impl FromStr for $name {
            type Err = ParseIntError;

            fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(Self) }
        }
    };
}

string_identifier!(ReservationId);
string_identifier!(CoordinationRunId);
string_identifier!(EdgeId);
string_identifier!(EventId);
string_identifier!(WorktreeId);

numeric_identifier!(Generation, u64);
numeric_identifier!(SchemaVersion, u32);

#[cfg(test)]
mod tests {
    use super::CoordinationRunId;
    use super::EdgeId;
    use super::EventId;
    use super::Generation;
    use super::ReservationId;
    use super::SchemaVersion;
    use super::WorktreeId;

    #[test]
    fn identifiers_round_trip_through_their_scalar_json_values() {
        let reservation_id = ReservationId::from("reservation".to_owned());
        let coordination_run_id = CoordinationRunId::from("run".to_owned());
        let edge_id = EdgeId::from("edge".to_owned());
        let event_id = EventId::from("event".to_owned());
        let generation = Generation::from(1);
        let schema_version = SchemaVersion::from(1);
        let worktree_id = WorktreeId::from("worktree".to_owned());

        assert_identifier_round_trip(&reservation_id, "\"reservation\"");
        assert_identifier_round_trip(&coordination_run_id, "\"run\"");
        assert_identifier_round_trip(&edge_id, "\"edge\"");
        assert_identifier_round_trip(&event_id, "\"event\"");
        assert_identifier_round_trip(&generation, "1");
        assert_identifier_round_trip(&schema_version, "1");
        assert_identifier_round_trip(&worktree_id, "\"worktree\"");
    }

    fn assert_identifier_round_trip<Identifier>(identifier: &Identifier, scalar_json: &str)
    where
        Identifier: std::fmt::Debug
            + std::fmt::Display
            + PartialEq
            + serde::de::DeserializeOwned
            + serde::Serialize,
    {
        let serialized_identifier = serde_json::to_string(&identifier);

        assert_eq!(identifier.to_string(), scalar_json.trim_matches('"'));
        assert!(
            serialized_identifier
                .as_ref()
                .is_ok_and(|serialized_identifier| serialized_identifier == scalar_json)
        );
        assert!(
            serialized_identifier
                .and_then(|serialized_identifier| {
                    serde_json::from_str::<Identifier>(&serialized_identifier)
                })
                .is_ok_and(|round_tripped_identifier| round_tripped_identifier == *identifier)
        );
    }
}
