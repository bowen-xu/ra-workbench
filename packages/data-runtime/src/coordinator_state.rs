use super::*;

impl<P: Projector> State<P> {
    pub(super) fn recover(
        log: RaftLog,
        projector: P,
        config: CoordinatorConfig,
        codec_counters: Option<CodecCounters>,
    ) -> CoordinatorResult<Self> {
        if config.materialize_records == 0
            || config.materialize_bytes == 0
            || config.replay_batch_records == 0
            || config.replay_batch_records > 4096
            || config.max_command_bytes == 0
            || config.max_pending_records < config.materialize_records
            || config.max_pending_bytes < config.materialize_bytes
            || config.max_pending_bytes < config.max_command_bytes
            || config.reclaim_after_records == 0
        {
            return Err(CoordinatorError::InvalidConfiguration(
                "invalid threshold or pending bound",
            ));
        }
        let head = log.head()?;
        let sql = projector
            .state()
            .map_err(|error| projector_error("open", head.accepted_position, error))?;
        if sql.epoch != head.epoch || sql.epoch.is_nil() {
            return Err(CoordinatorError::EpochMismatch);
        }
        if sql.materialized_position > head.accepted_position
            || sql.materialized_position < head.compacted_position
        {
            return Err(CoordinatorError::InvalidPairing {
                accepted: head.accepted_position,
                materialized: sql.materialized_position,
                compacted: head.compacted_position,
            });
        }
        let mut owner = Self {
            log,
            projector,
            config,
            codec_counters,
            epoch: head.epoch,
            accepted: head.accepted_position,
            materialized: sql.materialized_position,
            pending: vec![],
            pending_sizes: vec![],
            pending_bytes: 0,
            blocked: false,
            closed: false,
            last_reclaim_extent: None,
            last_reclaim_position: head.compacted_position,
        };
        // Fixed head: opening owns the engine exclusively, and never chases a
        // moving stream. Each bounded page is applied before retaining the next.
        while owner.materialized < owner.accepted {
            let start = owner
                .materialized
                .checked_add(1)
                .ok_or(CoordinatorError::InvalidFence)?;
            let page = owner.log.read_range(
                start,
                owner
                    .config
                    .replay_batch_records
                    .min(owner.config.max_pending_records),
                owner.config.max_pending_bytes,
            )?;
            if page.is_empty() {
                return Err(CoordinatorError::Corrupt {
                    position: start,
                    reason: "missing_log_suffix",
                });
            }
            for entry in page {
                let expected = owner.materialized + owner.pending.len() as u64 + 1;
                if entry.position != expected || entry.position > owner.accepted {
                    return Err(CoordinatorError::Corrupt {
                        position: entry.position,
                        reason: "noncontiguous_log_suffix",
                    });
                }
                let record = record::decode::<P>(
                    &entry.payload,
                    entry.position,
                    owner.epoch,
                    owner.config.max_command_bytes,
                    owner.codec_counters.as_ref(),
                )?;
                if owner
                    .pending
                    .iter()
                    .any(|old| old.operation_id == record.operation_id)
                    || owner
                        .projector
                        .lookup(record.operation_id)
                        .map_err(|error| projector_error("recovery_lookup", entry.position, error))?
                        .is_some()
                {
                    return Err(CoordinatorError::Corrupt {
                        position: entry.position,
                        reason: "duplicate_operation",
                    });
                }
                owner.pending_bytes = owner.pending_bytes.checked_add(entry.payload.len()).ok_or(
                    CoordinatorError::PendingLimit("pending byte counter overflow"),
                )?;
                owner.pending_sizes.push(entry.payload.len());
                owner.pending.push(record);
            }
            let through = owner
                .pending
                .last()
                .ok_or(CoordinatorError::InvalidFence)?
                .position;
            owner.materialize_through(through)?;
        }
        Ok(owner)
    }
    pub(super) fn ensure_ready(&self) -> CoordinatorResult<()> {
        if self.blocked {
            Err(CoordinatorError::RecoveryRequired)
        } else if self.closed {
            Err(CoordinatorError::Closed)
        } else {
            Ok(())
        }
    }
    pub(super) fn accepted_fence(&self) -> ReadFence {
        ReadFence {
            epoch: self.epoch,
            position: self.accepted,
        }
    }
    pub(super) fn check_fence(&self, fence: ReadFence) -> CoordinatorResult<()> {
        if fence.epoch != self.epoch {
            return Err(CoordinatorError::EpochMismatch);
        }
        if fence.position > self.accepted {
            return Err(CoordinatorError::InvalidFence);
        }
        Ok(())
    }
    fn check_sql_state(&mut self) -> CoordinatorResult<()> {
        let sql = match self.projector.state() {
            Ok(state) => state,
            Err(error) => {
                self.blocked = true;
                return Err(projector_error("state", self.materialized, error));
            }
        };
        if sql.epoch != self.epoch || sql.materialized_position != self.materialized {
            self.blocked = true;
            return Err(CoordinatorError::Corrupt {
                position: self.materialized,
                reason: "SQL_watermark_changed_outside_owner",
            });
        }
        Ok(())
    }
    pub(super) fn materialize_through(&mut self, target: u64) -> CoordinatorResult<()> {
        self.check_sql_state()?;
        while self.materialized < target {
            let count = self
                .pending
                .iter()
                .take_while(|record| record.position <= target)
                .take(self.config.replay_batch_records)
                .count();
            if count == 0 {
                self.blocked = true;
                return Err(CoordinatorError::Corrupt {
                    position: self.materialized + 1,
                    reason: "missing_pending_record",
                });
            }
            let through = self.pending[count - 1].position;
            if let Err(error) = self.projector.apply_batch(&self.pending[..count]) {
                self.blocked = true;
                return Err(projector_error("apply_batch", through, error));
            }
            // Verify the transactional acknowledgement before releasing the only
            // recoverable suffix. A failed/unknown SQL commit retains all bytes.
            let sql = match self.projector.state() {
                Ok(state) => state,
                Err(error) => {
                    self.blocked = true;
                    return Err(projector_error("applied_state", through, error));
                }
            };
            if sql.epoch != self.epoch || sql.materialized_position != through {
                self.blocked = true;
                return Err(CoordinatorError::Corrupt {
                    position: through,
                    reason: "atomic_SQL_watermark_not_committed",
                });
            }
            self.materialized = through;
            self.pending_bytes -= self.pending_sizes.drain(..count).sum::<usize>();
            self.pending.drain(..count);
        }
        Ok(())
    }
    pub(super) fn sync_log(&mut self) -> CoordinatorResult<u64> {
        match self.log.sync() {
            Ok(position) => Ok(position),
            Err(error) => {
                self.blocked = true;
                Err(error.into())
            }
        }
    }
    fn existing(
        &mut self,
        receipt: OperationReceipt<P::Receipt>,
        options: WriteOptions,
    ) -> CoordinatorResult<WriteReceipt<P::Receipt>> {
        let completion = (|| {
            if options.durability == Durability::Device {
                self.sync_log()?;
            }
            if options.mode == WriteMode::Immediate {
                self.materialize_through(receipt.position)?;
            }
            Ok::<_, CoordinatorError>(())
        })();
        if let Err(cause) = completion {
            self.blocked = true;
            return Err(CoordinatorError::AcceptedIncomplete {
                operation_id: receipt.operation_id,
                position: receipt.position,
                cause: Box::new(cause),
            });
        }
        Ok(WriteReceipt {
            epoch: self.epoch,
            position: receipt.position,
            operation_id: receipt.operation_id,
            result: receipt.result,
            materialized: receipt.position <= self.materialized,
            replayed: true,
            legacy: false,
        })
    }
    pub(super) fn write(
        &mut self,
        command: P::Command,
        options: WriteOptions,
    ) -> CoordinatorResult<WriteReceipt<P::Receipt>> {
        let operation_id = P::operation_id(&command);
        if operation_id.is_nil() {
            return Err(CoordinatorError::Corrupt {
                position: self.accepted,
                reason: "nil_operation_id",
            });
        }
        let position = self
            .accepted
            .checked_add(1)
            .ok_or(CoordinatorError::PendingLimit("position exhausted"))?;
        let encoded = record::command_bytes(&command, position, self.codec_counters.as_ref())?;
        if encoded.len() > self.config.max_command_bytes {
            return Err(CoordinatorError::PendingLimit("command byte limit"));
        }
        // A successful serialization that cannot replay is not a valid mutation.
        serde_json::from_slice::<P::Command>(&encoded)
            .map_err(|_| record::codec(position, "command_round_trip"))?;
        if let Some(counters) = &self.codec_counters {
            counters.command_round_trip(encoded.len());
        }
        let fingerprint = record::fingerprint(&encoded, self.codec_counters.as_ref());
        if let Some(record) = self
            .pending
            .iter()
            .find(|record| record.operation_id == operation_id)
        {
            if record.fingerprint != fingerprint {
                return Err(CoordinatorError::OperationConflict { operation_id });
            }
            return self.existing(
                OperationReceipt {
                    operation_id,
                    position: record.position,
                    fingerprint,
                    result: record.result.clone(),
                },
                options,
            );
        }
        self.check_sql_state()?;
        let saved = match self.projector.lookup(operation_id) {
            Ok(saved) => saved,
            Err(error) => {
                self.blocked = true;
                return Err(projector_error("lookup", self.materialized, error));
            }
        };
        if let Some(receipt) = saved {
            if receipt.operation_id != operation_id
                || receipt.position == 0
                || receipt.position > self.materialized
            {
                self.blocked = true;
                return Err(CoordinatorError::Corrupt {
                    position: receipt.position,
                    reason: "SQL_operation_receipt_identity",
                });
            }
            if receipt.fingerprint != fingerprint {
                return Err(CoordinatorError::OperationConflict { operation_id });
            }
            return self.existing(receipt, options);
        }
        let admission = self
            .projector
            .validate(&command, &self.pending)
            .map_err(|error| projector_error("validate", position, error))?;
        let result = match admission {
            Admission::New(result) => result,
            Admission::ExistingLegacy(result) => {
                let position = self.materialized;
                let completion = (|| {
                    if options.mode == WriteMode::Immediate {
                        self.materialize_through(self.accepted)?;
                    }
                    if options.durability == Durability::Device {
                        self.sync_log()?;
                    }
                    Ok::<_, CoordinatorError>(())
                })();
                if let Err(cause) = completion {
                    self.blocked = true;
                    return Err(CoordinatorError::AcceptedIncomplete {
                        operation_id,
                        position,
                        cause: Box::new(cause),
                    });
                }
                return Ok(WriteReceipt {
                    epoch: self.epoch,
                    position: self.materialized,
                    operation_id,
                    result,
                    materialized: true,
                    replayed: true,
                    legacy: true,
                });
            }
        };
        let record = Accepted {
            epoch: self.epoch,
            position,
            operation_id,
            fingerprint,
            command,
            result,
        };
        let payload = record::encode(&record, self.codec_counters.as_ref())?;
        if payload.len() > self.config.max_pending_bytes {
            return Err(CoordinatorError::PendingLimit("record byte limit"));
        }
        record::decode::<P>(
            &payload,
            position,
            self.epoch,
            self.config.max_command_bytes,
            self.codec_counters.as_ref(),
        )?;
        if self.pending.len() >= self.config.max_pending_records
            || payload.len()
                > self
                    .config
                    .max_pending_bytes
                    .saturating_sub(self.pending_bytes)
        {
            self.materialize_through(self.accepted)?;
        }
        // No success, receipt publication, or in-memory head advance precedes
        // this nonempty append. Unknown engine outcomes poison this owner.
        self.append_prepared(operation_id, position, &payload, options.durability)?;
        self.accepted = position;
        let receipt_result = record.result.clone();
        self.pending_sizes.push(payload.len());
        self.pending_bytes += payload.len();
        self.pending.push(record);
        if options.mode == WriteMode::Immediate
            || self.pending.len() >= self.config.materialize_records
            || self.pending_bytes >= self.config.materialize_bytes
        {
            self.materialize_through(position).map_err(|cause| {
                CoordinatorError::AcceptedIncomplete {
                    operation_id,
                    position,
                    cause: Box::new(cause),
                }
            })?;
        }
        self.maybe_reclaim(false).map_err(|cause| {
            self.blocked = true;
            CoordinatorError::AcceptedIncomplete {
                operation_id,
                position,
                cause: Box::new(cause),
            }
        })?;
        Ok(WriteReceipt {
            epoch: self.epoch,
            position,
            operation_id,
            result: receipt_result,
            materialized: position <= self.materialized,
            replayed: false,
            legacy: false,
        })
    }
}
