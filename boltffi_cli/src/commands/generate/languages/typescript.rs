use boltffi_bindgen::render::typescript::{
    TypeScriptEmitter, TypeScriptLowerError, TypeScriptLowerer,
};

use boltffi_bindgen::target::Target;

use crate::cli::{CliError, Result};
use crate::commands::generate::generator::{GenerateRequest, LanguageGenerator, ScanPointerWidth};

pub struct TypeScriptGenerator;

impl LanguageGenerator for TypeScriptGenerator {
    const TARGET: Target = Target::TypeScript;

    fn generate(request: &GenerateRequest<'_>) -> Result<()> {
        if !request.config().is_wasm_enabled() {
            return Err(CliError::CommandFailed {
                command: "targets.wasm.enabled = false".to_string(),
                status: None,
            });
        }

        let output_directory = request
            .output_override()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| request.config().wasm_typescript_output());
        let module_name = request.config().wasm_typescript_module_name();
        let browser_output_path = output_directory.join(format!("{module_name}.ts"));
        let node_output_path = output_directory.join(format!("{module_name}_node.ts"));

        request.ensure_output_directory(&output_directory)?;

        // `native_async` targets a real `BindingExpansion`-built native artifact
        // (dist/apple, dist/android) — long, module-qualified symbol names.
        // Plain wasm codegen must keep the stable macro path's short names
        // (byte-identical output). See runtime/cpp/README.md's reconciliation
        // note (boltffi fork) for why the two diverge.
        let naming_style = if request.config().typescript_experimental().native_async {
            boltffi_binding::NamingStyle::Experimental
        } else {
            boltffi_binding::NamingStyle::LegacyCompatible
        };
        let lowered_crate = request
            .lowered_crate_with_naming_style(ScanPointerWidth::Fixed(32), naming_style)?;
        let type_script_module = TypeScriptLowerer::new(
            &lowered_crate.ffi_contract,
            &lowered_crate.abi_contract,
            request.source_crate().crate_name().to_string(),
            request.config().typescript_experimental(),
        )
        .lower()
        .map_err(|error| match error {
            TypeScriptLowerError::ValueTypeMemberNameCollision { .. }
            | TypeScriptLowerError::TopLevelFunctionNameCollision { .. } => {
                CliError::CommandFailed {
                    command: format!("generate typescript: {error}"),
                    status: None,
                }
            }
        })?;
        let runtime_package = request.config().wasm_runtime_package();
        let browser_source = TypeScriptEmitter::emit(&type_script_module).replacen(
            "from \"@boltffi/runtime\"",
            &format!("from \"{}\"", runtime_package),
            1,
        );
        let node_source = TypeScriptEmitter::emit_node(&type_script_module, &module_name).replacen(
            "from \"@boltffi/runtime\"",
            &format!("from \"{}\"", runtime_package),
            1,
        );

        request.write_output(&browser_output_path, browser_source)?;
        request.write_output(&node_output_path, node_source)
    }
}
