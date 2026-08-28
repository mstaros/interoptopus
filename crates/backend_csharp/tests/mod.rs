// #![allow(unused)]

use interoptopus_csharp::pattern::Exception;
use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

#[macro_use]
mod common;
mod backend_plugins;
mod output;
mod reference_plugins;
mod reference_project;

pub const FILE_NOT_FOUND_EXCEPTION: Exception = Exception::new("System.IO.FileNotFoundException");

mod model {
    mod service_rval_result;
}

/// Plugins already built and staged in this test process, keyed by staged DLL path.
static BUILT_PLUGINS: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// How long a lock file may exist before a later process treats it as abandoned.
///
/// A killed test leaves its lock behind. A panicking one does not — unwinding runs `Drop` —
/// but `cargo-nextest` cancels outstanding tests on first failure, and a killed process runs
/// nothing. Every other waiter then blocks until this timeout expires.
///
/// Measured at 300s that turned a fast failure into a 301-second stall on two tests. Observed
/// plugin builds take 6-15s, so 60s is roughly 4x the slowest real build while bounding an
/// orphan stall to a minute. Raising it trades stall time for the risk of reclaiming a lock
/// whose holder is merely slow, which would reintroduce the concurrent build this prevents.
const PLUGIN_LOCK_TIMEOUT: Duration = Duration::from_secs(60);

const PLUGIN_LOCK_POLL: Duration = Duration::from_millis(100);

/// Cross-process exclusion for one plugin's build-and-stage, released on drop.
///
/// Per plugin rather than global, so unrelated plugins still build in parallel.
struct PluginBuildLock {
    path: PathBuf,
}

impl PluginBuildLock {
    fn acquire(path: PathBuf) -> Result<Self, Box<dyn Error>> {
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    // Recorded for diagnosis only. Reclamation is by age: a pid can be reused,
                    // and checking liveness portably is more machinery than this warrants.
                    use std::io::Write;
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_abandoned(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    sleep(PLUGIN_LOCK_POLL);
                }
                Err(e) => return Err(Box::new(e)),
            }
        }
    }
}

impl Drop for PluginBuildLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A lock older than [`PLUGIN_LOCK_TIMEOUT`]. A missing or unreadable file is not abandoned:
/// the holder may have just released it, and racing to delete nothing helps no one.
fn lock_is_abandoned(path: &Path) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else { return false };
    SystemTime::now().duration_since(modified).is_ok_and(|age| age > PLUGIN_LOCK_TIMEOUT)
}

/// True when the staged DLL is already at least as new as the one just built.
///
/// The copy is the second cross-process hazard: on Windows a DLL another process has mapped
/// cannot be overwritten (`os error 32`). Skipping the write when the staged file is current
/// means the common path does not touch it at all, rather than writing and hoping nobody holds
/// it. Unlike the build collision this one has not been reproduced - it is predicted by the
/// code path and by this module's own comment above, and designed against on that basis.
fn stage_is_current(staged: &Path, built: &Path) -> bool {
    let (Ok(staged), Ok(built)) = (std::fs::metadata(staged), std::fs::metadata(built)) else {
        return false;
    };
    match (staged.modified(), built.modified()) {
        (Ok(staged), Ok(built)) => staged >= built,
        _ => false,
    }
}

