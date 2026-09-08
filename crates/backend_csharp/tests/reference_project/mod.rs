use std::io::Write;
use std::path::{Path, PathBuf};

/// Uses the same override as the configured C# test tooling. Without an override, the Windows
/// x64 runtime checkout sits beside this repository, or beside its Guarded worktrees directory.
/// An invalid explicit override is an error; it never selects a different or stock runtime.
fn patched_corerun(manifest: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    const CONFIGURATION: &str = "CSHARPMPC_PATCHED_CORERUN";
    let corerun = if let Some(configured) = std::env::var_os(CONFIGURATION) {
        let configured = PathBuf::from(configured);
        if !configured.is_absolute() {
            return Err(format!("{CONFIGURATION} must name an absolute patched corerun path: {}", configured.display()).into());
        }
        configured
    } else {
        let repository = manifest.parent().and_then(Path::parent).expect("backend_csharp is under crates");
        let candidates = repository
            .parent()
            .into_iter()
            .chain(repository.parent().and_then(Path::parent))
            .map(|parent| {
                parent.join("runtime-async-dynamicmethod").join(
                    "artifacts/tests/coreclr/windows.x64.Release/Tests/Core_Root/corerun.exe",
                )
            })
            .collect::<Vec<_>>();
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return Err(format!("set {CONFIGURATION} to the patched corerun for this platform; the repository-relative default is Windows x64").into());
        }
        candidates.iter().find(|candidate| candidate.is_file()).cloned().ok_or_else(|| {
            format!(
                "patched corerun is required: set {CONFIGURATION}, or build the existing runtime checkout at one of {candidates:?}"
            )
        })?
    };

    let executable = if cfg!(windows) { "corerun.exe" } else { "corerun" };
    if !corerun.is_file() || corerun.file_name().and_then(|name| name.to_str()).is_none_or(|name| !name.eq_ignore_ascii_case(executable)) {
        return Err(format!("patched corerun is required but {} is not an existing {executable}; configure {CONFIGURATION}", corerun.display()).into());
    }
    let core_root = corerun.parent().expect("an absolute corerun path has a parent");
    let coreclr = if cfg!(windows) { "coreclr.dll" } else if cfg!(target_os = "macos") { "libcoreclr.dylib" } else { "libcoreclr.so" };
    for component in [coreclr, "System.Private.CoreLib.dll"] {
        if !core_root.join(component).is_file() {
            return Err(format!("patched corerun at {} has no {component} beside it", corerun.display()).into());
        }
    }
    Ok(corerun)
}

fn runtime_identifier() -> Result<String, Box<dyn std::error::Error>> {
    let platform = match std::env::consts::OS {
        "windows" => "win",
        "linux" => "linux",
        "macos" => "osx",
        other => return Err(format!("patched corerun test hosting is not configured for {other}").into()),
    };
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        "arm" => "arm",
        other => return Err(format!("patched corerun test hosting is not configured for architecture {other}").into()),
    };
    Ok(format!("{platform}-{architecture}"))
}

