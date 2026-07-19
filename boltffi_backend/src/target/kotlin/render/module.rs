use std::collections::HashSet;

use askama::Template as AskamaTemplate;

use boltffi_binding::{BuiltinType, DeclarationRef, Native};

use crate::{
    bridge::jni::JniBridgeContract,
    core::{
        Diagnostic, Error, FilePath, GeneratedFile, GeneratedOutput, RenderContext,
        RenderedDeclaration, Result,
    },
    target::{
        jvm::{
            DesktopLoader, NativeLibraries,
            resource::{PLATFORMS, Platform},
        },
        kotlin::{
            KotlinApiStyle, KotlinHost, KotlinPackage,
            render::{
                closure::Closures,
                native::{NativeFunction, NativeMethods},
            },
            syntax::Literal,
        },
    },
};

#[derive(AskamaTemplate)]
#[template(path = "target/kotlin/module.kt", escape = "none")]
struct ModuleTemplate {
    package: KotlinPackage,
    // Only `Some` when it differs from `package` — drives the `import` that lets classes/
    // callbacks/functions whose signatures reference a record/enum (rendered by-value into
    // `data_package`) resolve them, and widens the runtime helpers records/enums call back into
    // (`Native`, `Utf8Codec`, ...) from file-private to module-internal so the split-out data
    // file can still reach them. See `ModuleDataTemplate::ffi_package`'s doc comment for the
    // mirror-image case.
    data_package: Option<KotlinPackage>,
    native_libraries: LibraryLiterals,
    resource_platforms: &'static [Platform],
    runtime: String,
    closures: String,
    native_functions: Vec<NativeFunction>,
    declarations: String,
    async_runtime: bool,
    stream_runtime: bool,
}

#[derive(AskamaTemplate)]
#[template(path = "target/kotlin/module_data.kt", escape = "none")]
struct ModuleDataTemplate {
    package: KotlinPackage,
    // Only `Some` (and only ever rendered) when a split is active — see `ModuleTemplate::
    // data_package`'s doc comment for the mirror-image case. Imported so records/enums' own
    // `#[data(impl)]`/initializer methods can still reach `Native`/`WireReader`/`WireWriter`/
    // `Utf8Codec`, which stay declared in `ffi_package`.
    ffi_package: KotlinPackage,
    declarations: String,
}

#[derive(AskamaTemplate)]
#[template(path = "target/kotlin/runtime.kt", escape = "none")]
struct RuntimeTemplate {
    record_vectors: bool,
    // Widens the file-private codec helpers (`Utf8Codec`, `DirectVectorCodec`, `WireWriterPool`)
    // records/enums call into from `private` (file-scoped) to `internal` (module-scoped) — only
    // needed, and only rendered, when records/enums render into a separate file (see
    // `ModuleTemplate::data_package`). `false` reproduces today's byte-for-byte output.
    split_data_package: bool,
}

#[derive(AskamaTemplate)]
#[template(path = "target/kotlin/runtime/result.kt", escape = "none")]
struct ResultRuntimeTemplate {
    // See `RuntimeTemplate::split_data_package`'s doc comment — a record/enum field of a
    // `Result<T, E>` shape calls these extension functions too.
    split_data_package: bool,
}

#[derive(AskamaTemplate)]
#[template(path = "target/kotlin/runtime/builtin.kt", escape = "none")]
struct BuiltinRuntimeTemplate {
    // See `RuntimeTemplate::split_data_package`'s doc comment — records/enums with a
    // `Duration`/`SystemTime`/`Uuid`/`Url` field call these extension functions too.
    split_data_package: bool,
}

#[derive(AskamaTemplate)]
#[template(path = "target/kotlin/runtime/async.kt", escape = "none")]
struct AsyncRuntimeTemplate;

