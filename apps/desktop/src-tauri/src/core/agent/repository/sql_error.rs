//! Retain SQLite evidence until the owning boundary creates its safe diagnostic.
pub(crate) enum SqlRepositoryError<E> {
    Domain(E),
    Sql(rusqlite::Error),
}
impl<E> From<rusqlite::Error> for SqlRepositoryError<E> {
    fn from(cause: rusqlite::Error) -> Self {
        Self::Sql(cause)
    }
}
impl<E> SqlRepositoryError<E> {
    pub(crate) fn domain_or(self, unavailable: E) -> E {
        match self {
            Self::Domain(cause) => cause,
            Self::Sql(_) => unavailable,
        }
    }
}
