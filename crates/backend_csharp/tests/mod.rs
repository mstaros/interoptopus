// #![allow(unused)]

use interoptopus_csharp::pattern::Exception;
use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::thread::sleep;
use std::time::{Duration, SystemTime};

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
