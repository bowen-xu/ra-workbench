use std::{path::Path, sync::Arc};

use arrow_array::Array;
use arrow_array::{
    FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator, StringArray,
    types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures_util::TryStreamExt;
use lancedb::{
    DistanceType, Table, connect,
    query::{ExecutableQuery, QueryBase},
};
use research_semantic_engine_api::{
    SemanticNamespace, SemanticVectorRow, SemanticVectorSearchHit, StoredVectorMetadata,
    VectorWriteReceipt,
};

use crate::error::EngineError;

pub(crate) struct LanceDbStore {
    table: Table,
    schema: SchemaRef,
    dimension: usize,
}

impl LanceDbStore {
    pub(crate) async fn open(
        path: &Path,
        namespace: SemanticNamespace,
        dimension: usize,
    ) -> Result<Self, EngineError> {
        if dimension == 0 || dimension > 65_536 {
            return Err(EngineError::InvalidInput(
                "vector dimension is out of range",
            ));
        }
        let uri = path.to_str().ok_or(EngineError::InvalidInput(
            "database path is not valid UTF-8",
        ))?;
        let database = connect(uri).execute().await?;
        let schema = vector_schema(dimension);
        let table_names = database.table_names().execute().await?;
        let table = if table_names
            .iter()
            .any(|name| name == namespace.table_name())
        {
            database
                .open_table(namespace.table_name())
                .execute()
                .await?
        } else {
            database
                .create_empty_table(namespace.table_name(), schema.clone())
                .execute()
                .await?
        };
        let actual = table.schema().await?;
        if actual.as_ref() != schema.as_ref() {
            return Err(EngineError::IncompatibleSchema);
        }
        Ok(Self {
            table,
            schema,
            dimension,
        })
    }

    pub(crate) async fn ensure_ready(&self) -> Result<u64, EngineError> {
        self.version().await
    }

    pub(crate) async fn upsert(
        &self,
        row: &SemanticVectorRow,
    ) -> Result<VectorWriteReceipt, EngineError> {
        self.validate_row(row)?;
        let reader = RecordBatchIterator::new([Ok(self.batch(row)?)], self.schema.clone());
        let mut merge = self.table.merge_insert(&["row_id"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        let result = merge.execute(Box::new(reader)).await?;
        Ok(VectorWriteReceipt {
            dataset_version: result.version,
            inserted_rows: result.num_inserted_rows,
            updated_rows: result.num_updated_rows,
        })
    }

    pub(crate) async fn contains(
        &self,
        row_id: &str,
        input_sha256: &str,
    ) -> Result<bool, EngineError> {
        if !valid_id(row_id) || input_sha256.len() != 64 {
            return Err(EngineError::InvalidInput("vector row identity is invalid"));
        }
        let predicate = format!("row_id = '{row_id}' and input_sha256 = '{input_sha256}'");
        let rows = self
            .table
            .query()
            .only_if(predicate)
            .limit(1)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>();
        Ok(rows == 1)
    }

    pub(crate) async fn list_metadata(&self) -> Result<Vec<StoredVectorMetadata>, EngineError> {
        let dataset_version = self.version().await?;
        let batches = self
            .table
            .query()
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut metadata = Vec::new();
        for batch in batches {
            let row_ids = string_column(&batch, "row_id")?;
            let input_hashes = string_column(&batch, "input_sha256")?;
            let vectors = batch
                .column_by_name("vector")
                .and_then(|column| column.as_any().downcast_ref::<FixedSizeListArray>())
                .ok_or(EngineError::IncompatibleSchema)?;
            for index in 0..batch.num_rows() {
                let vector = vectors.value(index);
                let values = vector
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or(EngineError::IncompatibleSchema)?;
                let vector_norm = values
                    .values()
                    .iter()
                    .map(|value| value * value)
                    .sum::<f32>()
                    .sqrt();
                if !vector_norm.is_finite() {
                    return Err(EngineError::IncompatibleSchema);
                }
                metadata.push(StoredVectorMetadata {
                    row_id: row_ids.value(index).to_owned(),
                    input_sha256: input_hashes.value(index).to_owned(),
                    vector_norm,
                    dataset_version,
                });
            }
        }
        Ok(metadata)
    }

    pub(crate) async fn search(
        &self,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticVectorSearchHit>, EngineError> {
        if query.len() != self.dimension
            || query.iter().any(|value| !value.is_finite())
            || limit == 0
            || limit > 1_000
        {
            return Err(EngineError::InvalidInput(
                "query vector or limit is invalid",
            ));
        }
        let batches = self
            .table
            .query()
            .nearest_to(query)?
            .distance_type(DistanceType::Cosine)
            .limit(limit)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut hits = Vec::new();
        for batch in batches {
            let row_ids = string_column(&batch, "row_id")?;
            let representation_ids = string_column(&batch, "representation_id")?;
            let subject_ids = string_column(&batch, "subject_id")?;
            let revisions = batch
                .column_by_name("subject_revision")
                .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
                .ok_or(EngineError::IncompatibleSchema)?;
            let distances = batch
                .column_by_name("_distance")
                .and_then(|column| column.as_any().downcast_ref::<Float32Array>())
                .ok_or(EngineError::IncompatibleSchema)?;
            for index in 0..batch.num_rows() {
                hits.push(SemanticVectorSearchHit {
                    row_id: row_ids.value(index).to_owned(),
                    representation_id: representation_ids.value(index).to_owned(),
                    subject_id: subject_ids.value(index).to_owned(),
                    subject_revision: revisions.value(index),
                    distance: distances.value(index),
                });
            }
        }
        Ok(hits)
    }

    pub(crate) async fn delete(&self, row_id: &str) -> Result<u64, EngineError> {
        if !valid_id(row_id) {
            return Err(EngineError::InvalidInput("vector row ID is invalid"));
        }
        self.table.delete(&format!("row_id = '{row_id}'")).await?;
        self.version().await
    }

    fn validate_row(&self, row: &SemanticVectorRow) -> Result<(), EngineError> {
        if !valid_id(&row.row_id)
            || row.representation_id.trim().is_empty()
            || row.subject_kind.trim().is_empty()
            || row.subject_id.trim().is_empty()
            || row.subject_revision < 1
            || row.representation_kind.trim().is_empty()
            || row.input_sha256.len() != 64
            || !valid_id(&row.embedding_space_id)
            || row.generation < 1
            || row.vector.len() != self.dimension
            || row.vector.iter().any(|value| !value.is_finite())
            || row.created_at.trim().is_empty()
        {
            return Err(EngineError::InvalidInput("vector row is invalid"));
        }
        Ok(())
    }

    fn batch(&self, row: &SemanticVectorRow) -> Result<RecordBatch, EngineError> {
        Ok(RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![row.row_id.as_str()])),
                Arc::new(StringArray::from(vec![row.representation_id.as_str()])),
                Arc::new(StringArray::from(vec![row.subject_kind.as_str()])),
                Arc::new(StringArray::from(vec![row.subject_id.as_str()])),
                Arc::new(Int64Array::from(vec![row.subject_revision])),
                Arc::new(StringArray::from(vec![row.representation_kind.as_str()])),
                Arc::new(StringArray::from(vec![row.language.as_str()])),
                Arc::new(StringArray::from(vec![row.input_sha256.as_str()])),
                Arc::new(StringArray::from(vec![row.embedding_space_id.as_str()])),
                Arc::new(Int64Array::from(vec![row.generation])),
                Arc::new(
                    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                        [Some(
                            row.vector.iter().copied().map(Some).collect::<Vec<_>>(),
                        )],
                        self.dimension as i32,
                    ),
                ),
                Arc::new(StringArray::from(vec![row.created_at.as_str()])),
            ],
        )?)
    }

    async fn version(&self) -> Result<u64, EngineError> {
        Ok(self.table.version().await?)
    }
}

fn vector_schema(dimension: usize) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("row_id", DataType::Utf8, false),
        Field::new("representation_id", DataType::Utf8, false),
        Field::new("subject_kind", DataType::Utf8, false),
        Field::new("subject_id", DataType::Utf8, false),
        Field::new("subject_revision", DataType::Int64, false),
        Field::new("representation_kind", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("input_sha256", DataType::Utf8, false),
        Field::new("embedding_space_id", DataType::Utf8, false),
        Field::new("generation", DataType::Int64, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dimension as i32,
            ),
            false,
        ),
        Field::new("created_at", DataType::Utf8, false),
    ]))
}

fn string_column<'a>(batch: &'a RecordBatch, column: &str) -> Result<&'a StringArray, EngineError> {
    batch
        .column_by_name(column)
        .and_then(|value| value.as_any().downcast_ref::<StringArray>())
        .ok_or(EngineError::IncompatibleSchema)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
