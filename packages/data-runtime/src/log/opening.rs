//! Engine opening and explicit empty-bootstrap metadata initialization.
use super::*;

#[derive(Clone, Copy)]
enum MetadataInitialization {
    FreshDirectory,
    PreparingEmpty,
}

impl RaftLog {
    pub fn open(path: impl AsRef<Path>, expected_epoch: Option<Uuid>) -> LogResult<Self> {
        Self::open_config(LogConfig::desktop(path), expected_epoch)
    }

    pub fn open_config(config: LogConfig, expected_epoch: Option<Uuid>) -> LogResult<Self> {
        Self::open_with_file_system(config, expected_epoch, Arc::new(DefaultFileSystem))
    }

    /// Bind a fresh per-opening observer before any engine initialization or
    /// recovery I/O. Keep a clone to sample failures and the final close window.
    pub fn open_config_with_io(
        config: LogConfig,
        expected_epoch: Option<Uuid>,
        io: LogIoCounters,
    ) -> LogResult<Self> {
        Self::open_with_initialization(
            config,
            expected_epoch,
            Arc::new(DefaultFileSystem),
            MetadataInitialization::FreshDirectory,
            io,
        )
    }

    /// Only a trusted SQL schema-1 `preparing` pair at position zero may select
    /// this bootstrap recovery. Missing metadata is initialized only after the
    /// published engine proves it has no entries or KV data. Ready pairs must
    /// use ordinary open; this never replaces accepted business records.
    pub fn open_preparing(path: impl AsRef<Path>, expected_epoch: Uuid) -> LogResult<Self> {
        Self::open_preparing_with_io(path, expected_epoch, LogIoCounters::default())
    }

    /// The same trusted empty-bootstrap recovery as `open_preparing`, with an
    /// observer installed before the engine opens. No pairing rules change.
    pub fn open_preparing_with_io(
        path: impl AsRef<Path>,
        expected_epoch: Uuid,
        io: LogIoCounters,
    ) -> LogResult<Self> {
        let log = Self::open_with_initialization(
            LogConfig::desktop(path),
            Some(expected_epoch),
            Arc::new(DefaultFileSystem),
            MetadataInitialization::PreparingEmpty,
            io,
        )?;
        if log.head.accepted_position != 0 || log.head.compacted_position != 0 {
            return Err(LogError::Corrupt(
                "preparing log contains accepted business records",
            ));
        }
        Ok(log)
    }
}

impl<F: FileSystem> RaftLog<F> {
    pub(super) fn open_with_file_system(
        config: LogConfig,
        expected_epoch: Option<Uuid>,
        fs: Arc<F>,
    ) -> LogResult<Self> {
        Self::open_with_initialization(
            config,
            expected_epoch,
            fs,
            MetadataInitialization::FreshDirectory,
            LogIoCounters::default(),
        )
    }

    fn open_with_initialization(
        config: LogConfig,
        expected_epoch: Option<Uuid>,
        fs: Arc<F>,
        initialization: MetadataInitialization,
        io: LogIoCounters,
    ) -> LogResult<Self> {
        if expected_epoch.is_some_and(|epoch| epoch.is_nil()) {
            return Err(LogError::InvalidArgument("store epoch cannot be nil"));
        }
        let engine_config = config.engine_config()?;
        let existed_nonempty = match fs::read_dir(&config.path) {
            Ok(mut entries) => entries
                .next()
                .transpose()
                .map_err(|source| LogError::Io {
                    operation: "inspect log directory",
                    source,
                })?
                .is_some(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(source) => {
                return Err(LogError::Io {
                    operation: "inspect log directory",
                    source,
                });
            }
        };
        create_private_directory(&config.path)?;
        let account = Account::inventory(
            &config.path,
            config.max_managed_file_bytes,
            config.maintenance_reserve_bytes,
        )
        .map_err(|source| LogError::Io {
            operation: "inventory log extents",
            source,
        })?;
        let tracked_fs = Arc::new(ExtentFileSystem::new(fs, account.clone(), io.clone()));
        let engine = match catch_unwind(AssertUnwindSafe(|| {
            Engine::open_with_file_system(engine_config, tracked_fs)
        })) {
            Ok(Ok(engine)) => engine,
            Ok(Err(source)) => {
                return Err(LogError::Engine {
                    operation: "open",
                    source,
                });
            }
            Err(_) => return Err(LogError::EnginePanic { operation: "open" }),
        };
        let mut log = Self {
            engine,
            account,
            io,
            target_file_bytes: config.target_file_bytes,
            maintenance_trigger_bytes: config.target_file_bytes.saturating_mul(4),
            path: config.path,
            head: LogHead {
                epoch: Uuid::nil(),
                accepted_position: 0,
                synced_position: 0,
                compacted_position: 0,
            },
            poisoned: false,
        };
        let state = log.call("recover state", |engine| {
            engine.get_message::<LogState>(GROUP, STATE_KEY)
        })?;
        match state {
            Some(state) => {
                log.head = decode_state(state)?;
                if expected_epoch.is_some_and(|epoch| epoch != log.head.epoch) {
                    return Err(LogError::EpochMismatch);
                }
                log.validate_indexes()?;
            }
            None => {
                if (existed_nonempty
                    && matches!(initialization, MetadataInitialization::FreshDirectory))
                    || !log.call("inspect groups", |engine| Ok(engine.is_empty()))?
                {
                    return Err(LogError::Corrupt("nonempty log has no runtime metadata"));
                }
                log.head.epoch = expected_epoch.unwrap_or_else(Uuid::new_v4);
                let mut batch = LogBatch::default();
                put_state(&mut batch, log.head)?;
                log.call("initialize state", |engine| engine.write(&mut batch, true))?;
            }
        }
        Ok(log)
    }
}
