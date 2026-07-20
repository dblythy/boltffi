use crate::build::{
    BindingExpansion, BuildOptions, BuildResult, BuildSelection, Builder, all_successful,
    count_successful, failed_targets, resolve_build_profile,
};
use crate::cli::Result;
use crate::config::Config;
use crate::pack::PackError;

pub enum BuildPlatform {
    Apple,
    Android,
    Wasm,
    Dart,
    All,
}

pub struct BuildCommandOptions {
    pub platform: BuildPlatform,
    pub release: bool,
    pub cargo_args: Vec<String>,
}

pub fn run_build(config: &Config, options: BuildCommandOptions) -> Result<Vec<BuildResult>> {
    let BuildCommandOptions {
        platform,
        release,
        cargo_args: cli_cargo_args,
    } = options;

    let cargo_args: Vec<String> = config
        .cargo_args_for_command("build")
        .into_iter()
        .chain(cli_cargo_args)
        .collect();

    let build_profile = resolve_build_profile(release, &cargo_args);

    let profile = build_profile.output_directory_name();

    let results = match platform {
        BuildPlatform::Apple => {
            if !config.is_apple_enabled() {
                return Ok(Vec::new());
            }
            println!("Building for Apple ({})...", profile);
            expanded_builder(config, release, cargo_args.clone())?
                .build_targets(&config.apple_targets())?
        }
        BuildPlatform::Android => {
            if !config.is_android_enabled() {
                return Ok(Vec::new());
            }
            println!("Building for Android ({})...", profile);
            expanded_builder(config, release, cargo_args.clone())?
                .build_android(&config.android_targets())?
        }
        BuildPlatform::Wasm => {
            if !config.is_wasm_enabled() {
                return Ok(Vec::new());
            }
            println!("Building for wasm ({})...", profile);
            plain_builder(config, release, cargo_args.clone())
                .build_wasm_with_triple(config.wasm_triple())?
        }
        BuildPlatform::Dart => {
            if !config.is_dart_enabled() {
                return Ok(Vec::new());
            }
            println!("Building for dart ({})...", profile);
            plain_builder(config, release, cargo_args.clone())
                .build_targets(&config.dart_targets())?
        }
        BuildPlatform::All => {
            println!("Building all targets ({})...", profile);
            let mut all_results = Vec::new();
            if config.is_apple_enabled() {
                all_results.extend(
                    expanded_builder(config, release, cargo_args.clone())?
                        .build_targets(&config.apple_targets())?,
                );
            }
            if config.is_android_enabled() {
                all_results.extend(
                    expanded_builder(config, release, cargo_args.clone())?
                        .build_android(&config.android_targets())?,
                );
            }
            if config.is_wasm_enabled() {
                all_results.extend(
                    plain_builder(config, release, cargo_args.clone())
                        .build_wasm_with_triple(config.wasm_triple())?,
                );
            }
            if config.is_dart_enabled() {
                all_results.extend(
                    plain_builder(config, release, cargo_args.clone())
                        .build_targets(&config.dart_targets())?,
                );
            }
            all_results
        }
    };

    if results.is_empty() {
        println!("No enabled targets matched the requested platform");
        return Ok(results);
    }

    print_build_results(&results);

    if all_successful(&results) {
        Ok(results)
    } else {
        Err(PackError::BuildFailed {
            targets: failed_targets(&results),
        }
        .into())
    }
}

/// Builder for platforms whose real shipped artifact is built through the
/// experimental `BindingExpansion` macro path (`--cfg boltffi_binding_expansion`),
/// which mints long, module-path-qualified symbol names
/// (`boltffi_binding::lower::symbol::NamingStyle::Experimental`) — the scheme
/// every native-target renderer (Swift/Kotlin/JNI/Java/C#) mints by default.
///
/// Android must use this too, matching `pack android`'s own internal
/// `build_android_targets` (`boltffi_cli/src/pack/android/mod.rs`): before this,
/// `boltffi build android` alone used a plain `cargo build` (`BuildSelection::Default`,
/// the STABLE macro path's short-form scheme), so its real `.a`/`.so` never
/// exported the long-form symbols the Kotlin/JNI codegen always calls (callback
/// registration was the visibly broken lane — `boltffi_register_callback_*`
/// left `UND` in the linked `.so` — but every symbol category was equally
/// mismatched). `boltffi pack android` was never affected: it always built its
/// real artifact through `Expanded`, it was only the standalone `build android`
/// command that had drifted from it.
fn expanded_builder(config: &Config, release: bool, cargo_args: Vec<String>) -> Result<Builder<'_>> {
    let expansion = BindingExpansion::resolve(config, &cargo_args)?;
    Ok(Builder::new(
        config,
        build_options(release, BuildSelection::Expanded(expansion)),
    ))
}

