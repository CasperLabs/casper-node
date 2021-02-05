//! Chainspec loader component.
//!
//! The chainspec loader initializes a node by reading information from the chainspec or an
//! upgrade_point, and committing it to the permanent storage.
//!
//! See
//! <https://casperlabs.atlassian.net/wiki/spaces/EN/pages/135528449/Genesis+Process+Specification>
//! for full details.

use std::{
    fmt::{self, Display, Formatter},
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use datasize::DataSize;
use derive_more::From;
use once_cell::sync::Lazy;
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::task;
use tracing::{debug, error, info, trace, warn};

use casper_execution_engine::core::engine_state::{self, genesis::GenesisResult};

#[cfg(test)]
use crate::utils::RESOURCES_PATH;
use crate::{
    components::Component,
    crypto::hash::Digest,
    effect::{
        requests::{ChainspecLoaderRequest, ContractRuntimeRequest, StorageRequest},
        EffectBuilder, EffectExt, Effects,
    },
    rpcs::docs::DocExample,
    types::{
        chainspec::{Error, ProtocolConfig, CHAINSPEC_NAME},
        ActivationPoint, Chainspec,
    },
    utils::{self, Loadable},
    NodeRng,
};

/// `ChainspecHandler` events.
#[derive(Debug, From, Serialize)]
pub enum Event {
    #[from]
    Request(ChainspecLoaderRequest),
    /// The result of the `ChainspecHandler` putting a `Chainspec` to the storage component.
    PutToStorage { version: Version },
    /// The result of contract runtime running the genesis process.
    CommitGenesisResult(#[serde(skip_serializing)] Result<GenesisResult, engine_state::Error>),
}

impl Display for Event {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Request(_) => write!(formatter, "chainspec_loader request"),
            Event::PutToStorage { version } => {
                write!(formatter, "put chainspec {} to storage", version)
            }
            Event::CommitGenesisResult(result) => match result {
                Ok(genesis_result) => {
                    write!(formatter, "commit genesis result: {}", genesis_result)
                }
                Err(error) => write!(formatter, "failed to commit genesis: {}", error),
            },
        }
    }
}

#[derive(Clone, DataSize, Debug)]
pub struct ChainspecLoader {
    chainspec: Arc<Chainspec>,
    /// The path to the folder where all chainspec and upgrade_point files will be stored in
    /// subdirs corresponding to their versions.
    root_dir: PathBuf,
    /// If `Some`, we're finished loading and committing the chainspec.  The value of the bool
    /// indicates success (true) or not.
    completed_successfully: Option<bool>,
}

impl ChainspecLoader {
    pub(crate) fn new<P, REv>(
        chainspec_dir: P,
        effect_builder: EffectBuilder<REv>,
    ) -> Result<(Self, Effects<Event>), Error>
    where
        P: AsRef<Path>,
        REv: From<Event> + From<StorageRequest> + Send,
    {
        Ok(Self::new_with_chainspec_and_path(
            Chainspec::from_path(&chainspec_dir.as_ref())?,
            chainspec_dir,
            effect_builder,
        ))
    }

    #[cfg(test)]
    pub(crate) fn new_with_chainspec<REv>(
        chainspec: Chainspec,
        effect_builder: EffectBuilder<REv>,
    ) -> (Self, Effects<Event>)
    where
        REv: From<Event> + From<StorageRequest> + Send,
    {
        Self::new_with_chainspec_and_path(chainspec, &RESOURCES_PATH.join("local"), effect_builder)
    }

    fn new_with_chainspec_and_path<P, REv>(
        chainspec: Chainspec,
        chainspec_dir: P,
        effect_builder: EffectBuilder<REv>,
    ) -> (Self, Effects<Event>)
    where
        P: AsRef<Path>,
        REv: From<Event> + From<StorageRequest> + Send,
    {
        chainspec.validate_config();
        let root_dir = chainspec_dir
            .as_ref()
            .parent()
            .unwrap_or_else(|| {
                panic!("chainspec dir must have a parent");
            })
            .to_path_buf();

        let version = chainspec.protocol_config.version.clone();
        let effects = effect_builder
            .put_chainspec(chainspec.clone())
            .event(|_| Event::PutToStorage { version });
        let chainspec_loader = ChainspecLoader {
            chainspec,
            root_dir,
            completed_successfully: None,
        };

        (chainspec_loader, effects)
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.completed_successfully.is_some()
    }

