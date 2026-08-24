//! Opaque identifiers and counters used by the reservation ledger.

use std::fmt;
use std::num::ParseIntError;
use std::path::Component;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use uuid::Uuid;

macro_rules! uuid_identifier {
    ($name:ident) => {
        #[doc = concat!("An opaque UUID-v7 ", stringify!($name), ".")]
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(crate) struct $name(Uuid);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = InvalidUuidV7;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let identifier = value.parse::<Uuid>().map_err(InvalidUuidV7::InvalidUuid)?;
                if identifier.get_version_num() != 7 {
                    return Err(InvalidUuidV7::WrongVersion(identifier));
                }
                if identifier.get_variant() != uuid::Variant::RFC4122 {
                    return Err(InvalidUuidV7::WrongVariant(identifier));
                }
                Ok(Self(identifier))
            }
        }

        impl Serialize for $name {
            fn serialize<SerializerType>(
                &self,
                serializer: SerializerType,
            ) -> Result<SerializerType::Ok, SerializerType::Error>
            where
                SerializerType: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<DeserializerType>(
                deserializer: DeserializerType,
            ) -> Result<Self, DeserializerType::Error>
            where
                DeserializerType: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! numeric_identifier {
    ($name:ident, $primitive:ty, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
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

        impl From<$name> for $primitive {
            fn from(value: $name) -> Self { value.0 }
        }

        impl FromStr for $name {
            type Err = ParseIntError;

            fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(Self) }
        }
    };
}

uuid_identifier!(ReservationId);
uuid_identifier!(CoordinationRunId);
uuid_identifier!(EdgeId);
uuid_identifier!(EventId);
uuid_identifier!(RepoInstanceId);
uuid_identifier!(WorktreeId);

macro_rules! uuid_identifier_minter {
    (future $name:ident) => {
        impl $name {
            /// Mint a new non-recyclable identifier.
            #[cfg_attr(
                not(test),
                expect(
                    dead_code,
                    reason = "This UUID-v7 constructor belongs to a journal operation no verb constructs yet."
                )
            )]
            pub(crate) fn new() -> Self { Self(Uuid::now_v7()) }
        }
    };
    ($name:ident) => {
        impl $name {
            /// Mint a new non-recyclable identifier.
            pub(crate) fn new() -> Self { Self(Uuid::now_v7()) }
        }
    };
}

uuid_identifier_minter!(ReservationId);
uuid_identifier_minter!(CoordinationRunId);
uuid_identifier_minter!(EdgeId);
uuid_identifier_minter!(EventId);
uuid_identifier_minter!(RepoInstanceId);
uuid_identifier_minter!(WorktreeId);

numeric_identifier!(
    JournalByteOffset,
    u64,
    "A byte offset in the append-only journal."
);
numeric_identifier!(
    ProjectionGeneration,
    u64,
    "A projection cache generation counter."
);
numeric_identifier!(ReservationRevision, u64, "A reservation revision counter.");
numeric_identifier!(
    SchemaVersion,
    u32,
    "A journal or projection schema version."
);

/// The character count of a full SHA-1 object identifier.
const SHA1_OBJECT_ID_CHARACTERS: usize = 40;
/// The character count of a full SHA-256 object identifier.
const SHA256_OBJECT_ID_CHARACTERS: usize = 64;

/// A full lowercase hexadecimal git object identifier in either object format.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GitObjectId(String);

impl fmt::Display for GitObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for GitObjectId {
    type Err = InvalidGitObjectId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if matches!(
            value.len(),
            SHA1_OBJECT_ID_CHARACTERS | SHA256_OBJECT_ID_CHARACTERS
        ) && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidGitObjectId(value.to_owned()))
        }
    }
}

impl Serialize for GitObjectId {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for GitObjectId {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// A non-empty path whose components remain within the repository root.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ReservationScopePath(String);

impl fmt::Display for ReservationScopePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ReservationScopePath {
    type Err = InvalidReservationScopePath;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let path = Path::new(value);
        let has_windows_drive_prefix = value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && value.as_bytes().get(1) == Some(&b':');
        if !value.is_empty()
            && !path.is_absolute()
            && !path
                .components()
                .any(|component| matches!(component, Component::Prefix(_)))
            && !has_windows_drive_prefix
            && !value.contains('\\')
            && value.split('/').all(|component| {
                !component.is_empty()
                    && component != "."
                    && component != ".."
                    && !component.eq_ignore_ascii_case(".git")
            })
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidReservationScopePath(value.to_owned()))
        }
    }
}