fn plain_builder(config: &Config, release: bool, cargo_args: Vec<String>) -> Builder<'_> {
    Builder::new(
        config,
        build_options(release, BuildSelection::Default { cargo_args }),
    )
}

fn build_options(release: bool, selection: BuildSelection) -> BuildOptions {
    BuildOptions {
        release,
        selection,
        on_output: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{expanded_builder, plain_builder};
    use crate::build::BuildSelection;
    use crate::config::Config;
    use std::path::PathBuf;

    fn demo_manifest_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/demo/Cargo.toml")
    }

    fn demo_config() -> Config {
        let manifest = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/demo/boltffi.toml"),
        )
        .expect("read demo boltffi.toml");
        let config: Config = toml::from_str(&manifest).expect("parse demo boltffi.toml");
        config.validate().expect("validate demo boltffi.toml");
        config
    }

    fn manifest_path_cargo_args() -> Vec<String> {
        vec![
            "--manifest-path".to_string(),
            demo_manifest_path().display().to_string(),
        ]
    }

    // Regression for the Android/Kotlin naming-scheme mismatch: `boltffi build
    // android` used to request a plain `cargo build` (`BuildSelection::Default`,
    // the STABLE macro path's short-form symbol scheme) while the Kotlin/JNI
    // codegen it feeds always mints the experimental `BindingExpansion` scheme's
    // long-form names -- so the real linked `.so` never exported what the
    // generated JNI glue called (`boltffi_register_callback_*` etc left `UND`).
    // Android must request the SAME `Expanded` selection Apple does (and that
    // `pack android`'s own internal `build_android_targets` already does).
    #[test]
    fn android_and_apple_request_the_same_expanded_build_selection() {
        let config = demo_config();
        let cargo_args = manifest_path_cargo_args();

        let android = expanded_builder(&config, false, cargo_args.clone())
            .expect("android build selection should resolve BindingExpansion");
        let apple = expanded_builder(&config, false, cargo_args)
            .expect("apple build selection should resolve BindingExpansion");

        assert!(matches!(android.selection(), BuildSelection::Expanded(_)));
        assert!(matches!(apple.selection(), BuildSelection::Expanded(_)));
    }

    // Wasm/Dart deliberately stay on the plain `cargo build` path (their real
    // artifacts are not built through `BindingExpansion`) -- pinned so a future
    // edit doesn't accidentally widen `expanded_builder` to cover them too.
    #[test]
    fn wasm_and_dart_stay_on_the_plain_build_selection() {
        let config = demo_config();
        let cargo_args = manifest_path_cargo_args();

        let wasm = plain_builder(&config, false, cargo_args.clone());
        let dart = plain_builder(&config, false, cargo_args);

        assert!(matches!(
            wasm.selection(),
            BuildSelection::Default { .. } | BuildSelection::Package { .. }
        ));
        assert!(matches!(
            dart.selection(),
            BuildSelection::Default { .. } | BuildSelection::Package { .. }
        ));
    }
}

fn print_build_results(results: &[BuildResult]) {
    println!();

    results.iter().for_each(|result| {
        let icon = if result.success { "[ok]" } else { "[failed]" };
        println!("  {} {}", icon, result.triple);
    });

    println!();

    let success_count = count_successful(results);
    let total = results.len();

    if all_successful(results) {
        println!("Built {}/{} targets successfully", success_count, total);
    } else {
        println!(
            "Built {}/{} targets ({} failed)",
            success_count,
            total,
            total - success_count
        );
        println!();
        println!("Failed targets:");
        failed_targets(results).iter().for_each(|triple| {
            println!("  - {}", triple);
        });
    }
}
