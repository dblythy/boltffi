//! Regression coverage for two C# IR backend fixes:
//!
//! - The sibling-name-shadow fix (`Class::from_declaration` threading its
//!   `Namespace` into `Function::from_class_method`/`from_class_initializer`).
//!   A class method whose PascalCase name collides with the record/enum it
//!   decodes (`ServerInfo() -> Result<ServerInfo, _>`, the shape
//!   `parse-core-rs`'s real `ParseClient::server_info` reaches) must qualify
//!   its own `Decode` call with `global::<namespace>.` — C# member lookup
//!   otherwise resolves the bare type name to the enclosing method group
//!   (CS0119), not the type.
//! - The auto-generated disposal helper's collision-proof rename. Every
//!   generated class unconditionally emits a private disposal method; when
//!   that method was literally named `Release`, a real exported method also
//!   named `release` (a plausible resource-cleanup method name) collided
//!   with it at both the C# member level and the native P/Invoke import
//!   level (CS0111).

use std::{fs, path::Path, process::Command, time::UNIX_EPOCH};

use boltffi_ast::PackageInfo;
use boltffi_backend::{GeneratedOutput, Target, bridge::c::CBridge, target::csharp::CSharpHost};
use boltffi_binding::{Bindings, Native, lower};

fn bindings(source: &str) -> Bindings<Native> {
    let source = boltffi_scan::scan_file(
        syn::parse_str(source).expect("valid source"),
        PackageInfo::new("demo", None),
    )
    .expect("source should scan");
    lower::<Native>(&source).expect("source should lower")
}

fn target(host: CSharpHost) -> Target<CSharpHost, CBridge> {
    host.into_target().expect("C# target")
}

#[test]
fn csharp_target_qualifies_a_class_method_named_after_its_return_record() {
    let bindings = bindings(
        r#"
        #[data]
        pub struct ServerInfo {
            pub version: String,
        }

        pub struct ParseClient {
            id: i32,
        }

        #[export]
        impl ParseClient {
            pub fn new(id: i32) -> Self { Self { id } }

            pub fn server_info(&self) -> Result<ServerInfo, String> {
                Ok(ServerInfo { version: "1".to_string() })
            }
        }

        #[export]
        pub fn apply_server_info(
            f: impl Fn(ServerInfo) -> ServerInfo,
            value: ServerInfo,
        ) -> ServerInfo {
            f(value)
        }
        "#,
    );
    let output = target(
        CSharpHost::new()
            .namespace("Company.Bindings")
            .expect("valid namespace")
            .native_library("demo_native"),
    )
    .render(&bindings)
    .expect("a class method named after its return record should still render");

    assert!(
        output.diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        output.diagnostics()
    );

    let class = output
        .files()
        .iter()
        .find(|file| file.path().as_path() == Path::new("ParseClient.cs"))
        .map(|file| file.contents())
        .expect("generated ParseClient.cs");

    assert!(
        class.contains("return global::Company.Bindings.ServerInfo.Decode(resultReader);"),
        "expected the self-named ServerInfo() method to qualify its own Decode call:\n{class}"
    );
    assert!(
        !class.contains("return ServerInfo.Decode(resultReader);"),
        "an unqualified Decode call inside ServerInfo() would resolve to the method group, not \
         the type (CS0119):\n{class}"
    );

    compile_csharp_with_dotnet_when_available(&output, "csharp-sibling-shadow-smoke");
}

#[test]
fn csharp_target_qualifies_a_free_function_named_after_its_return_record() {
    // Adversarial review (fable model) of the class-only fix found this exact shape breaks for
    // a free function and a record method too — reproduced with a real `dotnet build` CS0119.
    // The qualification mechanism (direct_type_with/render_type_ref) has no per-collision
    // detection, so closing this means qualifying every direct record/enum reference reachable
    // from a free function or record/enum method, not just the colliding one — the same
    // trade-off `Enumeration::from_data` already accepts for data-enum methods.
    let bindings = bindings(
        r#"
        #[data]
        pub struct ServerInfo {
            pub version: String,
        }

        #[export]
        pub fn server_info() -> Result<ServerInfo, String> {
            Ok(ServerInfo { version: "1".to_string() })
        }

        #[export]
        pub fn apply_server_info(
            f: impl Fn(ServerInfo) -> ServerInfo,
            value: ServerInfo,
        ) -> ServerInfo {
            f(value)
        }
        "#,
    );
    let output = target(
        CSharpHost::new()
            .namespace("Company.Bindings")
            .expect("valid namespace")
            .native_library("demo_native"),
    )
    .render(&bindings)
    .expect("a free function named after its return record should still render");

    assert!(
        output.diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        output.diagnostics()
    );

    let module = output
        .files()
        .iter()
        .find(|file| file.path().as_path() == Path::new("Demo.cs"))
        .map(|file| file.contents())
        .expect("generated Demo.cs");

    assert!(
        module.contains("return global::Company.Bindings.ServerInfo.Decode(resultReader);"),
        "expected the self-named ServerInfo() free function to qualify its own Decode call:\n{module}"
    );

    compile_csharp_with_dotnet_when_available(&output, "csharp-free-function-shadow-smoke");
}

