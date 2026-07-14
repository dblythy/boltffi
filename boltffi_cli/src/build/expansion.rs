use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use boltffi_bindgen::cargo::LibraryCargoArgs;
use boltffi_bindgen::generate::Generation;
use boltffi_binding::{
    BINDING_EXPANSION_BUILD_ENV, BINDING_EXPANSION_ROOT_ENV, BINDING_EXPANSION_SOURCE_ENV,
    BINDING_EXPANSION_SURFACE_ENV, BindingMetadataSurface,
};

use crate::cargo::{Cargo, CargoMetadata, SelectedLibrary};
use crate::cli::{CliError, Result};
use crate::config::Config;

const BINDING_EXPANSION_CFG: &str = "boltffi_binding_expansion";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingExpansion {
    library: SelectedLibrary,
    target_directory: PathBuf,
    cargo_args: LibraryCargoArgs,
    toolchain_selector: Option<String>,
}

impl BindingExpansion {
    pub fn resolve_for_commands(
        config: &Config,
        commands: &[&str],
        cargo_args: &[String],
    ) -> Result<Self> {
        let resolved_cargo_args = config
            .cargo_args_for_commands(commands)
            .into_iter()
            .chain(cargo_args.iter().cloned())
            .collect::<Vec<_>>();
        Self::resolve(config, &resolved_cargo_args)
    }

    pub fn resolve(config: &Config, build_cargo_args: &[String]) -> Result<Self> {
        let cargo = Cargo::current(build_cargo_args)?;
        let cargo_args = LibraryCargoArgs::parse(cargo.probe_command_arguments())?;
        let metadata = cargo.metadata()?;
        let cargo_manifest_path = cargo.manifest_path()?;
        Self::from_metadata(config, &cargo, &metadata, &cargo_manifest_path, cargo_args)
    }

    pub fn package_id(&self) -> &str {
        self.library.package_id()
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.library.manifest_path().to_path_buf()
    }

    pub fn cargo_manifest_path(&self) -> &std::path::Path {
        self.library.cargo_manifest_path()
    }

    pub fn target_directory(&self) -> &std::path::Path {
        &self.target_directory
    }

    pub fn cargo_args(&self) -> &LibraryCargoArgs {
        &self.cargo_args
    }

    pub fn artifact_name(&self) -> &str {
        self.library.artifact_name()
    }

    pub fn toolchain_selector(&self) -> Option<&str> {
        self.toolchain_selector.as_deref()
    }

    pub(crate) fn selected_library(&self) -> &SelectedLibrary {
        &self.library
    }

    pub fn configure_rustc(&self, command: &mut Command) -> Result<()> {
        command.envs(self.env()?);
        command.arg("--").arg("--cfg").arg(BINDING_EXPANSION_CFG);
        Ok(())
    }

    pub fn generation(&self) -> Generation {
        Generation::new(self.manifest_path())
            .cargo_args(self.cargo_args.iter().cloned())
            .cargo_toolchain_selector(self.toolchain_selector().map(str::to_owned))
    }

    fn from_metadata(
        config: &Config,
        cargo: &Cargo,
        metadata: &CargoMetadata,
        cargo_manifest_path: &std::path::Path,
        cargo_args: LibraryCargoArgs,
    ) -> Result<Self> {
        let preferred_artifact = config
            .package
            .crate_name
            .as_ref()
            .map(|_| config.crate_artifact_name());
        let library = SelectedLibrary::resolve(
            config,
            cargo,
            metadata,
            cargo_manifest_path,
            preferred_artifact.as_deref(),
        )?;

        Ok(Self {
            library,
            target_directory: metadata.target_directory.clone(),
            cargo_args,
            toolchain_selector: cargo.toolchain_selector().map(str::to_owned),
        })
    }

    fn env(&self) -> Result<Vec<(OsString, OsString)>> {
        let root =
            self.library
                .manifest_path()
                .parent()
                .ok_or_else(|| CliError::CommandFailed {
                    command: format!(
                        "manifest path '{}' has no parent directory",
                        self.library.manifest_path().display()
                    ),
                    status: None,
                })?;

        Ok(vec![
            (BINDING_EXPANSION_BUILD_ENV.into(), "1".into()),
            (
                BINDING_EXPANSION_ROOT_ENV.into(),
                root.as_os_str().to_owned(),
            ),
            (
                BINDING_EXPANSION_SOURCE_ENV.into(),
                self.library.source_path().as_os_str().to_owned(),
            ),
            (
                BINDING_EXPANSION_SURFACE_ENV.into(),
                BindingMetadataSurface::Native.as_str().into(),
            ),
        ])
    }
}

