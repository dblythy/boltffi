//! Regression coverage for the C# fallible-return ABI mismatch
//! (`docs/tracks/boltffi-fork.md`'s "CRITICAL ... FIXED" entry, parse-core-sdks
//! `worktree-csharp-rollin`): `pack csharp`'s native build used to be a plain
//! `cargo build`, compiling `boltffi_macros`'s stable macro path — whose
//! fallible-constructor ABI (a raw pointer return + thread-local last-error)
//! structurally disagrees with what `boltffi_backend`'s C# renderer generates
//! (`FfiBuf errorBuffer, out T result` — the experimental macro's real ABI,
//! what `apple`/`android`/`kmp`/`python` already build their real artifacts
//! with). The mismatch is undefined behavior at the P/Invoke boundary, so it
//! doesn't reproduce for every fallible shape (a `String` error happened to
//! decode correctly by chance in earlier manual probing) — a record/struct
//! error type (`#[error]`, matching `parse-core-rs`'s real `ParseError`)
//! reproduces it reliably: `OverflowException`/"corrupt wire" on the success
//! path, `corrupt wire: truncated i32` on the error path.
//!
//! Structural/compile-only tests (`boltffi_backend/tests/csharp.rs`) and the
//! symbol-name tests (`boltffi_bindgen/src/metadata.rs`) can't catch this —
//! neither actually invokes a fallible P/Invoke call against a real compiled
//! dylib. This test does: a real fixture crate, packed with the real `boltffi`
//! binary, invoked from a real `dotnet run` consumer, asserting both the
//! success AND the error branch of a fallible constructor decode correctly.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("boltffi_cli has a workspace parent")
        .to_path_buf()
}

fn tool_available(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok()
}

