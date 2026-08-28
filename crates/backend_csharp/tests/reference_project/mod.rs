use std::path::PathBuf;

/// Generates the reference project's C# bindings and snapshot-tests the result.
///
/// Generation itself lives in [`crate::prepare_reference_bindings`] so that [`csharp_suite`] can
/// guarantee the sources exist without depending on this test having run first.
#[test]
fn interop() -> Result<(), Box<dyn std::error::Error>> {
    let multibuf = crate::prepare_reference_bindings()?;

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
/// `dotnet run` rather than `dotnet test`: `Tests.csproj` hosts on Microsoft.Testing.Platform via
/// `xunit.v3.aot`, which rejects VSTest-era arguments. Measured 2026-08-28 - `dotnet test` builds
/// the project, then the host prints its own help and exits 5 having run nothing.
///
/// Fails rather than skips when `dotnet` is absent. A guard that quietly stops running is the
/// failure this test exists to end, not one to reproduce.
#[test]
fn csharp_suite() -> Result<(), Box<dyn std::error::Error>> {
    // Generate first. The bindings are gitignored, so a fresh worktree has none, and test order
    // across threads and nextest processes is not something this test may assume.
    crate::prepare_reference_bindings()?;
    let staged = crate::stage_reference_cdylib()?;

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_dir = manifest.join("tests").join("reference_project").join("Tests");
    let csproj = project_dir.join("Tests.csproj");

    // `--project` explicitly. Deleting `InteropSpike.csproj` in a6d81c98 left one project here, so
    // directory inference happens to work today and would break silently on the next one added.
    let mut command = std::process::Command::new("dotnet");
    command.args(["run", "--project"]).arg(&csproj).args(["-c", "Debug"]);

    let status = crate::run_with_timeout(&mut command, crate::DOTNET_TIMEOUT)?;
    assert!(status.success(), "C# suite failed for {} with {status}", csproj.display());

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
        "{} is missing: the Content glob in Bindings.csproj matched nothing, so the suite ran without \
         the native library and every P/Invoke would throw DllNotFoundException",
        copied.display()
    );
    assert!(crate::is_no_older_than(&copied, &staged)?, "{} predates {}, so the suite loaded a stale native library", copied.display(), staged.display());
    assert!(
        crate::is_no_older_than(&bindings_assembly, &interop_cs)?,
        "{} predates {}, so the suite ran against bindings compiled from older sources",
        bindings_assembly.display(),
        interop_cs.display()
    );

    Ok(())
}
