//! Original compile SQL settlement and row codecs. Pure projection is Native-owned.
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use uuid::Uuid;

use super::resources::Sql;
use crate::agent::writing_compile_contracts::{CompileArtifact, CompileDiagnostic, CompileRun};
use crate::agent::writing_error::WritingError;

pub(crate) struct CompileCompletion<'a> {
    pub(crate) status: &'a str,
    pub(crate) input_hash: &'a str,
    pub(crate) toolchain_json: &'a str,
    pub(crate) raw_output: &'a str,
    pub(crate) exit_code: Option<i32>,
    pub(crate) diagnostics: &'a [CompileDiagnostic],
    pub(crate) pdf: Option<StoredPdf>,
    pub(crate) log: Option<CacheArtifact>,
    pub(crate) synctex: Option<CacheArtifact>,
}

pub(crate) use crate::agent::writing_compile_contracts::SynctexRunContext;

pub(crate) struct RunInputContext {
    pub(crate) manuscript_id: String,
    pub(crate) input_hash: String,
}

pub(crate) struct StoredPdf {
    pub(crate) sha256: String,
    pub(crate) object_relative_path: String,
    pub(crate) byte_size: u64,
}

pub(crate) struct CacheArtifact {
    pub(crate) relative_path: String,
    pub(crate) byte_size: u64,
}

