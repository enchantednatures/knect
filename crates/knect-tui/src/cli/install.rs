//! Idempotent k9s plugin merge and backup.

use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value};

use super::CliError;
use super::k9s::InstallK9sPluginArgs;
use crate::client::DEFAULT_SOCKET;

const FILE_NAME: &str = "plugins.yaml";
const BACKUP_NAME: &str = "plugins.yaml.bak";

/// Result of a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    /// Plugin file path.
    pub path: PathBuf,
    /// True when the file content changed.
    pub changed: bool,
    /// Rendered YAML.
    pub yaml: String,
    /// Backup path written when the file changed in place.
    pub backup: Option<PathBuf>,
}

/// Installs the knect plugins and reports the outcome.
pub fn run(args: &InstallK9sPluginArgs, socket: Option<&Path>) -> Result<(), CliError> {
    let socket = socket.unwrap_or_else(|| Path::new(DEFAULT_SOCKET));
    let outcome = install(args, socket)?;
    if args.dry_run {
        println!("dry run: would write {}", outcome.path.display());
        print!("{}", outcome.yaml);
        return Ok(());
    }
    if outcome.changed {
        println!("wrote {}", outcome.path.display());
        if let Some(backup) = &outcome.backup {
            println!("backup {}", backup.display());
        }
    } else {
        println!("already up to date: {}", outcome.path.display());
    }
    Ok(())
}

/// Merges the knect shortcuts into `<dir>/plugins.yaml`.
pub fn install(args: &InstallK9sPluginArgs, socket: &Path) -> Result<InstallOutcome, CliError> {
    let directory = match args.dir.as_ref() {
        Some(directory) => directory.clone(),
        None => default_directory()?,
    };
    let path = directory.join(FILE_NAME);
    let existing = if path.exists() {
        Some(fs::read_to_string(&path)?)
    } else {
        None
    };
    let (yaml, changed) = merge(existing.as_deref(), socket, args.force)?;
    let mut backup = None;
    if changed && !args.dry_run {
        fs::create_dir_all(&directory)?;
        if existing.is_some() {
            let backup_path = directory.join(BACKUP_NAME);
            fs::copy(&path, &backup_path)?;
            backup = Some(backup_path);
        }
        fs::write(&path, &yaml)?;
    }
    Ok(InstallOutcome {
        path,
        changed,
        yaml,
        backup,
    })
}

/// Merges the knect plugins into existing YAML, preserving other entries.
pub fn merge(
    existing: Option<&str>,
    socket: &Path,
    force: bool,
) -> Result<(String, bool), CliError> {
    let mut root = match existing.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => match serde_yaml::from_str::<Value>(text) {
            Ok(value) => value,
            Err(error) if force => {
                let _ = error;
                Value::Mapping(Mapping::new())
            }
            Err(error) => {
                return Err(CliError::Message(format!(
                    "cannot parse {FILE_NAME}: {error} (use --force to replace it)"
                )));
            }
        },
        None => Value::Mapping(Mapping::new()),
    };
    if !matches!(root, Value::Mapping(_)) {
        if !force {
            return Err(CliError::Message(format!(
                "{FILE_NAME} must contain a YAML mapping (use --force to replace it)"
            )));
        }
        root = Value::Mapping(Mapping::new());
    }
    let Value::Mapping(root_map) = &mut root else {
        return Err(CliError::Message("cannot build plugin mapping".to_owned()));
    };
    let key = Value::String("plugins".to_owned());
    let plugins = match root_map.remove(&key) {
        Some(Value::Mapping(mapping)) => mapping,
        Some(_) if force => Mapping::new(),
        Some(_) => {
            return Err(CliError::Message(
                "'plugins' must be a YAML mapping (use --force to replace it)".to_owned(),
            ));
        }
        None => Mapping::new(),
    };
    let merged = merge_plugins(plugins, socket)?;
    root_map.insert(key, Value::Mapping(merged));
    serialize(root, existing)
}

fn merge_plugins(plugins: Mapping, socket: &Path) -> Result<Mapping, CliError> {
    let desired = plugins_mapping(socket)?;
    let mut merged = plugins;
    let Value::Mapping(desired) = desired else {
        return Err(CliError::Message("cannot build plugin entries".to_owned()));
    };
    for (name, plugin) in desired {
        merged.insert(name, plugin);
    }
    Ok(merged)
}