#[test]
fn csharp_pack_round_trips_a_fallible_constructor_through_a_real_dotnet_consumer() {
    if !tool_available("dotnet") {
        return;
    }

    let root = tempfile::Builder::new()
        .prefix("boltffi-csharp-fallible-fixture")
        .tempdir()
        .expect("create fixture tempdir");
    let root = root.path();
    let boltffi_path = workspace_root().join("boltffi");

    fs::write(
        root.join("Cargo.toml"),
        format!(
            "[package]\nname = \"csharp_fallible_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [lib]\npath = \"src/lib.rs\"\ncrate-type = [\"cdylib\", \"rlib\"]\n\n\
             [dependencies]\nboltffi = {{ path = \"{}\" }}\n",
            boltffi_path.display()
        ),
    )
    .expect("write fixture Cargo.toml");

    fs::write(
        root.join("boltffi.toml"),
        "[package]\nname = \"CSharpFallibleFixture\"\n\n[targets.csharp]\noutput = \"dist/csharp\"\nenabled = true\nnamespace = \"Fixture.Ffi\"\n",
    )
    .expect("write fixture boltffi.toml");

    let source_dir = root.join("src");
    fs::create_dir_all(&source_dir).expect("create fixture src dir");
    fs::write(
        source_dir.join("lib.rs"),
        r#"
use boltffi::{error, export};

// A record/struct error (not a bare `String`) is what makes this reproduce
// reliably -- see this file's module doc.
#[error]
#[derive(Debug)]
pub struct WidgetError {
    pub code: i32,
    pub message: String,
}

pub struct Widget {
    class_name: String,
}

#[export]
impl Widget {
    pub fn new(client_id: String, class_name: String) -> Result<Self, WidgetError> {
        if client_id == "bad" {
            Err(WidgetError {
                code: 1,
                message: format!("no such client: {client_id}"),
            })
        } else {
            Ok(Self { class_name })
        }
    }

    pub fn class_name(&self) -> String {
        self.class_name.clone()
    }

    // A non-constructor fallible method: legacy compiles this as a single
    // tagged buffer (success and error share one return-slot encoding),
    // structurally different from the constructor's raw-pointer-return ABI
    // above -- covering both legacy fallible shapes this fix touches.
    pub fn rename(&self, new_name: String) -> Result<String, WidgetError> {
        if new_name.is_empty() {
            Err(WidgetError {
                code: 2,
                message: "class name must not be empty".to_string(),
            })
        } else {
            Ok(format!("{}->{new_name}", self.class_name))
        }
    }
}

// Closure/callback param+return needed to trigger BufFromBytes emission --
// unrelated to the ABI bug this test targets.
#[export]
pub fn apply_label(f: impl Fn(String) -> String, input: String) -> String {
    f(input)
}
"#,
    )
    .expect("write fixture lib.rs");

    let pack_status = Command::new(env!("CARGO_BIN_EXE_boltffi"))
        .current_dir(root)
        .arg("pack")
        .arg("csharp")
        .status()
        .expect("boltffi pack csharp should spawn");
    assert!(pack_status.success(), "boltffi pack csharp should succeed");

    let package_source = root.join("dist/csharp/packages");
    assert!(
        package_source.is_dir(),
        "expected a packages directory at {}",
        package_source.display()
    );

    // NuGet's global package cache is keyed by id+version, not content — a
    // fixed-name/fixed-version fixture package from an earlier run of this
    // test would otherwise silently win the restore over this run's freshly
    // packed one (the same staleness trap `build-csharp.sh` guards against
    // for the real `parse_core` package).
    if let Some(home) = std::env::var_os("HOME") {
        let _ = fs::remove_dir_all(PathBuf::from(home).join(".nuget/packages/csharpfalliblefixture"));
    }

    let consumer_dir = root.join("consumer");
    fs::create_dir_all(&consumer_dir).expect("create consumer dir");
    fs::write(
        consumer_dir.join("nuget.config"),
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<configuration>\n  <packageSources>\n    <clear />\n    <add key=\"local-fixture\" value=\"{}\" />\n  </packageSources>\n</configuration>\n",
            package_source.display()
        ),
    )
    .expect("write consumer nuget.config");
    fs::write(
        consumer_dir.join("Consumer.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    <TargetFramework>net10.0</TargetFramework>\n    <Nullable>enable</Nullable>\n    <ImplicitUsings>enable</ImplicitUsings>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"CSharpFallibleFixture\" Version=\"0.0.0\" />\n  </ItemGroup>\n</Project>\n",
    )
    .expect("write consumer csproj");
    fs::write(
        consumer_dir.join("Program.cs"),
        r#"using Fixture.Ffi;

using var ok = new Widget("good-client", "MyClass");
try
{
    Console.WriteLine($"CTOR_OK:{ok.ClassName()}");
}
catch (Exception ex)
{
    Console.WriteLine($"CTOR_OK_UNEXPECTED_ERROR:{ex.GetType().Name}:{ex.Message}");
}

try
{
    var _ = new Widget("bad", "MyClass");
    Console.WriteLine("CTOR_ERR_UNEXPECTED_SUCCESS");
}
catch (Exception ex)
{
    Console.WriteLine($"CTOR_ERR:{ex.GetType().Name}:{ex.Message}");
}

try
{
    Console.WriteLine($"METHOD_OK:{ok.Rename("Renamed")}");
}
catch (Exception ex)
{
    Console.WriteLine($"METHOD_OK_UNEXPECTED_ERROR:{ex.GetType().Name}:{ex.Message}");
}

try
{
    ok.Rename("");
    Console.WriteLine("METHOD_ERR_UNEXPECTED_SUCCESS");
}
catch (Exception ex)
{
    Console.WriteLine($"METHOD_ERR:{ex.GetType().Name}:{ex.Message}");
}
"#,
    )
    .expect("write consumer Program.cs");

    let run = Command::new("dotnet")
        .current_dir(&consumer_dir)
        .arg("run")
        .output()
        .expect("dotnet run should spawn");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        run.status.success(),
        "dotnet run should succeed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    assert!(
        stdout.contains("CTOR_OK:MyClass"),
        "fallible constructor's SUCCESS branch should decode the real handle/value, not throw \
         OverflowException/corrupt-wire reading an unwritten out-param as an error buffer:\n{stdout}"
    );
    assert!(
        stdout.contains("CTOR_ERR:WidgetErrorException:no such client: bad"),
        "fallible constructor's ERROR branch should decode the real WidgetError record:\n{stdout}"
    );
    assert!(
        stdout.contains("METHOD_OK:MyClass->Renamed"),
        "fallible non-constructor method's SUCCESS branch should decode the real value \
         (the legacy ABI's single-tagged-buffer shape, distinct from the constructor's \
         raw-pointer-return shape):\n{stdout}"
    );
    assert!(
        stdout.contains("METHOD_ERR:WidgetErrorException:class name must not be empty"),
        "fallible non-constructor method's ERROR branch should decode the real WidgetError \
         record:\n{stdout}"
    );
    assert!(
        !stdout.contains("UNEXPECTED"),
        "no branch should silently take the wrong outcome:\n{stdout}"
    );
}
