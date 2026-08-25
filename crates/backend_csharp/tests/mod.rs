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
/// A killed or panicking test leaves its lock behind. Without reclamation that wedges every
/// later run, which is worse than the collision the lock exists to prevent.
const PLUGIN_LOCK_TIMEOUT: Duration = Duration::from_secs(300);

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

/// Builds the `$base/$name` plugin project and stages its DLL into `$base/_plugins/`, at most
/// once per test process.
///
/// Every path that reaches a staged DLL must come through here - `define_plugin!`,
/// `load_plugin!`, and `dll_path_for`. Cargo runs a plugin's define and load tests on parallel
/// threads with no ordering guarantee, and on Windows a DLL the .NET runtime has already mapped
/// cannot be overwritten; the copy fails with `os error 32`. Whichever caller arrives first
/// therefore performs the build, and the rest find it recorded here and never touch the file.
///
/// Consequence worth knowing: if a loader wins the race, the DLL is built from the interop
/// sources as they were on disk, before `define_plugin!` regenerated them. That only differs
/// when the inventory has changed, and then `define_plugin!`'s snapshot assertion says so.
fn ensure_plugin_built(base: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    let staged = base.join("_plugins").join(name);

    // Recover from a poisoned lock rather than cascading one panicking test into every later one.
    let mut built = BUILT_PLUGINS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if built.contains(&staged) {
        return Ok(());
    }

    // `name` is the DLL file name; the project directory and its csproj share the stem.
    let project_dir = base.join(name);
    let stem = Path::new(name).file_stem().expect("plugin name must carry a .dll suffix");
    let csproj = project_dir.join(stem).with_extension("csproj");

    let staged_dir = staged.parent().expect("staged path has a parent");
    std::fs::create_dir_all(staged_dir)?;
    let _lock = PluginBuildLock::acquire(staged_dir.join(format!(".lock-{name}")))?;

    let status = std::process::Command::new("dotnet").args(["build", "-c", "Release", "-v", "q"]).arg(&csproj).status()?;
    assert!(status.success(), "dotnet build failed for {}", csproj.display());

    let built_dll = project_dir.join("bin").join("Release").join("net11.0").join(name);
    if !stage_is_current(&staged, &built_dll) {
        std::fs::copy(&built_dll, &staged)?;
    }

    built.insert(staged);
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
        use interoptopus_csharp::dispatch::Dispatch;

        let multibuf = ::interoptopus_csharp::DotnetLibrary::builder(<$plugin as ::interoptopus::lang::plugin::PluginInfo>::inventory())
            .dispatch(Dispatch::plugin_defaults_with("My.Company"))
            .exception(crate::FILE_NOT_FOUND_EXCEPTION)
            .build()
            .process()?;

        let base = ::std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join($base);
        multibuf.write_buffers_to(base.join($name))?;
        crate::ensure_plugin_built(&base, $name)?;

        insta::assert_snapshot!(multibuf);
    }};
}

/// Ensures the `$name.dll` plugin is built and staged in `$base/_plugins/`, then loads it and
/// returns an instance of `$plugin`.
#[macro_export]
macro_rules! load_plugin {
    ($plugin:ty, $name:expr, $base:expr) => {{
        let path = crate::dll_path_for($base, $name);
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
fn dll_path_for(base: impl AsRef<Path>, name: impl AsRef<Path>) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(base);
    let name = name.as_ref().to_str().expect("plugin name must be valid UTF-8");
    ensure_plugin_built(&base, name).expect("failed to build plugin");
    base.join("_plugins").join(name)
}
