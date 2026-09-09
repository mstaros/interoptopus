use interoptopus::lang::meta::FileEmission;
use interoptopus_csharp::RustLibrary;
use interoptopus_csharp::config::{DllImportSearchPath, HeaderConfig, SearchPathConfig};
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;
use std::path::PathBuf;

/// Compile and execute the ValueTask configuration, including its generic and void surfaces.
#[test]
fn csharp_value_tasks() -> Result<(), Box<dyn std::error::Error>> {
    let multibuf = RustLibrary::builder(::reference_project::inventory())
        .value_tasks()
        .dll_name("reference_project")
        .dispatch(Dispatch::custom(|x, _| match x.emission {
            FileEmission::Common => Target::new("Interop.Common.cs", "My.Company.Common"),
            _ => Target::new("Interop.cs", "My.Company"),
        }))
        .headers(HeaderConfig { emit_version: false })
        .search_path(SearchPathConfig { import_search_path: DllImportSearchPath::None })
        .build()
        .process()?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_dir = manifest.join("tests/reference_project/ValueTasks");
    multibuf.write_buffers_to_if_changed(&project_dir)?;
    let staged = crate::stage_reference_cdylib()?;
    let corerun = super::patched_corerun(&manifest)?;

    let mut build = std::process::Command::new("dotnet");
    build.arg("build").arg(project_dir.join("ValueTasks.csproj")).args(["-c", "Debug"]).current_dir(&project_dir);
    let status = crate::run_with_timeout(&mut build, crate::DOTNET_TIMEOUT)?;
    assert!(status.success(), "ValueTask bindings must compile: {status}");

    let output = project_dir.join("bin/Debug/net11.0");
    let assembly = output.join("ValueTasks.dll");
    assert!(crate::is_no_older_than(&assembly, &project_dir.join("Interop.cs"))?);
    assert!(crate::is_no_older_than(&assembly, &project_dir.join("Interop.Common.cs"))?);
    let copied = output.join(crate::REFERENCE_CDYLIB);
    std::fs::copy(&staged, &copied)?;
    assert!(crate::is_no_older_than(&copied, &staged)?);

    let mut run = std::process::Command::new(&corerun);
    run.args(["-p", &format!("RUNTIME_IDENTIFIER={}", super::runtime_identifier()?)])
        .arg(&assembly)
        .current_dir(&output)
        .env("CORE_LIBRARIES", &output);
    let status = crate::run_with_timeout(&mut run, crate::DOTNET_TIMEOUT)?;
    assert!(status.success(), "ValueTask suite failed under {}: {status}", corerun.display());
    Ok(())
}