/// Generates a plugin's interop sources, then builds and stages its DLL.
///
/// Generation lives here rather than in `define_plugin!` so arrival order stops mattering. It
/// used to be split: `define_plugin!` generated, this function built. A loader reaching a
/// plugin first therefore compiled against whatever was on disk — and on a fresh checkout the
/// interop files are gitignored, so nothing was, and the build failed with `CS0246` rather than
/// producing a stale DLL. Measured: a fresh worktree fails three of sixty-two on the first run
/// and passes on the second.
///
/// **Writes are conditional on content**, which is load-bearing rather than an optimisation.
/// `write_buffers_to` writes unconditionally for `Overwrite::Always` buffers, which the interop
/// files use. Rewriting identical bytes still bumps mtime, which would make `dotnet` rebuild,
/// which would make the built DLL newer than the staged one, which would fire the copy that
/// [`stage_is_current`] exists to avoid — reintroducing the mapped-DLL collision (`os error
/// 32`). The in-process set cannot prevent that: nextest gives each test its own process.
///
/// The generated buffer is returned rather than consumed so `define_plugin!` can snapshot it.
/// The snapshot assertion stays in that macro deliberately — moving it here would make every
/// loader assert a snapshot it did not ask for.
fn prepare_plugin<P: interoptopus::lang::plugin::PluginInfo>(base: &Path, name: &str) -> Result<impl std::fmt::Display, Box<dyn Error>> {
    use interoptopus_csharp::dispatch::Dispatch;

    // Single-sourced on purpose. If the definer and the loader each built their own generation
    // config, the files on disk would depend on which won the race.
    let multibuf = interoptopus_csharp::DotnetLibrary::builder(P::inventory())
        .dispatch(Dispatch::plugin_defaults_with("My.Company"))
        .exception(FILE_NOT_FOUND_EXCEPTION)
        .build()
        .process()?;

    let project_dir = base.join(name);
    let staged = base.join("_plugins").join(name);
    let staged_dir = staged.parent().expect("staged path has a parent");
    std::fs::create_dir_all(staged_dir)?;

    // Held across generation as well as build: two processes writing the same source file
    // interleaved would be as bad as two compiling it.
    let _lock = PluginBuildLock::acquire(staged_dir.join(format!(".lock-{name}")))?;

    let mut built = BUILT_PLUGINS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !built.contains(&staged) {
        std::fs::create_dir_all(&project_dir)?;
        // Not `write_buffers_to`: that rewrites unconditionally. The content check has to live
        // in `Multibuf` rather than here because the per-buffer `Overwrite` policy is private,
        // so a loop over `iter()` would silently clobber `Overwrite::Never` files.
        multibuf.write_buffers_to_if_changed(&project_dir)?;
        build_and_stage(&project_dir, &staged, name)?;
        built.insert(staged.clone());
    }

    Ok(multibuf)
}

/// Builds the plugin project and stages its DLL.
///
/// Assumes the caller holds the per-plugin lock and has already written the sources; this
/// function does neither. Split out of the old `ensure_plugin_built` so that generation,
/// locking and the once-per-process check live together in [`prepare_plugin`] and this is only
/// the part that shells out.
fn build_and_stage(project_dir: &Path, staged: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    // `name` is the DLL file name; the project directory and its csproj share the stem.
    let stem = Path::new(name).file_stem().expect("plugin name must carry a .dll suffix");
    let csproj = project_dir.join(stem).with_extension("csproj");

    let status = std::process::Command::new("dotnet").args(["build", "-c", "Release", "-v", "q"]).arg(&csproj).status()?;
    if !status.success() {
        // NuGet resolves its package root and user-level config from the ambient environment.
        // A stripped environment fails restore with `Value cannot be null (Parameter 'path1')`
        // at NuGet.targets, which names neither the variable nor the path it could not build.
        // Report them here so the next reader does not have to guess which one is missing.
        for var in ["USERPROFILE", "APPDATA", "NUGET_PACKAGES", "HOME", "DOTNET_CLI_HOME"] {
            eprintln!("  {var} = {:?}", std::env::var(var).ok());
        }
        panic!("dotnet build failed for {}", csproj.display());
    }

    let built_dll = project_dir.join("bin").join("Release").join("net11.0").join(name);
    if !stage_is_current(staged, &built_dll) {
        std::fs::copy(&built_dll, staged)?;
    }
    Ok(())
}

/// Generates interop files for `$plugin` into the `$base/$name` folder, ensures the plugin is
/// built and staged, and snapshot-tests the generated output.
///
/// Building here rather than in a separate `just build-dotnet-plugins` step keeps the DLL in
/// lockstep with the interop sources, which is what `ApiMismatch` used to catch late. Requires a
/// .NET 11 preview SDK on PATH.
#[macro_export]
macro_rules! define_plugin {
    ($plugin:ty, $name:expr, $base:expr) => {{
        let base = ::std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join($base);
        let multibuf = crate::prepare_plugin::<$plugin>(&base, $name)?;

        insta::assert_snapshot!(multibuf);
    }};
}

/// Ensures the `$name.dll` plugin is built and staged in `$base/_plugins/`, then loads it and
/// returns an instance of `$plugin`.
#[macro_export]
macro_rules! load_plugin {
    ($plugin:ty, $name:expr, $base:expr) => {{
        let path = crate::dll_path_for::<$plugin>($base, $name);
        let rt = ::interoptopus_csharp::rt::dynamic::runtime().expect("failed to initialize .NET runtime");
        rt.load::<$plugin>(path)?
    }};
}