/// Generates the reference project's C# bindings and snapshot-tests the result.
///
/// Generation itself lives in [`crate::prepare_reference_bindings`] so that [`csharp_suite`] can
/// guarantee the sources exist without depending on this test having run first.
#[test]
fn interop() -> Result<(), Box<dyn std::error::Error>> {
    let multibuf = crate::prepare_reference_bindings()?;
    let interop = multibuf.buffer("Interop.cs").expect("reference bindings must contain Interop.cs").to_string();
    let common = multibuf
        .buffer("Interop.Common.cs")
        .expect("reference bindings must contain Interop.Common.cs")
        .to_string();

    assert!(interop.contains(
        "public static partial ResultVoidError pattern_string_6a([MarshalUsing(typeof(UseString.InMarshallerMeta))] in UseString _0);"
    ));
    assert!(interop.contains("public static partial ResultVoidError pattern_string_6b(ref UseString y);"));
    assert!(interop.contains("public static partial IntPtr ref1(ref long x);"));
    assert!(interop.contains("[MarshalUsing(typeof(ResultUintError.InMarshallerMeta))] in ResultUintError _0"));
    assert!(interop.contains("[MarshalUsing(typeof(OptionUtf8String.InMarshallerMeta))] in OptionUtf8String _0"));
    assert!(interop.contains("[CustomMarshaller(typeof(UseString), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]"));
    assert!(interop.contains("public struct InMarshallerMeta { }"));
    assert!(interop.contains("public Unmanaged ToUnmanaged() { return _managed.AsUnmanaged(); }"));
    assert!(interop.contains("public static unsafe uint pattern_string_13("));
    assert!(interop.contains("return pattern_string_13(in _0, callback_wrapped);"));
    assert!(interop.contains("public static partial ulong __test_live_bytes();"));
    assert!(interop.contains("public static partial ulong __test_live_allocations();"));

    let slice_from = common
        .split_once("public static unsafe SliceOptionUtf8String From(OptionUtf8String[] managed)")
        .and_then(|(_, rest)| rest.split_once("/// Frees the native copy.").map(|(method, _)| method))
        .expect("generated SliceOptionUtf8String.From method");
    let mut cursor = 0;
    for fragment in [
        "if (managed[i] is null)",
        "rval._data = Marshal.AllocHGlobal(checked(size * managed.Length));",
        "\n        try\n        {",
        "var unmanaged = managed[i].AsUnmanaged();",
        "\n        catch\n        {",
        "rval.Dispose();",
        "throw;",
        "return rval;",
    ] {
        let relative = slice_from[cursor..]
            .find(fragment)
            .unwrap_or_else(|| panic!("SliceOptionUtf8String.From must contain ordered fragment `{fragment}`"));
        cursor += relative + fragment.len();
    }

    for source in [&interop, &common] {
        for (offset, _) in source.match_indices("[LibraryImport") {
            let declaration = &source[offset..];
            let attributes = declaration.split_once("static partial").expect("generated native import").0;
            assert!(attributes.contains("UnmanagedCallConv(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })"));
        }
    }

    insta::assert_snapshot!(multibuf);

    Ok(())
}

