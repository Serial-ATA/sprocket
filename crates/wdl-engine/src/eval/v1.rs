//! Implementation of evaluation for V1 documents.

mod expr;
mod task;
mod validators;
mod workflow;

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Context;
use anyhow::Result;
pub(crate) use expr::*;
use serde::Deserialize;
use serde::Serialize;
pub use task::requirements::ContainerSource;
pub(crate) use task::*;
use tokio::sync::broadcast;
use tracing::info;
use wdl_analysis::types::EnumVariantCacheKey;

use super::CancellationContext;
use super::Events;
use crate::EngineEvent;
use crate::Value;
use crate::backend::TaskExecutionBackend;
use crate::cache::CallCache;
use crate::cache::CallCacheExclusions;
use crate::config::CallCachingMode;
use crate::config::Config;
use crate::http::HttpTransferer;
use crate::http::Transferer;

/// The name of the inputs file to write for each task and workflow in the
/// outputs directory.
const INPUTS_FILE: &str = "inputs.json";

/// The name of the outputs file to write for each task and workflow in the
/// outputs directory.
const OUTPUTS_FILE: &str = "outputs.json";

/// Serializes a value into a JSON file.
fn write_json_file(path: impl AsRef<Path>, value: &impl Serialize) -> Result<()> {
    let path = path.as_ref();
    let file = File::create(path)
        .with_context(|| format!("failed to create file `{path}`", path = path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), value)
        .with_context(|| format!("failed to write file `{path}`", path = path.display()))
}

/// A map of container image overrides.
///
/// `<image name> -> <override>`
///
/// This is used in Sprocket lockfiles to map mutable image tags (e.g.
/// `ubuntu:latest`) to an immutable hash (e.g.
/// `ubuntu@sha256:
/// c4a8d5503dfb2a3eb8ab5f807da5bc69a85730fb49b5cfca2330194ebcc41c7b`).
///
/// See also: [`ImageDigests`].
pub type ImageOverrideMap = HashMap<String, ImageDigests>;

/// The digest specification for a container image.
///
/// This bridges the gap between OCI-compliant and non-compliant registries with
/// regards to storing architecture-specific hashes.
///
/// See also: [`ImageOverrideMap`].
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ImageDigests {
    /// (Possibly) multi-arch OCI manifest index.
    ///
    /// For OCI-compatible registries, all supported architectures can be
    /// identified under a single umbrella index hash. The runtime (e.g.,
    /// Docker) is responsible for determining the appropriate image for the
    /// host architecture based on that hash.
    OciManifest(ContainerSource),
    /// Per-architecture image hashes.
    ///
    /// This is used for non-OCI registries (e.g., Sylabs Cloud Library) where
    /// each image is an entirely separate artifact, and is thus hashed
    /// separately.
    ///
    /// The keys of the map match [`std::env::consts::ARCH`] values.
    PerArch(HashMap<String, ContainerSource>),
}

/// Represents a WDL evaluator.
///
/// The evaluator is used to evaluate a specific task or the workflow of an
/// analyzed document.
///
/// This type is cheaply cloned and sendable between threads.
#[derive(Clone)]
pub struct Evaluator {
    /// The associated evaluation configuration.
    config: Arc<Config>,
    /// The associated task execution backend.
    backend: Arc<dyn TaskExecutionBackend>,
    /// The cancellation context for cancelling task evaluation.
    cancellation: CancellationContext,
    /// The transferer to use for expression evaluation.
    transferer: Arc<dyn Transferer>,
    /// The call cache to use for task evaluation.
    cache: Option<CallCache>,
    /// The events for evaluation.
    events: Option<broadcast::Sender<EngineEvent>>,
    /// Cache for evaluated enum variant values to avoid redundant AST lookups.
    variant_cache: Arc<Mutex<HashMap<EnumVariantCacheKey, Value>>>,
    /// Container image overrides.
    ///
    /// See [`ImageOverrideMap`].
    image_overrides: Arc<ImageOverrideMap>,
}

impl Evaluator {
    /// Constructs a new evaluator with the given evaluation root directory,
    /// evaluation configuration, cancellation context, and events.
    ///
    /// Returns an error if the configuration isn't valid.
    pub async fn new(
        root_dir: impl AsRef<Path>,
        config: Arc<Config>,
        cancellation: CancellationContext,
        events: Events,
    ) -> Result<Self> {
        config.validate().await?;

        let root_dir = root_dir.as_ref();
        let backend = config
            .create_backend(root_dir, events.clone(), cancellation.clone())
            .await?;
        let transferer = Arc::new(HttpTransferer::new(
            config.clone(),
            cancellation.first(),
            events.transfer().clone(),
        )?);

        let cache = match config.task.cache {
            CallCachingMode::Off => {
                info!("call caching is disabled");
                None
            }
            _ => Some(
                CallCache::new(
                    config.task.cache_dir().as_deref(),
                    config.task.digests,
                    transferer.clone(),
                    Arc::new(CallCacheExclusions {
                        inputs: config.task.excluded_cache_inputs.clone(),
                        requirements: config.task.excluded_cache_requirements.clone(),
                        hints: config.task.excluded_cache_hints.clone(),
                    }),
                )
                .await?,
            ),
        };

        Ok(Self {
            config,
            backend,
            cancellation,
            transferer,
            cache,
            events: events.engine().clone(),
            variant_cache: Default::default(),
            image_overrides: Arc::default(),
        })
    }

    /// Add a set of container image overrides.
    pub fn with_image_overrides(mut self, overrides: Arc<ImageOverrideMap>) -> Self {
        self.image_overrides = overrides;
        self
    }
}