#[derive(AskamaTemplate)]
#[template(path = "target/kotlin/runtime/stream.kt", escape = "none")]
struct StreamRuntimeTemplate;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RuntimeFeatures {
    asynchronous: bool,
    streaming: bool,
    builtin: bool,
    result: bool,
    record_vectors: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LibraryLiterals {
    android: Literal,
    desktop_jni: Literal,
    desktop_fallback: Literal,
    desktop_loader: DesktopLoader,
}

impl LibraryLiterals {
    fn new(libraries: &NativeLibraries) -> Self {
        Self {
            android: Literal::string(libraries.android().as_str()),
            desktop_jni: Literal::string(libraries.desktop_jni().as_str()),
            desktop_fallback: Literal::string(libraries.desktop_fallback().as_str()),
            desktop_loader: libraries.desktop_loader(),
        }
    }

    fn android(&self) -> &Literal {
        &self.android
    }

    fn desktop_jni(&self) -> &Literal {
        &self.desktop_jni
    }

    fn desktop_fallback(&self) -> &Literal {
        &self.desktop_fallback
    }

    fn bundled_desktop_loader(&self) -> bool {
        self.desktop_loader.loads_bundled()
    }

    fn system_desktop_loader(&self) -> bool {
        self.desktop_loader.loads_system()
    }
}

pub struct Module<'host, 'bridge, 'decl> {
    host: &'host KotlinHost,
    bridge: &'bridge JniBridgeContract,
    context: &'decl RenderContext<'decl, Native>,
    declarations: Vec<RenderedDeclaration<'decl, Native>>,
}

impl<'host, 'bridge, 'decl> Module<'host, 'bridge, 'decl> {
    pub fn new(
        host: &'host KotlinHost,
        bridge: &'bridge JniBridgeContract,
        context: &'decl RenderContext<'decl, Native>,
        declarations: Vec<RenderedDeclaration<'decl, Native>>,
    ) -> Self {
        Self {
            host,
            bridge,
            context,
            declarations,
        }
    }

    pub fn render(self) -> Result<GeneratedOutput> {
        let diagnostics = self.diagnostics();
        let native_functions = self.native_functions()?;
        let closures = self.closures()?;
        let features = RuntimeFeatures::from_declarations(&self.declarations);
        let package = self.host.package().clone();
        let data_package = self.host.resolved_data_package().clone();
        let split = data_package != package;

        let ffi_declarations = if split {
            self.ffi_declarations()?
        } else {
            self.declarations()?
        };
        let ffi_contents = ModuleTemplate {
            package: package.clone(),
            data_package: split.then(|| data_package.clone()),
            native_libraries: LibraryLiterals::new(self.host.native_libraries()),
            resource_platforms: PLATFORMS,
            runtime: Runtime::new(features, split).render()?,
            closures,
            native_functions,
            declarations: ffi_declarations,
            async_runtime: features.asynchronous,
            stream_runtime: features.streaming,
        }
        .render()?;
        let mut files = vec![GeneratedFile::new(
            FilePath::new(self.host.file().path(&package))?,
            ffi_contents,
        )];

        if split {
            let data_declarations = self.data_declarations()?;
            let data_contents = ModuleDataTemplate {
                package: data_package.clone(),
                ffi_package: package,
                declarations: data_declarations,
            }
            .render()?;
            files.push(GeneratedFile::new(
                FilePath::new(self.host.file().path(&data_package))?,
                data_contents,
            ));
        }

        Ok(GeneratedOutput::new(files, diagnostics))
    }

