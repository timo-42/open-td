//! Build tasks: `cargo xtask <task>`.
//!
//! - `modules`: build the base rule modules for wasm32 and copy them into
//!   `packs/base/modules/`
//! - `pack [--out DIR]`: `modules`, then zip `packs/base` as
//!   `DIR/base-<version>.zip` (default `dist/`)
//! - `web [--out DIR]`: `pack`, then build the client for the browser with
//!   the pack embedded and run `wasm-bindgen` (default `dist/web/`)

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const MODULES: [&str; 6] = [
    "waves",
    "lives",
    "income",
    "versus",
    "builders",
    "canyon_gen",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(cmd: &mut Command) -> Result<(), String> {
    eprintln!("$ {cmd:?}");
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed: {status}"))
    }
}

fn modules() -> Result<(), String> {
    let root = root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(&root).args([
        "build",
        "--target",
        "wasm32-unknown-unknown",
        "--profile",
        "wasm-module",
    ]);
    for m in MODULES {
        cmd.args(["-p", m]);
    }
    run(&mut cmd)?;
    let out = root.join("packs/base/modules");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for m in MODULES {
        let src = root.join(format!(
            "target/wasm32-unknown-unknown/wasm-module/{m}.wasm"
        ));
        std::fs::copy(&src, out.join(format!("{m}.wasm")))
            .map_err(|e| format!("{}: {e}", src.display()))?;
    }
    eprintln!("modules copied to {}", out.display());
    Ok(())
}

fn pack_version(dir: &Path) -> Result<String, String> {
    let info = std::fs::read_to_string(dir.join("info.toml")).map_err(|e| e.to_string())?;
    info.lines()
        .find_map(|l| {
            l.strip_prefix("version").map(|r| {
                r.trim()
                    .trim_start_matches('=')
                    .trim()
                    .trim_matches('"')
                    .to_string()
            })
        })
        .ok_or_else(|| "info.toml has no version".into())
}

fn files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.')
            || name.ends_with(".md") && p.parent() == Some(base) && name != "CREDITS.md"
        {
            continue;
        }
        if p.is_dir() {
            files(&p, base, out)?;
        } else {
            out.push(p.strip_prefix(base).unwrap().to_path_buf());
        }
    }
    Ok(())
}

/// Writes a zip with sorted entries and fixed timestamps, so the same
/// inputs give the same bytes.
fn zip_dir(dir: &Path, dest: &Path) -> Result<(), String> {
    let mut list = Vec::new();
    files(dir, dir, &mut list)?;
    let file = std::fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for rel in list {
        let name = rel.to_string_lossy().replace('\\', "/");
        let data = std::fs::read(dir.join(&rel)).map_err(|e| e.to_string())?;
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn pack(out: &Path) -> Result<(), String> {
    modules()?;
    let dir = root().join("packs/base");
    let version = pack_version(&dir)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let dest = out.join(format!("base-{version}.zip"));
    zip_dir(&dir, &dest)?;
    eprintln!("wrote {}", dest.display());
    Ok(())
}

/// The wasm-bindgen version in Cargo.lock; the CLI must match it exactly.
fn wasm_bindgen_version() -> Option<String> {
    let lock = std::fs::read_to_string(root().join("Cargo.lock")).ok()?;
    let mut lines = lock.lines();
    while let Some(l) = lines.next() {
        if l == "name = \"wasm-bindgen\"" {
            return lines
                .next()?
                .strip_prefix("version = \"")?
                .strip_suffix('"')
                .map(String::from);
        }
    }
    None
}

fn web(out: &Path) -> Result<(), String> {
    let dist = root().join("dist");
    pack(&dist)?;
    let version = pack_version(&root().join("packs/base"))?;
    let zip = dist.join(format!("base-{version}.zip"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    run(Command::new(cargo)
        .current_dir(root())
        .env("TD_EMBED_PACK", &zip)
        .args([
            "build",
            "-p",
            "td_client",
            "--profile",
            "web",
            "--target",
            "wasm32-unknown-unknown",
            "--features",
            "embed-pack",
        ]))?;
    let wasm = root().join("target/wasm32-unknown-unknown/web/open-td.wasm");
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let bindgen = Command::new("wasm-bindgen")
        .args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(out)
        .arg(&wasm)
        .status();
    match bindgen {
        Ok(s) if s.success() => {}
        _ => {
            let v = wasm_bindgen_version().unwrap_or_else(|| "<see Cargo.lock>".into());
            return Err(format!(
                "wasm-bindgen failed or is missing; install it with `cargo install wasm-bindgen-cli --version {v}`"
            ));
        }
    }
    std::fs::copy(root().join("web/index.html"), out.join("index.html"))
        .map_err(|e| e.to_string())?;
    std::fs::copy(&zip, out.join(format!("base-{version}.zip"))).map_err(|e| e.to_string())?;
    eprintln!("web build in {}", out.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("modules") => modules(),
        Some("pack") => {
            let out = args
                .iter()
                .position(|a| a == "--out")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| root().join("dist"));
            pack(&out)
        }
        Some("web") => {
            let out = args
                .iter()
                .position(|a| a == "--out")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| root().join("dist/web"));
            web(&out)
        }
        Some("wasm-bindgen-version") => {
            println!("{}", wasm_bindgen_version().unwrap_or_default());
            Ok(())
        }
        _ => Err("usage: cargo xtask <modules | pack [--out DIR] | web [--out DIR]>".into()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
