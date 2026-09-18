//! YAML configuration files, one per component.
//!
//! Files live under `$XDG_CONFIG_HOME/shell/`, falling back to
//! `~/.config/shell/`. A component defines its config struct, implements
//! [`File`] with the file's name, and calls `load()`. When the file does not
//! exist it is written from `Default` so there is something to edit. When it
//! exists but lacks keys, those are appended with their defaults, so every
//! option is visible in the file without disturbing what the user wrote.

mod extend;

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Directory under the XDG config home.
pub const DIR: &str = "shell";

/// A component's configuration. Implementors should carry
/// `#[serde(default, deny_unknown_fields)]` so every key is optional and
/// typos are errors.
pub trait File: Default + Serialize + DeserializeOwned {
    /// File name without the `.yaml` extension.
    const NAME: &'static str;

    /// Where [`File::load`] reads from.
    fn path() -> Result<PathBuf> {
        path(Self::NAME)
    }

    /// Reads the file, creating it from the defaults when it is missing and
    /// appending any keys it lacks. A file that exists but does not parse is
    /// an error, not a silent fallback.
    fn load() -> Result<Self> {
        load_from(&Self::path()?)
    }
}

/// `$XDG_CONFIG_HOME/shell/<name>.yaml`.
pub fn path(name: &str) -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".config"),
    };
    Ok(base.join(DIR).join(format!("{name}.yaml")))
}

pub fn load_from<T: Default + Serialize + DeserializeOwned>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let config: T =
                serde_saphyr::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
            let defaults = serialize(&T::default())?;
            if let Some(extended) = extend::extend(&text, &defaults)
                .with_context(|| format!("extending {}", path.display()))?
            {
                fs::write(path, extended).with_context(|| format!("writing {}", path.display()))?;
            }
            Ok(config)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let config = T::default();
            write(&config, path)?;
            Ok(config)
        }
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

fn serialize<T: Serialize>(config: &T) -> Result<String> {
    serde_saphyr::to_string(config).context("serializing default config")
}

fn write<T: Serialize>(config: &T, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(path, serialize(config)?).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Test {
        height: u32,
        nested: Nested,
    }

    impl Default for Test {
        fn default() -> Test {
            Test {
                height: 32,
                nested: Nested::default(),
            }
        }
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Nested {
        enabled: bool,
    }

    impl Default for Nested {
        fn default() -> Nested {
            Nested { enabled: true }
        }
    }

    impl File for Test {
        const NAME: &'static str = "test";
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("shell-config-test-{}-{name}", std::process::id()))
            .join("test.yaml")
    }

    #[test]
    fn missing_file_is_created_from_defaults_and_reads_back() {
        let path = scratch("missing");
        let _ = fs::remove_dir_all(path.parent().unwrap());

        let first: Test = load_from(&path).unwrap();
        assert_eq!(first, Test::default());
        assert!(path.is_file());

        let second: Test = load_from(&path).unwrap();
        assert_eq!(second, Test::default());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn partial_file_keeps_its_values_and_gains_the_missing_keys() {
        let path = scratch("partial");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "# mine\nheight: 40\n").unwrap();

        let config: Test = load_from(&path).unwrap();
        assert_eq!(config.height, 40);
        assert_eq!(config.nested, Nested::default());

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, "# mine\nheight: 40\nnested:\n  enabled: true\n");
        let again: Test = load_from(&path).unwrap();
        assert_eq!(again, config);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = serde_saphyr::from_str::<Test>("hieght: 40\n").unwrap_err();
        assert!(err.to_string().contains("hieght"), "{err}");
    }

    #[test]
    fn path_is_named_after_the_file() {
        let path = Test::path().unwrap();
        assert!(path.ends_with("shell/test.yaml"), "{}", path.display());
    }
}