pub(crate) fn create_run(
    sql: &Sql,
    connection: &Connection,
    manuscript_id: &str,
    engine: &str,
) -> Result<CompileRun, WritingError> {
    let root_path = connection
        .query_row(
            sql.get("project-compile-root.v1.sql"),
            [manuscript_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(WritingError::ManuscriptNotFound)?;
    let id = Uuid::new_v4().to_string();
    if let Err(error) = connection.execute(
        sql.get("project-compile-create.v1.sql"),
        params![id, manuscript_id, engine, root_path],
    ) {
        if matches!(
            &error,
            rusqlite::Error::SqliteFailure(value, _)
                if value.code == ErrorCode::ConstraintViolation
        ) {
            return Err(WritingError::CompileAlreadyRunning);
        }
        return Err(error.into());
    }
    get_run(sql, connection, &id)
}

pub(crate) fn mark_running(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<(), WritingError> {
    let changed = connection.execute(sql.get("project-compile-running.v1.sql"), [run_id])?;
    if changed == 0 {
        return Err(WritingError::CompileRunNotFound);
    }
    Ok(())
}

pub(crate) fn request_cancellation(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<(), WritingError> {
    let changed = connection.execute(sql.get("project-compile-cancel.v1.sql"), [run_id])?;
    if changed == 0 {
        return Err(WritingError::CompileRunNotFound);
    }
    Ok(())
}

pub(crate) fn finish_run(
    sql: &Sql,
    connection: &mut Connection,
    run_id: &str,
    completion: CompileCompletion<'_>,
) -> Result<(), WritingError> {
    let transaction = connection.transaction()?;
    transaction.execute(
        sql.get("project-compile-finish.v1.sql"),
        params![
            run_id,
            completion.status,
            completion.input_hash,
            completion.toolchain_json,
            completion.raw_output,
            completion.exit_code,
        ],
    )?;
    for (position, diagnostic) in completion.diagnostics.iter().enumerate() {
        transaction.execute(
            sql.get("project-compile-diagnostic-insert.v1.sql"),
            params![
                Uuid::new_v4().to_string(),
                run_id,
                diagnostic.severity,
                diagnostic.message,
                diagnostic.file,
                diagnostic.line,
                diagnostic.column,
                position as i64,
            ],
        )?;
    }
    if let Some(pdf) = completion.pdf {
        transaction.execute(
            sql.get("project-compile-object-insert.v1.sql"),
            params![pdf.sha256, pdf.object_relative_path, pdf.byte_size as i64],
        )?;
        insert_artifact(
            sql,
            &transaction,
            run_id,
            "pdf",
            Some(&pdf.sha256),
            None,
            pdf.byte_size,
        )?;
        transaction.execute(sql.get("project-compile-last-success.v1.sql"), [run_id])?;
    }
    if let Some(log) = completion.log {
        insert_artifact(
            sql,
            &transaction,
            run_id,
            "log",
            None,
            Some(&log.relative_path),
            log.byte_size,
        )?;
    }
    if let Some(synctex) = completion.synctex {
        insert_artifact(
            sql,
            &transaction,
            run_id,
            "synctex",
            None,
            Some(&synctex.relative_path),
            synctex.byte_size,
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn fail_internal(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
    message: &str,
) -> Result<(), WritingError> {
    connection.execute(
        sql.get("project-compile-fail.v1.sql"),
        params![run_id, message.chars().take(4000).collect::<String>()],
    )?;
    Ok(())
}

pub(crate) fn get_run(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<CompileRun, WritingError> {
    let mut run = connection
        .query_row(sql.get("project-compile-get.v1.sql"), [run_id], |row| {
            let status: String = row.get(3)?;
            Ok(CompileRun {
                id: row.get(0)?,
                manuscript_id: row.get(1)?,
                engine: row.get(2)?,
                status,
                diagnostics: Vec::new(),
                artifacts: Vec::new(),
                raw_output: row.get(4)?,
                exit_code: row.get(5)?,
                started_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })
        .optional()?
        .ok_or(WritingError::CompileRunNotFound)?;
    run.diagnostics = read_diagnostics(sql, connection, run_id)?;
    run.artifacts = read_artifacts(sql, connection, run_id)?;
    Ok(run)
}

pub(crate) fn latest_run(
    sql: &Sql,
    connection: &Connection,
    manuscript_id: &str,
) -> Result<Option<CompileRun>, WritingError> {
    let id = connection
        .query_row(
            sql.get("project-compile-latest.v1.sql"),
            [manuscript_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    id.map(|value| get_run(sql, connection, &value)).transpose()
}

pub(crate) fn latest_pdf_run(
    sql: &Sql,
    connection: &Connection,
    manuscript_id: &str,
) -> Result<Option<CompileRun>, WritingError> {
    let id = connection
        .query_row(
            sql.get("project-compile-latest-pdf.v1.sql"),
            [manuscript_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    id.map(|value| get_run(sql, connection, &value)).transpose()
}

pub(crate) fn pdf_object_path(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<String, WritingError> {
    connection
        .query_row(
            sql.get("project-compile-pdf-path.v1.sql"),
            [run_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(WritingError::CompileRunNotFound)
}

pub(crate) fn synctex_run_context(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<SynctexRunContext, WritingError> {
    connection
        .query_row(
            sql.get("project-compile-synctex-context.v1.sql"),
            [run_id],
            |row| {
                Ok(SynctexRunContext {
                    manuscript_id: row.get(0)?,
                    root_path: row.get(1)?,
                    toolchain_json: row.get(2)?,
                    input_manifest_hash: row.get(3)?,
                })
            },
        )
        .optional()?
        .ok_or(WritingError::SynctexUnavailable)
}

pub(crate) fn run_input_context(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<RunInputContext, WritingError> {
    connection
        .query_row(
            sql.get("project-compile-input-context.v1.sql"),
            [run_id],
            |row| {
                Ok(RunInputContext {
                    manuscript_id: row.get(0)?,
                    input_hash: row.get(1)?,
                })
            },
        )
        .optional()?
        .ok_or(WritingError::CompileRunNotFound)
}

fn insert_artifact(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
    kind: &str,
    content_sha256: Option<&str>,
    cache_relative_path: Option<&str>,
    byte_size: u64,
) -> Result<(), WritingError> {
    connection.execute(
        sql.get("project-compile-artifact-insert.v1.sql"),
        params![
            Uuid::new_v4().to_string(),
            run_id,
            kind,
            content_sha256,
            cache_relative_path,
            byte_size as i64,
        ],
    )?;
    Ok(())
}

fn read_diagnostics(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<CompileDiagnostic>, WritingError> {
    let mut statement = connection.prepare(sql.get("project-compile-diagnostics.v1.sql"))?;
    statement
        .query_map([run_id], |row| {
            Ok(CompileDiagnostic {
                severity: row.get(0)?,
                message: row.get(1)?,
                file: row.get(2)?,
                line: row.get(3)?,
                column: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn read_artifacts(
    sql: &Sql,
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<CompileArtifact>, WritingError> {
    let mut statement = connection.prepare(sql.get("project-compile-artifacts.v1.sql"))?;
    statement
        .query_map([run_id], |row| {
            Ok(CompileArtifact {
                kind: row.get(0)?,
                byte_size: row.get(1)?,
                content_hash: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}