    pub(crate) fn stopped_successfully(&self) -> bool {
        self.completed_successfully.unwrap_or_default()
    }

    pub(crate) fn genesis_state_root_hash(&self) -> &Option<Digest> {
        &self.state_root_hash
    }

    pub(crate) fn chainspec(&self) -> &Chainspec {
        &self.chainspec
    }
}

impl<REv> Component<REv> for ChainspecLoader
where
    REv: From<Event> + From<StorageRequest> + From<ContractRuntimeRequest> + Send,
{
    type Event = Event;
    type ConstructionError = Error;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Request(ChainspecLoaderRequest::NextUpgradeActivationPoint(responder)) => {
                let root_dir = self.root_dir.clone();
                let current_version = self.chainspec.protocol_config.version.clone();
                async move {
                    let maybe_upgrade_activation_point = task::spawn_blocking(move || {
                        next_activation_point(root_dir, current_version)
                    })
                    .await
                    .unwrap_or_else(|error| {
                        warn!(%error, "failed to join tokio task");
                        None
                    });
                    responder.respond(maybe_upgrade_activation_point).await
                }
                .ignore()
            }
            Event::PutToStorage { version } => {
                debug!("stored chainspec {}", version);
                effect_builder
                    .commit_genesis(self.chainspec.clone())
                    .event(Event::CommitGenesisResult)
            }
            Event::CommitGenesisResult(result) => {
                match result {
                    Ok(genesis_result) => match genesis_result {
                        GenesisResult::RootNotFound
                        | GenesisResult::KeyNotFound(_)
                        | GenesisResult::TypeMismatch(_)
                        | GenesisResult::Serialization(_) => {
                            error!("failed to commit genesis: {}", genesis_result);
                            self.completed_successfully = Some(false);
                        }
                        GenesisResult::Success {
                            post_state_hash,
                            effect,
                        } => {
                            info!("chainspec name {}", self.chainspec.network_config.name);
                            info!("genesis state root hash {}", post_state_hash);
                            trace!(%post_state_hash, ?effect);
                            self.completed_successfully = Some(true);
                            self.state_root_hash = Some(post_state_hash.into());
                        }
                    },
                    Err(error) => {
                        error!("failed to commit genesis: {}", error);
                        self.completed_successfully = Some(false);
                    }
                }
                Effects::new()
            }
        }
    }
}

/// This struct can be parsed from a TOML-encoded chainspec file.  It means that as the
/// chainspec format changes over versions, as long as we maintain the protocol config in this
/// form at the top of the chainspec file, it can continue to be parsed as an `UpgradePoint`.
#[derive(Deserialize)]
struct UpgradePoint {
    #[serde(rename = "protocol")]
    pub(crate) protocol_config: ProtocolConfig,
}

impl UpgradePoint {
    /// Parses a chainspec file at the given path as an `UpgradePoint`.
    fn from_chainspec_path<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let bytes = utils::read_file(path.as_ref().join(&CHAINSPEC_NAME))
            .map_err(Error::LoadUpgradePoint)?;
        Ok(toml::from_slice(&bytes)?)
    }
}

fn dir_name_from_version(version: &Version) -> PathBuf {
    PathBuf::from(version.to_string().replace(".", "_"))
}