impl Serialize for ReservationScopePath {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ReservationScopePath {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// An opaque, non-empty phase label supplied by a work-plan integration.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkPlanPhase(String);

impl fmt::Display for WorkPlanPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for WorkPlanPhase {
    type Err = InvalidWorkPlanPhase;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err(InvalidWorkPlanPhase)
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}

impl Serialize for WorkPlanPhase {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for WorkPlanPhase {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// An RFC 3339 UTC timestamp with millisecond precision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedAt(String);

impl RecordedAt {
    /// Capture the current UTC time in the journal's stable wire representation.
    pub(crate) fn now() -> Self {
        const SECONDS_PER_DAY: u64 = 86_400;
        const SECONDS_PER_HOUR: u64 = 3_600;
        const SECONDS_PER_MINUTE: u64 = 60;

        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        let days_since_epoch = elapsed.as_secs() / SECONDS_PER_DAY;
        let seconds_since_midnight = elapsed.as_secs() % SECONDS_PER_DAY;
        let (year, month, day) =
            civil_date_from_unix_days(i64::try_from(days_since_epoch).unwrap_or(i64::MAX));
        let hour = seconds_since_midnight / SECONDS_PER_HOUR;
        let minute = (seconds_since_midnight % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
        let second = seconds_since_midnight % SECONDS_PER_MINUTE;
        Self(format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
            elapsed.subsec_millis()
        ))
    }
}

impl fmt::Display for RecordedAt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for RecordedAt {
    type Err = InvalidRecordedAt;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if is_rfc3339_utc_milliseconds(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidRecordedAt(value.to_owned()))
        }
    }
}

impl Serialize for RecordedAt {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RecordedAt {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

fn is_rfc3339_utc_milliseconds(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
    {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| {
        matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 23) || byte.is_ascii_digit()
    })
}

const fn civil_date_from_unix_days(days_since_epoch: i64) -> (i64, i64, i64) {
    const DAYS_TO_CIVIL_EPOCH: i64 = 719_468;
    const DAYS_PER_400_YEAR_ERA: i64 = 146_097;
    const DAYS_PER_4_YEAR_ERA: i64 = 1_460;
    const DAYS_PER_CENTURY: i64 = 36_524;
    const DAYS_PER_YEAR: i64 = 365;
    const MONTH_NUMERATOR_MULTIPLIER: i64 = 5;
    const MONTH_NUMERATOR_OFFSET: i64 = 2;
    const DAYS_PER_MONTH_CYCLE: i64 = 153;
    const MARCH_MONTH_OFFSET: i64 = 3;
    const JANUARY_MONTH_OFFSET: i64 = -9;
    const MONTHS_BEFORE_JANUARY: i64 = 10;
    const YEARS_PER_CENTURY: i64 = 100;
    const YEARS_PER_ERA: i64 = 400;
    const YEARS_PER_LEAP_CYCLE: i64 = 4;

    let shifted_days = days_since_epoch.saturating_add(DAYS_TO_CIVIL_EPOCH);
    let era = if shifted_days >= 0 {
        shifted_days / DAYS_PER_400_YEAR_ERA
    } else {
        (shifted_days - (DAYS_PER_400_YEAR_ERA - 1)) / DAYS_PER_400_YEAR_ERA
    };
    let day_of_era = shifted_days - (era * DAYS_PER_400_YEAR_ERA);
    let year_of_era = (day_of_era - (day_of_era / DAYS_PER_4_YEAR_ERA)
        + (day_of_era / DAYS_PER_CENTURY)
        - (day_of_era / (DAYS_PER_400_YEAR_ERA - 1)))
        / DAYS_PER_YEAR;
    let year = year_of_era + (era * YEARS_PER_ERA);
    let day_of_year = day_of_era
        - (DAYS_PER_YEAR * year_of_era + (year_of_era / YEARS_PER_LEAP_CYCLE)
            - (year_of_era / YEARS_PER_CENTURY));
    let month_position =
        (MONTH_NUMERATOR_MULTIPLIER * day_of_year + MONTH_NUMERATOR_OFFSET) / DAYS_PER_MONTH_CYCLE;
    let day = day_of_year
        - ((DAYS_PER_MONTH_CYCLE * month_position + MONTH_NUMERATOR_OFFSET)
            / MONTH_NUMERATOR_MULTIPLIER)
        + 1;
    let month = if month_position < MONTHS_BEFORE_JANUARY {
        month_position + MARCH_MONTH_OFFSET
    } else {
        month_position + JANUARY_MONTH_OFFSET
    };
    let completed_year = if month <= 2 { year + 1 } else { year };
    (completed_year, month, day)
}

/// The immutable role of a worktree in its repository.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorktreeKind {
    /// The worktree associated with the common git directory.
    Main,
    /// A worktree attached through git's linked-worktree mechanism.
    Linked,
}

/// An error returned when text does not identify a UUID-v7 value.
#[derive(Debug)]
pub(crate) enum InvalidUuidV7 {
    /// The text is not any UUID.
    InvalidUuid(uuid::Error),
    /// The UUID has a version other than seven.
    WrongVersion(Uuid),
    /// The UUID does not use the RFC 4122 variant bits.
    WrongVariant(Uuid),
}

impl fmt::Display for InvalidUuidV7 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUuid(error) => write!(formatter, "invalid UUID: {error}"),
            Self::WrongVersion(identifier) => {
                write!(formatter, "{identifier} is not a UUID-v7 identifier")
            },
            Self::WrongVariant(identifier) => {
                write!(
                    formatter,
                    "{identifier} is not an RFC 4122 UUID-v7 identifier"
                )
            },
        }
    }
}