    fn native_functions(&self) -> Result<Vec<NativeFunction>> {
        let methods = NativeMethods::new(self.bridge);
        let functions = self
            .declarations
            .iter()
            .filter(|declaration| !declaration.emitted().primary_chunk().is_empty())
            .map(|declaration| match declaration.declaration() {
                DeclarationRef::Function(function) => methods.function(function),
                DeclarationRef::Record(record) => methods.record(record),
                DeclarationRef::Enum(enumeration) => methods.enumeration(enumeration),
                DeclarationRef::Class(class) => methods.class(class),
                DeclarationRef::Callback(callback) => methods.callback(callback),
                DeclarationRef::Stream(stream) => methods.stream(stream),
                DeclarationRef::Constant(constant) => methods.constant(constant),
                _ => Ok(Vec::new()),
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .chain(methods.callback_handle_lifecycle()?)
            .chain(methods.callback_completions()?)
            .chain(methods.success_out_writers()?)
            .collect::<Vec<_>>();
        Self::unique_native_functions(functions)
    }

    fn closures(&self) -> Result<String> {
        Ok(
            Closures::from_declarations(&self.declarations, self.host, self.bridge, self.context)?
                .render()?
                .into_iter()
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    }

    fn unique_native_functions(functions: Vec<NativeFunction>) -> Result<Vec<NativeFunction>> {
        let mut names = HashSet::new();
        functions
            .into_iter()
            .try_fold(Vec::new(), |mut unique, function| {
                if names.insert(function.name().clone()) {
                    unique.push(function);
                    Ok(unique)
                } else {
                    Err(Error::KotlinNameCollision {
                        scope: "Native".to_owned(),
                        name: function.name().to_string(),
                    })
                }
            })
    }

    fn functions(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::Function(_))
        })
    }