#[test]
fn csharp_target_does_not_collide_with_a_class_exporting_its_own_release_method() {
    let bindings = bindings(
        r#"
        pub struct Resource {
            id: i32,
        }

        #[export]
        impl Resource {
            pub fn new(id: i32) -> Self { Self { id } }

            pub fn release(&self) {}
        }
        "#,
    );
    let output = target(
        CSharpHost::new()
            .namespace("Company.Bindings")
            .expect("valid namespace")
            .native_library("demo_native"),
    )
    .render(&bindings)
    .expect("a class exporting its own release() method should still render");

    assert!(
        output.diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        output.diagnostics()
    );

    let class = output
        .files()
        .iter()
        .find(|file| file.path().as_path() == Path::new("Resource.cs"))
        .map(|file| file.contents())
        .expect("generated Resource.cs");

    assert!(
        class.contains("public void Release()"),
        "expected the user's own release() method to render:\n{class}"
    );
    assert!(
        class.contains("private void BoltFfiRelease()"),
        "expected the auto-generated disposal helper to be collision-proofed:\n{class}"
    );

    compile_csharp_with_dotnet_when_available(&output, "csharp-release-collision-smoke");
}

/// `[targets.csharp] data_namespace` (parse-core-sdks' consumer-facing namespace-leak fix):
/// records/enums (the DTO/public-contract surface) render into a namespace separate from
/// classes/callbacks (the handle surface), so a shim's own hand-written types never collide with
/// generated handle types of the same name. Verifies every declaration kind that can carry a
/// record/enum reference — a free function, a class method, a callback method, and a record's own
/// `#[data(impl)]` method calling back into the native runtime — still resolves once split, with a
/// real `dotnet build` including a standalone consumer file that reaches every DTO kind through
/// `using ParseCore;` alone, never `ParseCore.Ffi`.
#[test]
fn csharp_target_splits_data_records_and_enums_into_a_separate_namespace() {
    let bindings = bindings(
        r#"
        #[repr(u8)]
        #[data]
        pub enum Mode {
            Fast = 1,
            Slow = 2,
        }

        #[data]
        pub struct Profile {
            pub name: String,
            pub mode: Mode,
        }

        #[data(impl)]
        impl Profile {
            pub fn describe(&self) -> String { self.name.clone() }
        }

        #[data]
        pub enum Shape {
            Empty,
            Circle { radius: f64 },
        }

        pub struct ParseObject {
            id: i32,
        }

        #[export]
        impl ParseObject {
            pub fn new(id: i32) -> Self { Self { id } }
            pub fn profile(&self) -> Profile {
                Profile { name: "demo".to_string(), mode: Mode::Fast }
            }
        }

        #[export]
        pub trait ProfileListener {
            fn on_profile(&self, profile: Profile) -> Profile;
        }

        #[export]
        pub fn echo_profile(profile: Profile) -> Profile { profile }

        #[export]
        pub fn echo_shape(shape: Shape) -> Shape { shape }
        "#,
    );
    let output = target(
        CSharpHost::new()
            .namespace("ParseCore.Ffi")
            .expect("valid namespace")
            .data_namespace("ParseCore")
            .expect("valid data namespace")
            .native_library("demo_native"),
    )
    .render(&bindings)
    .expect("a split data/handle namespace should still render");

    assert!(
        output.diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        output.diagnostics()
    );

    let file = |name: &str| -> String {
        output
            .files()
            .iter()
            .find(|file| file.path().as_path() == Path::new(name))
            .map(|file| file.contents().to_owned())
            .unwrap_or_else(|| panic!("generated {name}"))
    };

    let profile = file("Profile.cs");
    assert!(
        profile.contains("namespace ParseCore\n"),
        "Profile (a #[data] record) should render into the data namespace:\n{profile}"
    );
    assert!(
        profile.contains("using ParseCore.Ffi;"),
        "Profile's own #[data(impl)] method needs the ffi namespace in scope for \
         NativeMethods/WireReader/WireWriter:\n{profile}"
    );

    let shape = file("Shape.cs");
    assert!(
        shape.contains("namespace ParseCore\n"),
        "Shape (data enum) should split too"
    );

    let mode = file("Mode.cs");
    assert!(
        mode.contains("namespace ParseCore\n"),
        "Mode (C-style data enum) should split too"
    );

    let object = file("ParseObject.cs");
    assert!(
        object.contains("namespace ParseCore.Ffi\n"),
        "the handle class itself stays on the ffi namespace:\n{object}"
    );
    assert!(
        object.contains("global::ParseCore.Profile"),
        "a class method returning a record must qualify against the DATA namespace, not the \
         ffi namespace it's declared in:\n{object}"
    );

    let listener = file("ProfileListener.cs");
    assert!(
        listener.contains("using ParseCore;"),
        "a callback interface referencing a record type in its signature needs the data \
         namespace imported, since callback signatures render type refs unqualified:\n{listener}"
    );
    assert!(
        listener.contains("Profile OnProfile(Profile profile);"),
        "the callback method signature itself stays unqualified (resolved via the `using`):\n{listener}"
    );

    let module = file("Demo.cs");
    assert!(
        module.contains("global::ParseCore.Profile EchoProfile(global::ParseCore.Profile profile)"),
        "a free function taking/returning a record must qualify against the data namespace:\n{module}"
    );
    assert!(
        module.contains("global::ParseCore.Shape EchoShape(global::ParseCore.Shape shape)"),
        "same for a data enum:\n{module}"
    );

    compile_csharp_with_dotnet_when_available_with_extra_source(
        &output,
        "csharp-data-namespace-split-smoke",
        "Consumer.cs",
        r#"using ParseCore;

namespace ParseCore.Consumer
{
    // The whole point of the split: a consumer only ever needs `using ParseCore;` — never
    // `ParseCore.Ffi` — to work with every generated DTO kind (record, enum, and a data record
    // with its own #[data(impl)] method).
    public static class Reach
    {
        public static Profile MakeProfile() => new Profile("demo", Mode.Fast);

        public static string Describe(Profile profile) => profile.Describe();

        public static Shape MakeShape() => new Shape.Circle(1.0);

        public static Mode MakeMode() => Mode.Slow;
    }
}
"#,
    );
}

