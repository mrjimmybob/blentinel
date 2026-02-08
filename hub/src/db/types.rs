use std::borrow::Cow;

use sqlx::{
    Encode, Decode, Type, Sqlite,
    encode::IsNull,
};
use sqlx::sqlite::{
    SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef,
};

use common::models::ResourceType;

/// SQLx adapter for ResourceType.
///
/// This type exists ONLY to cross the domain → persistence boundary.
/// Domain code must never depend on SQLx.
#[derive(Debug, Clone, Copy)]
pub struct DbResourceType(pub ResourceType);

impl Type<Sqlite> for DbResourceType {
    fn type_info() -> SqliteTypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'q> Encode<'q, Sqlite> for DbResourceType {
    fn encode_by_ref(
        &self,
        args: &mut Vec<SqliteArgumentValue<'q>>,
    ) -> Result<IsNull, Box<dyn std::error::Error + Send + Sync>> {
        args.push(SqliteArgumentValue::Text(
            Cow::Borrowed(self.0.as_str()),
        ));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, Sqlite> for DbResourceType {
    fn decode(
        value: SqliteValueRef<'r>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let s = <&str as Decode<Sqlite>>::decode(value)?;
        Ok(DbResourceType(ResourceType::try_from(s)?))
    }
}
