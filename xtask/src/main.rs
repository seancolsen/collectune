use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn usage() {
    eprintln!(
        "cargo xtask <command>\n\n\
         Commands:\n  \
           build-release   Build the WASM frontend with trunk and the production binary\n  \
           clean-web       Remove the frontend/dist directory"
    );
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must live in a workspace")
        .to_path_buf()
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn `{cmd:?}`: {e}"))?;
    if !status.success() {
        return Err(format!("`{cmd:?}` exited with {status}"));
    }
    Ok(())
}

fn build_release() -> Result<(), String> {
    let root = workspace_root();
    let frontend = root.join("frontend");

    let trunk_check = Command::new("trunk").arg("--version").output();
    if trunk_check.is_err() || !trunk_check.unwrap().status.success() {
        return Err(
            "`trunk` is required. Install with: cargo install --locked trunk\n\
             You also need the wasm target: rustup target add wasm32-unknown-unknown"
                .into(),
        );
    }

    println!("==> trunk build --release");
    run(Command::new("trunk")
        .args(["build", "--release"])
        .current_dir(&frontend))?;

    println!("==> cargo build --release -p collectune");
    run(Command::new("cargo")
        .args(["build", "--release", "-p", "collectune"])
        .current_dir(&root))?;

    let bin = root.join("target/release/collectune");
    println!("\nBuilt: {}", bin.display());
    Ok(())
}

fn clean_web() -> Result<(), String> {
    let dist = workspace_root().join("frontend/dist");
    if dist.exists() {
        std::fs::remove_dir_all(&dist).map_err(|e| e.to_string())?;
        println!("Removed {}", dist.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next() else {
        usage();
        return ExitCode::from(2);
    };

    let result = match cmd.as_str() {
        "build-release" => build_release(),
        "clean-web" => clean_web(),
        "--help" | "-h" | "help" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command: {other}")),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