/// Real `dotnet build` of the generated sources when a `dotnet` toolchain is
/// on `PATH` (skips cleanly otherwise, matching the Java backend's
/// `*_when_available` convention in `boltffi_backend/tests/java.rs`).
fn compile_csharp_with_dotnet_when_available(output: &GeneratedOutput, prefix: &str) {
    compile_csharp_with_dotnet_when_available_impl(output, prefix, None);
}

/// Same as [`compile_csharp_with_dotnet_when_available`], plus one extra hand-written consumer
/// source file compiled into the same project — proves real, non-generated code can consume the
/// generated output the way a shim/consumer actually would.
fn compile_csharp_with_dotnet_when_available_with_extra_source(
    output: &GeneratedOutput,
    prefix: &str,
    extra_file_name: &str,
    extra_source: &str,
) {
    compile_csharp_with_dotnet_when_available_impl(
        output,
        prefix,
        Some((extra_file_name, extra_source)),
    );
}

fn compile_csharp_with_dotnet_when_available_impl(
    output: &GeneratedOutput,
    prefix: &str,
    extra: Option<(&str, &str)>,
) {
    if Command::new("dotnet").arg("--version").output().is_err() {
        return;
    }

    let directory = std::env::temp_dir().join(format!(
        "{prefix}-{}",
        UNIX_EPOCH.elapsed().expect("system clock").as_nanos()
    ));
    let src = directory.join("src");
    fs::create_dir_all(&src).expect("create dotnet smoke src directory");
    for file in output
        .files()
        .iter()
        .filter(|file| file.path().as_path().extension().is_some_and(|ext| ext == "cs"))
    {
        let path = src.join(file.path().as_path());
        fs::create_dir_all(path.parent().expect("generated C# parent"))
            .expect("create generated C# parent directory");
        fs::write(&path, file.contents()).expect("write generated C# source");
    }
    if let Some((name, source)) = extra {
        fs::write(src.join(name), source).expect("write extra consumer source");
    }
    fs::write(
        directory.join("Smoke.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <Nullable>enable</Nullable>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
  </PropertyGroup>
</Project>
"#,
    )
    .expect("write smoke csproj");

    let build = Command::new("dotnet")
        .arg("build")
        .arg(directory.join("Smoke.csproj"))
        .arg("--nologo")
        .output()
        .expect("dotnet build should execute");
    assert!(
        build.status.success(),
        "generated C# failed to build:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    fs::remove_dir_all(&directory).expect("remove dotnet smoke directory");
}