impl std::error::Error for InvalidUuidV7 {}

/// An error returned when text is not a full lowercase hexadecimal git object identifier.
#[derive(Debug)]
pub(crate) struct InvalidGitObjectId(String);

impl fmt::Display for InvalidGitObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid lowercase full git object identifier: {}",
            self.0
        )
    }
}

impl std::error::Error for InvalidGitObjectId {}

/// An error returned when text cannot identify a path inside the repository.
#[derive(Debug)]
pub(crate) struct InvalidReservationScopePath(String);

impl fmt::Display for InvalidReservationScopePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid repository-relative reservation scope path: {}",
            self.0
        )
    }
}

impl std::error::Error for InvalidReservationScopePath {}

/// An error returned when a work-plan phase label is empty.
#[derive(Debug)]
pub(crate) struct InvalidWorkPlanPhase;

impl fmt::Display for InvalidWorkPlanPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("work-plan phase labels cannot be empty")
    }
}

impl std::error::Error for InvalidWorkPlanPhase {}

/// An error returned when text is not the journal's RFC 3339 timestamp representation.
#[derive(Debug)]
pub(crate) struct InvalidRecordedAt(String);

impl fmt::Display for InvalidRecordedAt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid RFC 3339 UTC millisecond timestamp: {}",
            self.0
        )
    }
}

