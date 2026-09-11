use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, clap::Args)]
pub struct InstallArgs {
    /// Install for all users in /usr/local/bin (usually requires sudo).
    #[arg(long, conflicts_with = "dir")]
    system: bool,
    /// Install in this directory instead of ~/.local/bin.
    #[arg(long, value_name = "DIRECTORY")]
    dir: Option<PathBuf>,
}

pub fn run(arguments: InstallArgs) -> Result<(), Box<dyn std::error::Error>> {
    if !cfg!(unix) {
        return Err("Self-installation is supported on macOS and Linux only.".into());
    }
    let directory = if arguments.system {
        PathBuf::from("/usr/local/bin")
    } else if let Some(directory) = arguments.dir {
        if directory.as_os_str().is_empty() {
            return Err("Installation directory must not be empty.".into());
        }
        directory
    } else {
        home::home_dir()
            .ok_or("Cannot determine your home directory; use --dir DIRECTORY.")?
            .join(".local/bin")
    };
    let source = env::current_exe()?;
    let destination = directory.join("kestrel");
    match copy_executable(&source, &destination) {
        Ok(InstallOutcome::Installed) => {
            println!("Installed: {}", fs::canonicalize(&destination)?.display())
        }
        Ok(InstallOutcome::AlreadyInstalled) => {
            println!("Already installed: {}", destination.display())
        }
        Ok(InstallOutcome::Cancelled) => {
            println!("Installation cancelled; nothing was replaced.");
            return Ok(());
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            let hint = if arguments.system {
                "Run with sudo for --system, or omit --system to install for your user."
            } else {
                "Choose a writable directory with --dir DIRECTORY."
            };
            return Err(format!(
                "Cannot install to {}: {error}. {hint}",
                destination.display()
            )
            .into());
        }
        Err(error) => return Err(error.into()),
    }
    report_path(&destination)?;
    println!("Verify with: kestrel --version");
    println!("Optional agent setup: kestrel skill install");
    Ok(())
}

enum InstallOutcome {
    Installed,
    AlreadyInstalled,
    Cancelled,
}

fn copy_executable(source: &Path, destination: &Path) -> io::Result<InstallOutcome> {
    let replace = match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.is_file() && fs::canonicalize(source)? == fs::canonicalize(destination)? {
                return Ok(InstallOutcome::AlreadyInstalled);
            }
            if !metadata.is_file() && !metadata.is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "{} exists and is not a file or symlink; nothing was replaced.",
                        destination.display()
                    ),
                ));
            }
            eprint!(
                "{} already exists. Replace it? [y/N] ",
                destination.display()
            );
            io::stderr().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                return Ok(InstallOutcome::Cancelled);
            }
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let directory = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Destination has no parent directory",
        )
    })?;
    fs::create_dir_all(directory)?;
    // Stage in the destination directory so a failed copy leaves the old binary
    // intact. Only overwrite when the user explicitly confirmed replacement.
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    io::copy(&mut fs::File::open(source)?, staged.as_file_mut())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Preserve ordinary access bits, but never propagate setuid/setgid bits.
        let mode = fs::metadata(source)?.permissions().mode() & 0o777;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    staged.as_file().sync_all()?;
    if replace {
        staged.persist(destination).map_err(|error| error.error)?;
    } else {
        staged
            .persist_noclobber(destination)
            .map_err(|error| error.error)?;
    }
    Ok(InstallOutcome::Installed)
}

fn report_path(destination: &Path) -> io::Result<()> {
    let destination = fs::canonicalize(destination)?;
    let directory = destination.parent().expect("executable has a parent");
    let paths = env::var_os("PATH").unwrap_or_default();
    let directories: Vec<_> = env::split_paths(&paths).collect();
    let on_path = directories
        .iter()
        .any(|entry| fs::canonicalize(entry).is_ok_and(|entry| entry == directory));
    if !on_path {
        let quoted = format!("'{}'", directory.to_string_lossy().replace('\'', "'\\''"));
        println!(
            "Add {} to PATH to run kestrel from any directory.",
            directory.display()
        );
        println!("For sh/bash/zsh, run this now and add it to your shell startup file:");
        println!("  export PATH={quoted}:\"$PATH\"");
        println!("For fish, run:");
        let fish_quoted = format!(
            "'{}'",
            directory
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
        );
        println!("  fish_add_path {fish_quoted}");
    }
    for entry in directories {
        let candidate = entry.join("kestrel");
        if !is_executable(&candidate) {
            continue;
        }
        if fs::canonicalize(&candidate)? != destination {
            println!(
                "PATH currently selects another installation: {}. Put {} first in PATH to use this copy.",
                candidate.display(),
                directory.display()
            );
        }
        break;
    }
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}