#[cfg(test)]
impl BindingExpansion {
    pub(crate) fn fixture(
        cargo_manifest_path: impl AsRef<std::path::Path>,
        package_manifest_path: impl AsRef<std::path::Path>,
        cargo_args: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            library: SelectedLibrary::fixture("demo", package_manifest_path, "demo")
                .fixture_cargo_manifest(cargo_manifest_path),
            target_directory: PathBuf::from("/external/workspace/target"),
            cargo_args: LibraryCargoArgs::parse(cargo_args).unwrap(),
            toolchain_selector: Some("+nightly".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::process::Command;

    use boltffi_bindgen::cargo::{LibraryCargoArgs, LibraryCargoArgsError};
    use boltffi_binding::{
        BINDING_EXPANSION_BUILD_ENV, BINDING_EXPANSION_ROOT_ENV, BINDING_EXPANSION_SOURCE_ENV,
        BINDING_EXPANSION_SURFACE_ENV,
    };

    use super::BindingExpansion;
    use crate::cargo::fixture::{CargoMetadataFixture, CargoPackageFixture, CargoTargetFixture};
    use crate::cargo::{Cargo, CargoCrateType, SelectedLibrary};
    use crate::cli::CliError;
    use crate::config::{CargoConfig, Config, PackageConfig, TargetsConfig};

    fn expansion() -> BindingExpansion {
        let crate_root = std::env::temp_dir().join("boltffi-expansion-test");
        BindingExpansion {
            library: SelectedLibrary::fixture(
                "demo-ffi",
                crate_root.join("Cargo.toml"),
                "demo_ffi",
            ),
            target_directory: crate_root.join("target"),
            cargo_args: LibraryCargoArgs::parse(["--features".to_string(), "ffi".to_string()])
                .unwrap(),
            toolchain_selector: Some("+nightly".to_string()),
        }
    }

    #[test]
    fn retains_selected_artifact_name() {
        assert_eq!(expansion().artifact_name(), "demo_ffi");
    }

    #[test]
    fn rejects_incompatible_library_arguments_before_cargo_metadata() {
        let config = Config {
            experimental: Vec::new(),
            cargo: CargoConfig::default(),
            package: PackageConfig {
                name: "demo".to_string(),
                crate_name: None,
                version: None,
                description: None,
                license: None,
                repository: None,
            },
            targets: TargetsConfig::default(),
        };

        let error = BindingExpansion::resolve(&config, &["--all-targets".to_string()])
            .expect_err("target-set selection must fail before Cargo metadata");

        assert!(matches!(
            error,
            CliError::LibraryCargoArgs(LibraryCargoArgsError::TargetSet { argument })
                if argument == "--all-targets"
        ));
    }

    #[test]
    fn resolves_hyphenated_configured_library_to_exact_cargo_package_and_artifact() {
        let cargo_manifest_path = std::path::Path::new("/tmp/workspace/Cargo.toml");
        let selected_manifest_path = std::path::Path::new("/tmp/workspace/ffi/Cargo.toml");
        let metadata = CargoMetadataFixture::new("/tmp/target")
            .package(
                CargoPackageFixture::workspace_package(
                    "ffi-package",
                    selected_manifest_path,
                    "1.2.3",
                )
                .target(CargoTargetFixture::library(
                    "ffi_artifact",
                    [CargoCrateType::StaticLib, CargoCrateType::Cdylib],
                )),
            )
            .package(
                CargoPackageFixture::workspace_package(
                    "other-package",
                    "/tmp/workspace/other/Cargo.toml",
                    "1.2.3",
                )
                .target(CargoTargetFixture::library(
                    "other_artifact",
                    [CargoCrateType::StaticLib],
                )),
            )
            .metadata();
        let cargo = Cargo::in_working_directory(
            "/tmp/workspace".into(),
            &[
                "--manifest-path".to_string(),
                cargo_manifest_path.display().to_string(),
            ],
        );
        let config = Config {
            experimental: Vec::new(),
            cargo: CargoConfig::default(),
            package: PackageConfig {
                name: "distribution-name".to_string(),
                crate_name: Some("ffi-artifact".to_string()),
                version: None,
                description: None,
                license: None,
                repository: None,
            },
            targets: TargetsConfig::default(),
        };

        let expansion = BindingExpansion::from_metadata(
            &config,
            &cargo,
            &metadata,
            cargo_manifest_path,
            LibraryCargoArgs::default(),
        )
        .expect("hyphenated configured library should select its normalized Cargo artifact");

        assert_eq!(expansion.artifact_name(), "ffi_artifact");
        assert_eq!(expansion.manifest_path(), selected_manifest_path);
        assert_eq!(
            expansion.target_directory(),
            std::path::Path::new("/tmp/target")
        );
        assert!(expansion.package_id().contains("#ffi-package@1.2.3"));
    }

    #[test]
    fn retains_selected_rustup_toolchain() {
        assert_eq!(expansion().toolchain_selector(), Some("+nightly"));
    }

    #[test]
    fn configures_cargo_command_with_selected_expansion() {
        let expansion = expansion();
        let mut command = Command::new("cargo");
        expansion
            .configure_rustc(&mut command)
            .expect("expansion should configure command");
        let configured = command.get_envs().collect::<Vec<_>>();
        let manifest_path = expansion.manifest_path();
        let crate_root = manifest_path
            .parent()
            .expect("manifest should have a parent")
            .as_os_str()
            .to_owned();
        let source_path = expansion
            .selected_library()
            .source_path()
            .as_os_str()
            .to_owned();

        [
            (BINDING_EXPANSION_BUILD_ENV, OsString::from("1")),
            (BINDING_EXPANSION_ROOT_ENV, crate_root),
            (BINDING_EXPANSION_SOURCE_ENV, source_path),
            (BINDING_EXPANSION_SURFACE_ENV, OsString::from("native")),
        ]
        .into_iter()
        .for_each(|(expected_key, expected_value)| {
            assert!(configured.iter().any(|(key, value)| {
                *key == OsStr::new(expected_key) && *value == Some(expected_value.as_os_str())
            }));
        });
        assert!(configured.iter().all(|(key, _)| {
            *key != OsStr::new("RUSTFLAGS") && *key != OsStr::new("CARGO_ENCODED_RUSTFLAGS")
        }));
        assert_eq!(
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["--", "--cfg", "boltffi_binding_expansion"]
        );
    }
}
