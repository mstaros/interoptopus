use interoptopus_backends::template::pack_assets;
use std::fs::File;
use std::path::{Path, PathBuf};

fn main() {
    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("templates.tar");
    let out_file = File::create(&out_path).unwrap();

    pack_assets(out_file, "templates/").unwrap();

    watch(Path::new("templates"));
}

/// Emits `cargo:rerun-if-changed` for every template file, not just the root directory.
///
/// `cargo:rerun-if-changed` on a directory **does not recurse** — Cargo stats the path it is
/// given. Every template lives at least three levels down (`templates/common/types/enums/...`),
/// so a single line for `templates/` meant editing a template in place never invalidated this
/// script. `templates.tar` stayed stale, the binary went on rendering the previously embedded
/// copy, and a template-only change produced no diff in generated output at all.
///
/// That is what let `9e3383f` pass a gate it never executed. The impact analyser not selecting
/// tests for a `.cs` change was the shallower half; even had it selected them, they would have
/// compared stale output against stale snapshots and matched.
///
/// Directories are emitted too, so that adding or deleting a template is caught as well as
/// editing one.
fn watch(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());

    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));

    for entry in entries {
        let path = entry.expect("directory entry is readable").path();

        if path.is_dir() {
            watch(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
