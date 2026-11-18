//! Times as SQLite stores them: milliseconds in a signed 64-bit integer.

use ren_recorder::time::UnixMillis;
use sqlx::{
    Database, Encode, Sqlite, Type, encode::IsNull, error::BoxDynError, sqlite::SqliteTypeInfo,
};
use std::time::Duration;

/// A time or duration in milliseconds, as a column holds it. A value past
/// `i64::MAX` saturates there, so binding one never fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SqlMillis(i64);

impl SqlMillis {
    /// Earlier than every stored time: a cursor that starts at the beginning.
    pub(crate) const BEFORE_EPOCH: Self = Self(-1);

    /// A wall-clock time.
    pub(crate) fn at(time: UnixMillis) -> Self {
        Self(i64::try_from(time.as_millis()).unwrap_or(i64::MAX))
    }

    /// A span of time, such as a monotonic clock's reading.
    pub(crate) fn span(duration: Duration) -> Self {
        Self(i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
    }
}

impl Type<Sqlite> for SqlMillis {
    fn type_info() -> SqliteTypeInfo {
        <i64 as Type<Sqlite>>::type_info()
    }

    fn compatible(ty: &SqliteTypeInfo) -> bool {
        <i64 as Type<Sqlite>>::compatible(ty)
    }
}

impl<'q> Encode<'q, Sqlite> for SqlMillis {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as Database>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <i64 as Encode<'q, Sqlite>>::encode_by_ref(&self.0, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::SqlMillis;
    use ren_recorder::time::UnixMillis;
    use std::time::Duration;

    #[test]
    fn a_time_past_the_column_saturates() {
        assert_eq!(SqlMillis::at(UnixMillis::from_millis(42)), SqlMillis(42));
        assert_eq!(
            SqlMillis::at(UnixMillis::from_millis(u64::MAX)),
            SqlMillis(i64::MAX)
        );
        assert_eq!(SqlMillis::span(Duration::from_secs(2)), SqlMillis(2000));
        assert_eq!(SqlMillis::span(Duration::MAX), SqlMillis(i64::MAX));
        assert!(SqlMillis::BEFORE_EPOCH < SqlMillis::at(UnixMillis::from_millis(0)));
    }
}