fn plugins_mapping(socket: &Path) -> Result<Value, CliError> {
    let socket = socket.to_string_lossy().into_owned();
    let mut plugins = Mapping::new();
    plugins.insert(
        Value::String("knect-topology".to_owned()),
        plugin(
            "Shift-T",
            "Open knect topology for this resource",
            &[
                "graph",
                "$RESOURCE_NAME",
                "-n",
                "$NAMESPACE",
                "--socket",
                &socket,
            ],
        ),
    );
    plugins.insert(
        Value::String("knect-diff".to_owned()),
        plugin(
            "Shift-M",
            "Show the mutation diff for this resource",
            &[
                "diff",
                "$RESOURCE_NAME",
                "-n",
                "$NAMESPACE",
                "--socket",
                &socket,
            ],
        ),
    );
    Ok(Value::Mapping(plugins))
}

fn plugin(shortcut: &str, description: &str, args: &[&str]) -> Value {
    let mut entry = Mapping::new();
    entry.insert(
        Value::String("shortCut".to_owned()),
        Value::String(shortcut.to_owned()),
    );
    entry.insert(
        Value::String("description".to_owned()),
        Value::String(description.to_owned()),
    );
    entry.insert(
        Value::String("scopes".to_owned()),
        Value::Sequence(
            ["ksvc", "broker", "trigger"]
                .into_iter()
                .map(|scope| Value::String(scope.to_owned()))
                .collect(),
        ),
    );
    entry.insert(
        Value::String("command".to_owned()),
        Value::String("knect".to_owned()),
    );
    entry.insert(
        Value::String("args".to_owned()),
        Value::Sequence(
            args.iter()
                .map(|argument| Value::String((*argument).to_owned()))
                .collect(),
        ),
    );
    Value::Mapping(entry)
}

fn serialize(root: Value, existing: Option<&str>) -> Result<(String, bool), CliError> {
    let yaml = serde_yaml::to_string(&root)
        .map_err(|error| CliError::Message(format!("cannot encode plugins YAML: {error}")))?;
    let changed = existing
        .map(str::trim)
        .is_none_or(|previous| previous != yaml.trim());
    Ok((yaml, changed))
}

fn default_directory() -> Result<PathBuf, CliError> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("k9s"));
    }
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::Message("HOME is not set; pass --dir".to_owned()))?;
    Ok(PathBuf::from(home).join(".config").join("k9s"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn merge_preserves_existing_plugins_and_is_idempotent() {
        let existing = "plugins:\n  fred:\n    shortCut: Ctrl-L\n    command: kubectl\n";
        let socket = Path::new("/tmp/knectd.sock");
        let (first, changed) = merge(Some(existing), socket, false).expect("merge");
        assert!(changed);
        let value: Value = serde_yaml::from_str(&first).expect("yaml");
        let plugins = value
            .get("plugins")
            .and_then(Value::as_mapping)
            .expect("plugins");
        assert!(plugins.contains_key(Value::String("fred".to_owned())));
        assert!(plugins.contains_key(Value::String("knect-topology".to_owned())));
        assert!(plugins.contains_key(Value::String("knect-diff".to_owned())));
        let (second, _) = merge(Some(&first), socket, false).expect("second merge");
        assert_eq!(first, second);
    }

    #[test]
    fn install_writes_backup_and_keeps_user_entries() {
        let directory = tempdir().expect("temp");
        let path = directory.path().join(FILE_NAME);
        fs::write(
            &path,
            "plugins:\n  fred:\n    shortCut: Ctrl-L\n    command: kubectl\n",
        )
        .expect("seed");
        let args = InstallK9sPluginArgs {
            dir: Some(directory.path().to_path_buf()),
            force: false,
            dry_run: false,
        };
        let outcome = install(&args, Path::new("/tmp/knectd.sock")).expect("install");
        assert!(outcome.changed);
        assert_eq!(outcome.backup, Some(directory.path().join(BACKUP_NAME)));
        assert!(path.exists());
        assert!(directory.path().join(BACKUP_NAME).exists());
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("fred"));
        assert!(text.contains("knect-topology"));
        let again = install(&args, Path::new("/tmp/knectd.sock")).expect("second install");
        assert!(!again.changed);
    }

    #[test]
    fn malformed_files_require_force() {
        assert!(merge(Some("not: [valid"), Path::new("/tmp/a.sock"), false).is_err());
        let (yaml, _) = merge(Some("not: [valid"), Path::new("/tmp/a.sock"), true).expect("force");
        assert!(yaml.contains("knect-topology"));
    }

    #[test]
    fn dry_run_does_not_write() {
        let directory = tempdir().expect("temp");
        let path = directory.path().join(FILE_NAME);
        let args = InstallK9sPluginArgs {
            dir: Some(directory.path().to_path_buf()),
            force: false,
            dry_run: true,
        };
        let outcome = install(&args, Path::new("/tmp/knectd.sock")).expect("dry run");
        assert!(outcome.changed);
        assert!(!path.exists());
        assert!(outcome.yaml.contains("Shift-T"));
        assert!(outcome.yaml.contains("Shift-M"));
    }
}