/// Returns the path to the compiled plugin DLL under the given base directory, building and
/// staging it first if this process has not already done so.
///
/// Several tests load a plugin without going through `load_plugin!`, reaching the DLL through
/// this helper instead. It must therefore build too, or those tests map a stale DLL and leave it
/// locked against a later `define_plugin!` copy.
///
/// Panics rather than returning a `Result` so the signature stays usable from thread closures and
/// other non-`Result` contexts. A plugin that will not build is a test failure either way.
fn dll_path_for<P: interoptopus::lang::plugin::PluginInfo>(base: impl AsRef<Path>, name: impl AsRef<Path>) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(base);
    let name = name.as_ref().to_str().expect("plugin name must be valid UTF-8");
    prepare_plugin::<P>(&base, name).expect("failed to prepare plugin");
    base.join("_plugins").join(name)
}

/// The platform file name of the `reference_project` cdylib.
#[cfg(windows)]
const REFERENCE_CDYLIB: &str = "reference_project.dll";
#[cfg(target_os = "macos")]
const REFERENCE_CDYLIB: &str = "libreference_project.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const REFERENCE_CDYLIB: &str = "libreference_project.so";

/// How long `dotnet` may run before it is killed and the test fails.
///
/// `cargo test` applies no per-test timeout, so a hung restore would stall the whole suite with
/// no diagnostic and no obvious culprit. A cold build plus the full suite finishes well inside a
/// minute locally, so ten minutes trips only on a genuine hang.
const DOTNET_TIMEOUT: Duration = Duration::from_secs(600);

const DOTNET_POLL: Duration = Duration::from_millis(200);

/// The directory holding the running test binary, i.e. `<target>/<profile>/deps`.
///
/// Used as scratch space for the generation lock. It sits inside the target directory under every
/// `CARGO_TARGET_DIR` setting, so it is always writable and never tracked, and unlike
/// [`target_debug_dir`] it assumes nothing about where that directory sits relative to the repo -
/// which keeps the snapshot test free of a constraint only the C# build actually has.
fn test_binary_dir() -> Result<PathBuf, Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or("test binary has no parent directory")?;
    Ok(dir.to_path_buf())
}

/// The directory `Bindings.csproj` looks in for the native library.
///
/// That project reaches the native library through a `Content` glob five levels up from its own
/// directory, which resolves to `<repo>/target/debug` and nothing else. That path is fixed no
/// matter where cargo is writing, so staging copies *into* it rather than requiring the two to
/// agree.
///
/// An earlier version asserted they matched and refused otherwise. It was wrong, and instructively
/// so: it fired on 2026-08-28 under this repository's own transaction validation, which runs cargo
/// with a private `CARGO_TARGET_DIR`, and would have failed every future transaction touching this
/// crate. The two directories never needed to agree - the library only needs to be where the glob
/// looks. A guard can be accurate about the facts and still enforce the wrong requirement.
fn csproj_native_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join("debug")
}

/// Copies the built cdylib up from `deps/` to where `Bindings.csproj` looks for it.
///
/// Cargo uplifts a workspace member's artifacts to `target/debug` only when that member is
/// selected on the command line. Built instead as a dev-dependency of this crate,
/// `reference_project` leaves an unhashed copy in `target/debug/deps` with no hardlink up, so the
/// csproj glob matches nothing and the library never reaches the test output directory. Confirmed
/// on this checkout 2026-08-28: `deps` held it, `target/debug` did not.
///
/// A copy is the whole of the fix, deliberately not a `cargo build`. Invoking cargo from inside a
/// `cargo test` contends for the target-directory lock and blocks.
///
/// Features do not enter into this. `reference_project` declares none of its own and already
/// requests the complete `interoptopus` feature set, so `--all-features` cannot change a byte of
/// this library; the `deps` copy and an uplifted one are the same build.
fn stage_reference_cdylib() -> Result<PathBuf, Box<dyn Error>> {
    // Read from the directory holding this test binary - cargo's real `deps`, wherever that is -
    // and write to the fixed path the csproj globs. The two need not be related, which is what
    // makes this work under a custom `CARGO_TARGET_DIR`.
    let built = test_binary_dir()?.join(REFERENCE_CDYLIB);
    let native_dir = csproj_native_dir();
    std::fs::create_dir_all(&native_dir)?;
    let staged = native_dir.join(REFERENCE_CDYLIB);

    if !built.exists() {
        return Err(format!(
            "{} is missing. `reference_project` is a dev-dependency of this crate, so cargo builds its \
             cdylib before this test runs; absence means the layout changed.",
            built.display()
        )
        .into());
    }

    // Same guard as the plugin staging: on Windows a mapped DLL cannot be overwritten.
    if !stage_is_current(&staged, &built) {
        std::fs::copy(&built, &staged)?;
    }

    Ok(staged)
}