    fn records(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::Record(_))
        })
    }

    fn enumerations(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::Enum(_))
        })
    }

    fn classes(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::Class(_))
        })
    }

    fn callbacks(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::Callback(_))
        })
    }

    fn streams(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::Stream(_))
        })
    }

    fn constants(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::Constant(_))
        })
    }

    fn custom_types(&self) -> Result<Vec<String>> {
        self.primary_chunks(|declaration| {
            matches!(declaration.declaration(), DeclarationRef::CustomType(_))
        })
    }

    fn api_declarations(&self, declarations: String) -> String {
        match self.host.api_layout() {
            KotlinApiStyle::TopLevel => declarations,
            KotlinApiStyle::ModuleObject => format!(
                "object {} {{\n{}\n}}",
                self.host.file(),
                Self::indent_declarations(declarations)
            ),
        }
    }

    fn indent_declarations(declarations: String) -> String {
        declarations
            .lines()
            .map(|line| match line.is_empty() {
                true => String::new(),
                false => format!("    {line}"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn declarations(&self) -> Result<String> {
        match self.host.api_layout() {
            KotlinApiStyle::TopLevel => self.all_declarations(),
            KotlinApiStyle::ModuleObject => {
                let callbacks = self.callbacks()?.join("\n\n");
                let declarations = self.api_declarations(self.object_declarations()?);
                Ok([callbacks, declarations]
                    .into_iter()
                    .filter(|chunk| !chunk.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n"))
            }
        }
    }

    fn all_declarations(&self) -> Result<String> {
        Ok(Self::join_declarations([
            self.custom_types()?,
            self.records()?,
            self.enumerations()?,
            self.callbacks()?,
            self.classes()?,
            self.streams()?,
            self.constants()?,
            self.functions()?,
        ]))
    }

    /// Every non-DTO declaration kind — the ffi-package file's content when a data-package split
    /// is active (records/enums render into `data_declarations` instead). Mirrors
    /// `all_declarations`/`object_declarations` minus the record/enumeration groups.
    fn ffi_declarations(&self) -> Result<String> {
        match self.host.api_layout() {
            KotlinApiStyle::TopLevel => Ok(Self::join_declarations([
                self.custom_types()?,
                self.callbacks()?,
                self.classes()?,
                self.streams()?,
                self.constants()?,
                self.functions()?,
            ])),
            KotlinApiStyle::ModuleObject => {
                let callbacks = self.callbacks()?.join("\n\n");
                let declarations = self.api_declarations(Self::join_declarations([
                    self.custom_types()?,
                    self.classes()?,
                    self.streams()?,
                    self.constants()?,
                    self.functions()?,
                ]));
                Ok([callbacks, declarations]
                    .into_iter()
                    .filter(|chunk| !chunk.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n"))
            }
        }
    }

    /// The DTO surface (records/enums) — the data-package file's content when a split is active.
    fn data_declarations(&self) -> Result<String> {
        Ok(Self::join_declarations([
            self.records()?,
            self.enumerations()?,
        ]))
    }

    fn object_declarations(&self) -> Result<String> {
        Ok(Self::join_declarations([
            self.custom_types()?,
            self.records()?,
            self.enumerations()?,
            self.classes()?,
            self.streams()?,
            self.constants()?,
            self.functions()?,
        ]))
    }

    fn join_declarations<const COUNT: usize>(groups: [Vec<String>; COUNT]) -> String {
        groups
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn primary_chunks(
        &self,
        include: impl Fn(&RenderedDeclaration<'decl, Native>) -> bool,
    ) -> Result<Vec<String>> {
        self.declarations
            .iter()
            .filter(|declaration| {
                let chunk = declaration.emitted().primary_chunk();
                include(declaration) && !chunk.is_empty()
            })
            .map(Self::primary_chunk)
            .collect()
    }

    fn primary_chunk(declaration: &RenderedDeclaration<'decl, Native>) -> Result<String> {
        Ok(declaration
            .emitted()
            .primary_chunk()
            .as_str()
            .trim_end()
            .to_owned())
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.declarations
            .iter()
            .flat_map(|declaration| declaration.emitted().diagnostics().iter().cloned())
            .collect()
    }
}

struct Runtime {
    features: RuntimeFeatures,
    split_data_package: bool,
}

impl Runtime {
    fn new(features: RuntimeFeatures, split_data_package: bool) -> Self {
        Self {
            features,
            split_data_package,
        }
    }

    fn render(self) -> Result<String> {
        let mut blocks = vec![
            RuntimeTemplate {
                record_vectors: self.features.record_vectors,
                split_data_package: self.split_data_package,
            }
            .render()?,
        ];
        if self.features.asynchronous {
            blocks.push(AsyncRuntimeTemplate.render()?);
        }
        if self.features.streaming {
            blocks.push(StreamRuntimeTemplate.render()?);
        }
        if self.features.builtin {
            blocks.push(
                BuiltinRuntimeTemplate {
                    split_data_package: self.split_data_package,
                }
                .render()?,
            );
        }
        if self.features.result {
            blocks.push(
                ResultRuntimeTemplate {
                    split_data_package: self.split_data_package,
                }
                .render()?,
            );
        }
        Ok(blocks.join("\n\n"))
    }
}

impl RuntimeFeatures {
    fn from_declarations(declarations: &[RenderedDeclaration<'_, Native>]) -> Self {
        Self {
            asynchronous: declarations.iter().any(|declaration| {
                declaration.declaration().uses_async_execution()
                    || matches!(declaration.declaration(), DeclarationRef::Stream(_))
            }),
            streaming: declarations
                .iter()
                .any(|declaration| matches!(declaration.declaration(), DeclarationRef::Stream(_))),
            builtin: declarations
                .iter()
                .any(|declaration| Self::declaration_uses_builtin(declaration.declaration())),
            result: declarations
                .iter()
                .any(|declaration| declaration.declaration().uses_result_codec()),
            record_vectors: declarations
                .iter()
                .any(|declaration| declaration.declaration().uses_direct_record_vector()),
        }
    }

    fn declaration_uses_builtin(declaration: DeclarationRef<'_, Native>) -> bool {
        [
            BuiltinType::Duration,
            BuiltinType::SystemTime,
            BuiltinType::Uuid,
            BuiltinType::Url,
        ]
        .into_iter()
        .any(|kind| declaration.uses_builtin_codec(kind))
    }
}