impl std::error::Error for InvalidRecordedAt {}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::CoordinationRunId;
    use super::EdgeId;
    use super::EventId;
    use super::GitObjectId;
    use super::JournalByteOffset;
    use super::ProjectionGeneration;
    use super::RecordedAt;
    use super::RepoInstanceId;
    use super::ReservationId;
    use super::ReservationRevision;
    use super::ReservationScopePath;
    use super::SchemaVersion;
    use super::WorkPlanPhase;
    use super::WorktreeId;

    #[test]
    fn identifiers_round_trip_through_their_scalar_json_values() {
        let reservation_id = ReservationId::new();
        let coordination_run_id = CoordinationRunId::new();
        let edge_id = EdgeId::new();
        let event_id = EventId::new();
        let git_object_id = "0123456789abcdef0123456789abcdef01234567"
            .parse::<GitObjectId>()
            .expect("git object identifier should parse");
        let journal_byte_offset = JournalByteOffset::from(4_096);
        let repo_instance_id = RepoInstanceId::new();
        let recorded_at = "2026-08-23T17:34:54.123Z"
            .parse::<RecordedAt>()
            .expect("recorded timestamp should parse");
        let generation = ProjectionGeneration::from(1);
        let reservation_revision = ReservationRevision::from(1);
        let reservation_scope_path = "crates/cargo-berth"
            .parse::<ReservationScopePath>()
            .expect("reservation scope path should parse");
        let schema_version = SchemaVersion::from(1);
        let worktree_id = WorktreeId::new();
        let work_plan_phase = "3b"
            .parse::<WorkPlanPhase>()
            .expect("opaque work-plan phase should parse");

        assert_identifier_round_trip(&reservation_id, &SerializedForm::Quoted);
        assert_identifier_round_trip(&coordination_run_id, &SerializedForm::Quoted);
        assert_identifier_round_trip(&edge_id, &SerializedForm::Quoted);
        assert_identifier_round_trip(&event_id, &SerializedForm::Quoted);
        assert_identifier_round_trip(&git_object_id, &SerializedForm::Quoted);
        assert_identifier_round_trip(&journal_byte_offset, &SerializedForm::Bare);
        assert_identifier_round_trip(&repo_instance_id, &SerializedForm::Quoted);
        assert_identifier_round_trip(&recorded_at, &SerializedForm::Quoted);
        assert_identifier_round_trip(&generation, &SerializedForm::Bare);
        assert_identifier_round_trip(&reservation_revision, &SerializedForm::Bare);
        assert_identifier_round_trip(&reservation_scope_path, &SerializedForm::Quoted);
        assert_identifier_round_trip(&schema_version, &SerializedForm::Bare);
        assert_identifier_round_trip(&worktree_id, &SerializedForm::Quoted);
        assert_identifier_round_trip(&work_plan_phase, &SerializedForm::Quoted);
    }

    #[test]
    fn reservation_scope_paths_enforce_lexical_repository_boundaries() {
        for invalid_path in [
            "",
            ".",
            "crates/./cargo-berth",
            ".git/config",
            "crates/.GIT/config",
            "crates/../Cargo.toml",
            "/absolute/path",
            "crates//cargo-berth",
            "crates/cargo-berth/",
            "crates\\cargo-berth",
            "C:",
            "C:crates/cargo-berth",
        ] {
            assert!(invalid_path.parse::<ReservationScopePath>().is_err());
        }

        assert!(
            "files/that/do/not/exist/yet.rs"
                .parse::<ReservationScopePath>()
                .is_ok()
        );
    }

    #[test]
    fn git_object_identifiers_parse_in_both_repository_object_formats() {
        let sha1_object_id = "0123456789abcdef0123456789abcdef01234567";
        let sha256_object_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        assert!(sha1_object_id.parse::<GitObjectId>().is_ok());
        assert!(sha256_object_id.parse::<GitObjectId>().is_ok());
        assert!("0123456789abcdef".parse::<GitObjectId>().is_err());
        assert!(
            "0123456789ABCDEF0123456789abcdef01234567"
                .parse::<GitObjectId>()
                .is_err()
        );
    }

    #[test]
    fn arbitrary_text_cannot_deserialize_as_an_identifier() {
        assert!(serde_json::from_str::<ReservationId>("\"reservation\"").is_err());
        assert!(
            serde_json::from_str::<ReservationId>("\"01900a1b-2c3d-7e4f-0a5b-6c7d8e9f0a1b\"")
                .is_err()
        );
    }

    /// How an identifier is expected to appear in JSON.
    enum SerializedForm {
        /// A quoted JSON string.
        Quoted,
        /// A bare JSON number.
        Bare,
    }

    fn assert_identifier_round_trip<Identifier>(
        identifier: &Identifier,
        expected_form: &SerializedForm,
    ) where
        Identifier: std::fmt::Debug
            + std::fmt::Display
            + PartialEq
            + serde::de::DeserializeOwned
            + serde::Serialize,
    {
        let serialized_identifier = serde_json::to_string(identifier);

        assert!(
            serialized_identifier
                .as_ref()
                .is_ok_and(|serialized_identifier| {
                    match expected_form {
                        SerializedForm::Quoted => {
                            serialized_identifier == &format!("\"{identifier}\"")
                        },
                        SerializedForm::Bare => serialized_identifier == &identifier.to_string(),
                    }
                })
        );
        assert!(
            serialized_identifier
                .and_then(|serialized_identifier| serde_json::from_str::<Identifier>(
                    &serialized_identifier
                ))
                .is_ok_and(|round_tripped_identifier| round_tripped_identifier == *identifier)
        );
    }
}
