use clap::CommandFactory;
use clap_complete::{generate_to, Shell};
use std::{env, fs};

include!("src/cli_command.rs");

fn main() {
    let outdir = std::path::PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let mut cmd = Cli::command();

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
        generate_to(shell, &mut cmd, "loops", &outdir).expect("generate completions");
    }

    let man = clap_mangen::Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf).expect("render man page");
    fs::write(outdir.join("loops.1"), buf).expect("write man page");

    // Stable path for cargo-dist `include` (gitignored). Skip during `cargo publish`
    // verify — build scripts must not modify the source tree outside OUT_DIR.
    let manifest_dir =
        std::path::PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    if !in_publish_verify_tree(&manifest_dir) {
        let artifacts = manifest_dir.join("dist-artifacts");
        fs::create_dir_all(&artifacts).expect("create dist-artifacts");
        for entry in fs::read_dir(&outdir).expect("read OUT_DIR") {
            let entry = entry.expect("dirent");
            let path = entry.path();
            if path.is_file() {
                let dest = artifacts.join(entry.file_name());
                fs::copy(&path, &dest).expect("copy artifact");
            }
        }
    }

    println!("cargo:rerun-if-changed=src/cli_command.rs");
}

fn in_publish_verify_tree(manifest_dir: &std::path::Path) -> bool {
    let components = manifest_dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    components
        .windows(2)
        .any(|w| w[0] == "target" && w[1] == "package")
}