/// Builds and runs the generated bindings' C# test suite, failing if the build or any test fails.
///
/// This is the only thing in the repo that compiles the generated C# from `cargo test`. The
/// snapshot test above asserts *text*, and text can be well formed and still not compile: that is
/// how `emit_enum_serialize` shipped `IsA`/`IsB`/`IsC` accessors on types projected as plain C#
/// enums past 78 green tests, caught only when someone compiled the bindings by hand.
///
/// `Tests.csproj` hosts xUnit v3 on Microsoft.Testing.Platform. Build it with the .NET 11
/// preview SDK, then launch its managed executable with the required patched corerun. Passing
/// VSTest-era arguments to `dotnet test` only prints help and exits 5 without running tests
/// (measured 2026-08-28).
///
/// Fails rather than skips when the SDK or patched runtime is absent. The selected corerun
/// and runtime identifier are reported, so a successful run also identifies its actual host.
///
/// **Editing only `Tests/*.cs` will not run this test.** Guarded's impact analyser does not model
/// a `.cs` file as an input to a Rust test, so a commit touching only C# sources validates with
/// `step_count: 1`, selects nothing, and passes a gate it never executed. Measured 2026-08-28 on
/// `c78c689`, which added a test to `Test.Core.Enums.cs` and never compiled it. A `.cs` *template*
/// bypasses the generator's tests the same way (`9e3383f`, reverted in `a4e39ad`). Touch a Rust
/// file in the same commit, and check the summary reports two steps rather than one. That is why unrelated-looking Rust edits accompany C# test commits here; `84eff37e` (item 5e) is one, as are the commits adding items 5h and 5i the `Utf8String` ownership test, and the Open items 1 null-at-marshal-out measurement.
#[test]
fn csharp_suite() -> Result<(), Box<dyn std::error::Error>> {
    // Generate first. The bindings are gitignored, so a fresh worktree has none, and test order
    // across threads and nextest processes is not something this test may assume.
    crate::prepare_reference_bindings()?;
    let staged = crate::stage_reference_cdylib()?;

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_dir = manifest.join("tests").join("reference_project").join("Tests");
    let csproj = project_dir.join("Tests.csproj");

    let corerun = patched_corerun(&manifest)?;
    let runtime_identifier = runtime_identifier()?;

    // Name the project explicitly; this build never selects the managed test runtime.
    let mut build = std::process::Command::new("dotnet");
    build.arg("build").arg(&csproj).args(["-c", "Debug"]).current_dir(&project_dir);
    let status = crate::run_with_timeout(&mut build, crate::DOTNET_TIMEOUT)?;
    assert!(status.success(), "C# suite build failed for {} with {status}", csproj.display());

    // A finished build is not evidence that anything was built. The tool-level build cache served
    // stale outputs across four consecutive builds on 2026-08-27 and produced three separate wrong
    // diagnoses; a green run over a stale assembly would recreate precisely the blind spot this
    // test closes.
    //
    // Be exact about what these three have and have not shown. The *condition* the first one
    // catches was observed directly: before the staging above existed, the output directory held
    // no native library, because the Content glob in `Bindings.csproj` matched nothing. The
    // assertion itself has never fired - it did not exist then. All three are unfired guards, and
    // an unfired guard is worth less than one that has been watched failing. Do not write that any
    // of them caught something until it has.
    let output_dir = project_dir.join("bin").join("Debug").join("net11.0");
    let interop_cs = manifest.join("tests").join("reference_project").join("Bindings").join("Interop.cs");
    let copied = output_dir.join(crate::REFERENCE_CDYLIB);
    let bindings_assembly = output_dir.join("Bindings.dll");

    assert!(
        copied.is_file(),
        "{} is missing: the Content glob in Bindings.csproj matched nothing, so the suite would run without \
         the native library and every P/Invoke would throw DllNotFoundException",
        copied.display()
    );
    assert!(crate::is_no_older_than(&copied, &staged)?, "{} predates {}, so the suite would load a stale native library", copied.display(), staged.display());
    assert!(
        crate::is_no_older_than(&bindings_assembly, &interop_cs)?,
        "{} predates {}, so the suite would run against bindings compiled from older sources",
        bindings_assembly.display(),
        interop_cs.display()
    );

    let test_assembly = output_dir.join("Tests.dll");
    assert!(test_assembly.is_file(), "{} is missing after dotnet build", test_assembly.display());

    // Raw corerun does not read deps.json. The output directory contains the managed package
    // closure and staged cdylib. Put that closure on the TPA list and prepend its native path
    // without changing the parent process environment or falling back to the stock host.
    let native_search_variable = match std::env::consts::OS {
        "windows" => "PATH",
        "macos" => "DYLD_LIBRARY_PATH",
        _ => "LD_LIBRARY_PATH",
    };
    let mut native_directories = vec![output_dir.clone()];
    if let Some(existing) = std::env::var_os(native_search_variable) {
        native_directories.extend(std::env::split_paths(&existing));
    }
    let mut run = std::process::Command::new(&corerun);
    run.args(["-p", &format!("RUNTIME_IDENTIFIER={runtime_identifier}")])
        .arg(&test_assembly)
        .current_dir(&project_dir)
        .env("CORE_LIBRARIES", &output_dir)
        .env(native_search_variable, std::env::join_paths(native_directories)?);
    // Direct stderr writes remain visible under libtest's Rust-print capture, like the child
    // process output, so the Cargo operation records the host used by this validation.
    writeln!(
        std::io::stderr(),
        "PatchedCoreRun: {} ({runtime_identifier}) -> {}",
        corerun.display(),
        test_assembly.display()
    )?;
    let status = crate::run_with_timeout(&mut run, crate::DOTNET_TIMEOUT)?;
    assert!(status.success(), "C# suite failed under patched corerun {} with {status}", corerun.display());

    Ok(())
}
