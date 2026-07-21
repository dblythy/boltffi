use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use boltffi_ast::PackageInfo;
use boltffi_binding::{
    BINDING_METADATA_BUILD_ENV, BINDING_METADATA_FEATURES_ENV,
    BINDING_METADATA_NAMING_STYLE_ENV, BINDING_METADATA_ROOT_ENV, BINDING_METADATA_SOURCE_ENV,
    BINDING_METADATA_SURFACE_ENV, BindingMetadataSurface, LowerError, NamingStyle, Native,
    SerializedBindings, Wasm32, lower_with_declarations, lower_with_declarations_and_style,
};
use boltffi_scan::{ActiveCfg, ScanError, ScanInput};
use proc_macro2::{Span, TokenStream};
use quote::quote_spanned;

use crate::experimental::{error::Error as MetadataError, metadata};

pub enum Item {
    Inactive,
    Preserve,
    Tokens(TokenStream),
    Error(TokenStream),
}

static EMITTED: AtomicBool = AtomicBool::new(false);

pub fn item() -> Item {
    if env::var_os(BINDING_METADATA_BUILD_ENV).is_none() {
        return Item::Inactive;
    }

    match Request::from_env() {
        Ok(Some(_)) if EMITTED.swap(true, Ordering::AcqRel) => Item::Preserve,
        Ok(Some(request)) => request
            .render()
            .map(Item::Tokens)
            .unwrap_or_else(|error| Item::Error(error.into_compile_error())),
        Ok(None) => Item::Inactive,
        Err(error) => Item::Error(error.into_compile_error()),
    }
}

struct Request {
    root: PathBuf,
    source: PathBuf,
    package: PackageInfo,
}

impl Request {
    fn from_env() -> Result<Option<Self>, BuildError> {
        if !compiling_generated_crate()? {
            return Ok(None);
        }

        Ok(Some(Self {
            root: generated_root()?,
            source: PathBuf::from(required_env(BINDING_METADATA_SOURCE_ENV)?),
            package: PackageInfo::new(
                required_env("CARGO_PKG_NAME")?,
                env::var("CARGO_PKG_VERSION")
                    .ok()
                    .filter(|version| !version.is_empty()),
            ),
        }))
    }

    fn render(self) -> Result<TokenStream, BuildError> {
        let scan = boltffi_scan::scan_package(
            &ScanInput::new(&self.source, self.package)
                .with_manifest_dir(&self.root)
                .with_cfg(active_cfg()),
        )?;
        let source = scan.root_with_support();
        match requested_surface()? {
            // The `Native` surface's embedded metadata is read by every
            // native-target renderer (Java, Kotlin, Swift, C#, ...) to
            // describe a crate's real, compiled entry points — but different
            // `boltffi_cli` pack commands build that real artifact
            // differently (`apple`/`android`/`kotlin_multiplatform` route
            // through the experimental macro expansion themselves via
            // `BindingExpansion`; `csharp` does a plain `cargo build`), so
            // there is no single style that's correct for every caller. The
            // caller (`BindingMetadataBuild`) says which one via
            // `BINDING_METADATA_NAMING_STYLE_ENV`, defaulting to
            // `Experimental` — the style every existing caller already
            // implicitly relied on before this env var existed. See
            // `NamingStyle`'s doc for why getting this wrong is a silent,
            // compiles-clean-but-`EntryPointNotFoundException`-at-runtime bug
            // (`docs/tracks/boltffi-fork.md`, the C#-`LegacyCompatible` case
            // this env var was introduced to fix).
            BindingMetadataSurface::Native => {
                let native = lower_with_declarations_and_style::<Native>(&source, requested_naming_style())?;
                metadata::render(SerializedBindings::native(native.into_bindings()))
                    .map_err(Into::into)
            }
            // Left on `NamingStyle::Experimental` (unchanged): no current
            // `boltffi_bindgen` caller reads this surface's metadata (grep
            // confirms zero `self.bindings::<Wasm32>()` call sites), so the
            // identical divergence this fix closes for `Native` is dormant
            // here, not exercised — not "no divergence exists in principle"
            // (the stable macro's wasm export path,
            // `boltffi_macros/src/exports/function.rs`, mints the same
            // short `boltffi_ffi_rules::naming`-based scheme `Native` does).
            // Widen this the same way if a wasm-target renderer starts
            // reading it.
            BindingMetadataSurface::Wasm32 => {
                let wasm32 = lower_with_declarations::<Wasm32>(&source)?;
                metadata::render(SerializedBindings::wasm32(wasm32.into_bindings()))
                    .map_err(Into::into)
            }
        }
    }
}

enum BuildError {
    MissingEnv(&'static str),
    InvalidSurface(String),
    Scan(ScanError),
    Lower(LowerError),
    Metadata(MetadataError),
}

impl BuildError {
    fn into_compile_error(self) -> TokenStream {
        let message = self.to_string();
        quote_spanned! { Span::call_site() =>
            compile_error!(#message);
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEnv(key) => {
                write!(formatter, "BoltFFI metadata build: `{key}` is not set")
            }
            Self::InvalidSurface(surface) => {
                write!(
                    formatter,
                    "BoltFFI metadata build: `{BINDING_METADATA_SURFACE_ENV}` has invalid value `{surface}`"
                )
            }
            Self::Scan(error) => write!(formatter, "BoltFFI metadata build scan failed: {error}"),
            Self::Lower(error) => write!(formatter, "BoltFFI metadata build lower failed: {error}"),
            Self::Metadata(error) => {
                write!(formatter, "BoltFFI metadata build emission failed: {error}")
            }
        }
    }
}

impl From<ScanError> for BuildError {
    fn from(error: ScanError) -> Self {
        Self::Scan(error)
    }
}

impl From<LowerError> for BuildError {
    fn from(error: LowerError) -> Self {
        Self::Lower(error)
    }
}

impl From<MetadataError> for BuildError {
    fn from(error: MetadataError) -> Self {
        Self::Metadata(error)
    }
}

fn required_env(key: &'static str) -> Result<String, BuildError> {
    env::var(key).map_err(|_| BuildError::MissingEnv(key))
}

fn requested_surface() -> Result<BindingMetadataSurface, BuildError> {
    let surface = required_env(BINDING_METADATA_SURFACE_ENV)?;
    BindingMetadataSurface::parse(&surface).ok_or(BuildError::InvalidSurface(surface))
}

/// Absent or unparseable defaults to [`NamingStyle::Experimental`] — every
/// caller that predates this env var (every one except `BindingMetadataBuild`
/// callers that opt in) gets the exact behavior it always had.
fn requested_naming_style() -> NamingStyle {
    env::var(BINDING_METADATA_NAMING_STYLE_ENV)
        .ok()
        .and_then(|value| NamingStyle::parse(&value))
        .unwrap_or_default()
}

fn compiling_generated_crate() -> Result<bool, BuildError> {
    let manifest_dir = PathBuf::from(required_env("CARGO_MANIFEST_DIR")?);
    Ok(canonical(&manifest_dir) == canonical(&generated_root()?))
}

fn generated_root() -> Result<PathBuf, BuildError> {
    Ok(PathBuf::from(required_env(BINDING_METADATA_ROOT_ENV)?))
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn active_cfg() -> ActiveCfg {
    let features = env::var(BINDING_METADATA_FEATURES_ENV)
        .ok()
        .into_iter()
        .flat_map(|features| {
            features
                .split(',')
                .filter(|feature| !feature.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
    ActiveCfg::from_cargo_env().with_features(features)
}