/// True when `artifact` was last written no earlier than `source`.
fn is_no_older_than(artifact: &Path, source: &Path) -> Result<bool, Box<dyn Error>> {
    let artifact = std::fs::metadata(artifact)?.modified()?;
    let source = std::fs::metadata(source)?.modified()?;
    Ok(artifact >= source)
}

/// Runs `command` to completion with inherited stdio and a wall-clock timeout.
///
/// Inherited rather than captured, deliberately. `libtest` intercepts Rust-level printing, not a
/// child's file descriptors, so compiler diagnostics and test-host output reach the terminal as
/// they happen. Capturing with `output()` would hide exactly what a red test needs to be
/// actionable, leaving the reader an exit code and nothing else.
fn run_with_timeout(command: &mut std::process::Command, timeout: Duration) -> Result<std::process::ExitStatus, Box<dyn Error>> {
    let mut child = command.spawn().map_err(|e| -> Box<dyn Error> {
        if e.kind() == std::io::ErrorKind::NotFound {
            "`dotnet` is not on PATH. This test needs a .NET 11 preview SDK and fails rather than skips, \
             so a missing toolchain cannot quietly stop the C# suite from running."
                .into()
        } else {
            format!("failed to start `dotnet`: {e}").into()
        }
    })?;

    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("`dotnet` exceeded {}s and was killed", timeout.as_secs()).into());
        }
        sleep(DOTNET_POLL);
    }
}

/// Generates the reference-project bindings into the two directories that consume them.
///
/// Extracted from `reference_project::interop` so the C# suite test can guarantee the sources
/// exist without depending on test ordering. Both callers are `#[test]`s in this binary, and
/// `cargo-nextest` gives each its own process, so ordering is not merely unspecified - it is
/// unavailable. The generated files are gitignored, so on a fresh worktree whichever test arrives
/// first finds nothing on disk.
///
/// The lock and the content-conditional write are the two mechanisms [`prepare_plugin`] uses, for
/// the same two reasons: interleaved writes from two processes corrupt the file, and an
/// unconditional rewrite bumps mtime even when the bytes are identical. The second matters more
/// here, because `reference_project::csharp_suite` compares timestamps to prove the assembly was
/// built from the current sources, and churn would make that comparison vacuous.
fn prepare_reference_bindings() -> Result<impl std::fmt::Display, Box<dyn Error>> {
    use interoptopus::lang::meta::FileEmission;
    use interoptopus_csharp::RustLibrary;
    use interoptopus_csharp::config::{DllImportSearchPath, HeaderConfig, SearchPathConfig};
    use interoptopus_csharp::dispatch::Dispatch;
    use interoptopus_csharp::output::Target;

    let multibuf = RustLibrary::builder(::reference_project::inventory())
        .dll_name("reference_project")
        .dispatch(Dispatch::custom(|x, _| match x.emission {
            FileEmission::Common => Target::new("Interop.Common.cs", "My.Company.Common"),
            FileEmission::Default => Target::new("Interop.cs", "My.Company"),
            FileEmission::CustomModule(_) => Target::new("Interop.cs", "My.Company"),
        }))
        .headers(HeaderConfig { emit_version: false })
        .search_path(SearchPathConfig { import_search_path: DllImportSearchPath::None })
        .build()
        .process()?;

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bindings = manifest.join("tests").join("reference_project").join("Bindings");
    std::fs::create_dir_all(&bindings)?;

    // The lock lives in the target directory, not beside the sources: `Bindings` is a csproj folder
    // whose only ignored entries are the generated `Interop*.cs`, so a lock file there would surface
    // as untracked in any `git status` taken mid-run.
    let _lock = PluginBuildLock::acquire(test_binary_dir()?.join(".lock-reference-bindings"))?;

    multibuf.write_buffers_to_if_changed(&bindings)?;
    multibuf.write_buffers_to_if_changed(manifest.join("benches").join("dotnet"))?;

    Ok(multibuf)
}
