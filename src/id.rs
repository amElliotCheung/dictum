//! Canonical UUIDv7 identities shared by storage, MCP and HTTP.

use rmcp::schemars::{self, JsonSchema, Schema, SchemaGenerator};
use rusqlite::{
    Connection,
    types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Value, ValueRef},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{borrow::Cow, fmt, str::FromStr};

/// A lowercase, hyphenated RFC 9562 UUIDv7. All persisted identities use this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(uuid::Uuid);

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("expected a canonical lowercase UUIDv7")]
pub struct InvalidId;

impl FromStr for Id {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let uuid = uuid::Uuid::parse_str(value).map_err(|_| InvalidId)?;
        if uuid.get_version_num() != 7
            || uuid.get_variant() != uuid::Variant::RFC4122
            || uuid.to_string() != value
        {
            return Err(InvalidId);
        }
        Ok(Self(uuid))
    }
}

impl fmt::Display for Id {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Id {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "UuidV7".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "uuid",
            "minLength": 36,
            "maxLength": 36,
            "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
        })
    }
}

impl ToSql for Id {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Text(self.to_string())))
    }
}

impl FromSql for Id {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str()?.parse().map_err(FromSqlError::other)
    }
}

/// Generate an identity while holding the database's immediate write transaction.
/// SQLite has no built-in UUIDv7 generator, so every entity uses this one function.
/// Compare against persisted identities to preserve append order across processes,
/// restarts and clock regressions, including multiple writes in one millisecond.
pub(crate) fn new_id(connection: &Connection) -> rusqlite::Result<Id> {
    let candidate = Id(uuid::Uuid::now_v7());
    let previous: Option<Id> = connection.query_row(
        "SELECT max(id) FROM (
            SELECT max(id) AS id FROM projects
            UNION ALL SELECT max(id) FROM topics
            UNION ALL SELECT max(id) FROM tasks
            UNION ALL SELECT max(id) FROM sources
            UNION ALL SELECT max(id) FROM decisions
            UNION ALL SELECT max(id) FROM task_comments
        )",
        [],
        |row| row.get(0),
    )?;
    let Some(previous) = previous.filter(|previous| candidate <= *previous) else {
        return Ok(candidate);
    };

    // Increment the 74 variable suffix bits, skipping the fixed version/variant.
    // Carry into the 48-bit millisecond field only after exhausting that suffix.
    let raw = previous.0.as_u128();
    let mut milliseconds = raw >> 80;
    let rand_b_mask = (1_u128 << 62) - 1;
    let mut suffix = (((raw >> 64) & 0xfff) << 62) | (raw & rand_b_mask);
    if suffix == (1_u128 << 74) - 1 {
        milliseconds += 1;
        suffix = 0;
        if milliseconds >= 1_u128 << 48 {
            return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
                std::io::Error::other("UUIDv7 timestamp space exhausted"),
            )));
        }
    } else {
        suffix += 1;
    }
    Ok(Id(uuid::Uuid::from_u128(
        (milliseconds << 80)
            | (7_u128 << 76)
            | ((suffix >> 62) << 64)
            | (2_u128 << 62)
            | (suffix & rand_b_mask),
    )))
}
