//! Versioned pure SQL values; domain expressions remain fixed package resources.
use rusqlite::{
    Connection,
    functions::{Context, FunctionFlags},
    types::ValueRef,
};
use sha2::{Digest, Sha256};

type TextOperation = fn(&str) -> rusqlite::Result<String>;

pub(crate) fn register(connection: &Connection) -> rusqlite::Result<()> {
    // No callback captures a connection, owner, process or mutable state.
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    let text_functions: [(&str, TextOperation); 5] = [
        ("data_text_trim_v1", |s| Ok(s.trim().to_owned())),
        ("data_text_collapse_whitespace_v1", |s| {
            Ok(s.split_whitespace().collect::<Vec<_>>().join(" "))
        }),
        ("data_text_lowercase_v1", |s| Ok(s.to_lowercase())),
        ("data_json_quote_v1", |s| {
            serde_json::to_string(s).map_err(|_| invalid("encoding"))
        }),
        ("data_sha256_v1", |s| {
            Ok(format!("{:x}", Sha256::digest(s.as_bytes())))
        }),
    ];
    for (name, operation) in text_functions {
        connection
            .create_scalar_function(name, 1, flags, move |context| operation(text(context, 0)?))?;
    }
    connection.create_scalar_function("data_text_char_count_v1", 1, flags, |context| {
        i64::try_from(text(context, 0)?.chars().count()).map_err(|_| invalid("integer-range"))
    })?;
    connection.create_scalar_function("data_text_prefix_v1", 2, flags, |context| {
        let count = match context.get_raw(1) {
            ValueRef::Integer(count) if count >= 0 => {
                usize::try_from(count).map_err(|_| invalid("integer-range"))?
            }
            _ => return Err(invalid("nonnegative-integer")),
        };
        Ok(text(context, 0)?.chars().take(count).collect::<String>())
    })?;
    connection.create_scalar_function("data_text_trim_end_matches_v1", 2, flags, |context| {
        let source = text(context, 0)?;
        let characters = text(context, 1)?;
        Ok(source
            .trim_end_matches(|ch| characters.contains(ch))
            .to_owned())
    })?;
    Ok(())
}

fn text<'a>(context: &'a Context<'_>, index: usize) -> rusqlite::Result<&'a str> {
    match context.get_raw(index) {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes).map_err(|_| invalid("utf8-text")),
        _ => Err(invalid("text")),
    }
}

fn invalid(expected: &'static str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("DATA_VALUE_FUNCTION_INVALID: expected {expected}"),
        )
        .into(),
    )
}

#[cfg(test)]
#[path = "value_functions/tests.rs"]
mod tests;