/// Iterates the given path, returning the subdir representing the greatest SemVer version.
///
/// Subdir names should be semvers with dots replaced with underscores.
fn max_installed_version(dir: &Path) -> Result<Version, Error> {
    let mut max_version = Version::new(0, 0, 0);
    let mut read_version = false;
    for entry in fs::read_dir(dir).map_err(|error| Error::ReadDir {
        dir: dir.to_path_buf(),
        error,
    })? {
        let path = match entry {
            Ok(dir_entry) => dir_entry.path(),
            Err(error) => {
                debug!(dir=%dir.display(), %error, "bad entry while reading dir");
                continue;
            }
        };

        let subdir_name = match path.file_name() {
            Some(name) => name.to_string_lossy().replace("_", "."),
            None => continue,
        };

        let version = match Version::from_str(&subdir_name) {
            Ok(version) => version,
            Err(error) => {
                trace!(%error, path=%path.display(), "failed to get a version");
                continue;
            }
        };

        if version > max_version {
            max_version = version;
        }
        read_version = true;
    }

    if !read_version {
        return Err(Error::NoVersionSubdirFound {
            dir: dir.to_path_buf(),
        });
    }

    Ok(max_version)
}

/// Uses `max_installed_version()` to find the latest versioned subdir.  If this is greater than
/// `current_version`, reads the UpgradePoint file from there and returns its activation point.
/// Returns `None` if there is no greater version available, or if any step errors.
fn next_activation_point(dir: PathBuf, current_version: Version) -> Option<ActivationPoint> {
    let max_version = match max_installed_version(&dir) {
        Ok(version) => version,
        Err(error) => {
            warn!(dir=%dir.display(), %error, "failed to get a valid version from subdirs");
            return None;
        }
    };

    if max_version <= current_version {
        return None;
    }

    let subdir = dir.join(dir_name_from_version(&max_version));
    let upgrade_point = match UpgradePoint::from_chainspec_path(&subdir) {
        Ok(upgrade_point) => upgrade_point,
        Err(error) => {
            debug!(subdir=%subdir.display(), %error, "failed to load upgrade point");
            return None;
        }
    };

    if upgrade_point.protocol_config.version != max_version {
        warn!(
            upgrade_point_version=%upgrade_point.protocol_config.version,
            subdir_version=%max_version,
            "next chainspec installed to wrong subdir"
        );
        return None;
    }

    Some(upgrade_point.protocol_config.activation_point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{testing::TestRng, types::chainspec::CHAINSPEC_NAME};

    #[test]
    fn should_get_max_installed_version() {
        let tempdir = tempfile::tempdir().expect("should create temp dir");

        let max_version = || max_installed_version(tempdir.path()).unwrap();

        fs::create_dir(tempdir.path().join("1_0_0")).unwrap();
        assert_eq!(max_version(), Version::new(1, 0, 0));

        fs::create_dir(tempdir.path().join("1_0_3")).unwrap();
        assert_eq!(max_version(), Version::new(1, 0, 3));

        fs::create_dir(tempdir.path().join("1_2_3")).unwrap();
        assert_eq!(max_version(), Version::new(1, 2, 3));

        fs::create_dir(tempdir.path().join("1_2_2")).unwrap();
        assert_eq!(max_version(), Version::new(1, 2, 3));

        fs::create_dir(tempdir.path().join("2_2_2")).unwrap();
        assert_eq!(max_version(), Version::new(2, 2, 2));
    }

    #[test]
    fn should_ignore_invalid_versions() {
        let tempdir = tempfile::tempdir().expect("should create temp dir");

        // Executes `max_installed_version()` and asserts the resulting error as a string starts
        // with the given text.
        let assert_error_starts_with = |path: &Path, expected: String| {
            let error_msg = max_installed_version(path).unwrap_err().to_string();
            assert!(
                error_msg.starts_with(&expected),
                "Error message expected to start with \"{}\"\nActual error message: \"{}\"",
                expected,
                error_msg
            );
        };

        // Try with a non-existent dir.
        let non_existent_dir = Path::new("not_a_dir");
        assert_error_starts_with(
            non_existent_dir,
            format!("failed to read dir {}", non_existent_dir.display()),
        );

        // Try with a dir which has no subdirs.
        assert_error_starts_with(
            tempdir.path(),
            format!(
                "failed to get a valid version from subdirs in {}",
                tempdir.path().display()
            ),
        );

        // Try with a dir which has one subdir which is not a valid version representation.
        fs::create_dir(tempdir.path().join("not_a_version")).unwrap();
        assert_error_starts_with(
            tempdir.path(),
            format!(
                "failed to get a valid version from subdirs in {}",
                tempdir.path().display()
            ),
        );

        // Try with a dir which has a valid and invalid subdir - the invalid one should be ignored.
        fs::create_dir(tempdir.path().join("1_2_3")).unwrap();
        assert_eq!(
            max_installed_version(tempdir.path()).unwrap(),
            Version::new(1, 2, 3)
        );
    }

    /// Creates the appropriate subdir in `root_dir`, and adds a random chainspec.toml with the
    /// protocol_config.version field set to `version`.
    fn install_chainspec(rng: &mut TestRng, root_dir: &Path, version: &Version) -> Chainspec {
        let mut chainspec = Chainspec::random(rng);
        chainspec.protocol_config.version = version.clone();

        let subdir = root_dir.join(dir_name_from_version(&version));
        fs::create_dir(&subdir).unwrap();

        let path = subdir.join(CHAINSPEC_NAME);
        fs::write(
            path,
            toml::to_string_pretty(&chainspec).expect("should encode to toml"),
        )
        .expect("should install chainspec");
        chainspec
    }

    #[test]
    fn should_get_next_activation_point() {
        let tempdir = tempfile::tempdir().expect("should create temp dir");

        let max_point = |current_version: &Version| {
            next_activation_point(tempdir.path().to_path_buf(), current_version.clone()).unwrap()
        };

        let mut rng = crate::new_rng();

        let mut current = Version::new(0, 9, 9);
        let v1_0_0 = Version::new(1, 0, 0);
        let chainspec_v1_0_0 = install_chainspec(&mut rng, tempdir.path(), &v1_0_0);
        assert_eq!(
            max_point(&current),
            chainspec_v1_0_0.protocol_config.activation_point
        );

        current = v1_0_0;
        let v1_0_3 = Version::new(1, 0, 3);
        let chainspec_v1_0_3 = install_chainspec(&mut rng, tempdir.path(), &v1_0_3);
        assert_eq!(
            max_point(&current),
            chainspec_v1_0_3.protocol_config.activation_point
        );
    }

    #[test]
    fn should_not_get_old_or_invalid_activation_points() {
        let tempdir = tempfile::tempdir().expect("should create temp dir");

        let maybe_max_point = |current_version: &Version| {
            next_activation_point(tempdir.path().to_path_buf(), current_version.clone())
        };

        let mut rng = crate::new_rng();

        // Check we return `None` if there are no version subdirs.
        let v1_0_0 = Version::new(1, 0, 0);
        let mut current = v1_0_0.clone();
        assert!(maybe_max_point(&current).is_none());

        // Check we return `None` if current_version == max_version.
        let chainspec_v1_0_0 = install_chainspec(&mut rng, tempdir.path(), &v1_0_0);
        assert!(maybe_max_point(&current).is_none());

        // Check we return `None` if current_version > max_version.
        current = Version::new(2, 0, 0);
        assert!(maybe_max_point(&current).is_none());

        // Check we return `None` if we find an upgrade file where the protocol_config.version field
        // doesn't match the subdir name.
        let v0_9_9 = Version::new(0, 9, 9);
        current = v0_9_9.clone();
        assert!(maybe_max_point(&current).is_some());

        let mut chainspec_v0_9_9 = chainspec_v1_0_0;
        chainspec_v0_9_9.protocol_config.version = v0_9_9;
        let path_v1_0_0 = tempdir
            .path()
            .join(dir_name_from_version(&v1_0_0))
            .join(CHAINSPEC_NAME);
        fs::write(
            &path_v1_0_0,
            toml::to_string_pretty(&chainspec_v0_9_9).expect("should encode to toml"),
        )
        .expect("should install upgrade point");
        assert!(maybe_max_point(&current).is_none());

        // Check we return `None` if the max version upgrade_point file is corrupt.
        fs::write(&path_v1_0_0, "bad data".as_bytes()).unwrap();
        assert!(maybe_max_point(&current).is_none());

        // Check we return `None` if the max version upgrade_point file is missing.
        fs::remove_file(&path_v1_0_0).unwrap();
        assert!(maybe_max_point(&current).is_none());
    }
}
