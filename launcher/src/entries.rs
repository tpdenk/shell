//! Installed desktop applications: discovery, fuzzy search, and launching.

use std::borrow::Cow;
use std::collections::HashSet;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use freedesktop_desktop_entry::{DesktopEntry, Iter, default_paths};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// One launchable application.
pub struct Entry {
    pub name: String,
    pub generic_name: Option<String>,
    keywords: Vec<String>,
    /// Exec with field codes stripped, argv form.
    exec: Vec<String>,
    terminal: bool,
    /// `Path=` key, working directory for the process.
    cwd: Option<PathBuf>,
}

impl Entry {
    /// Match quality against `pattern`. A name match counts in full, a
    /// generic name or keyword match at half, so a name match always
    /// outranks a keyword-only match of equal quality.
    fn score(&self, pattern: &Pattern, matcher: &mut Matcher, buf: &mut Vec<char>) -> Option<u32> {
        if let Some(score) = pattern.score(Utf32Str::new(&self.name, buf), matcher) {
            return Some(score);
        }
        self.generic_name
            .iter()
            .chain(&self.keywords)
            .filter_map(|text| pattern.score(Utf32Str::new(text, buf), matcher))
            .max()
            .map(|score| score / 2)
    }
}

/// The installed applications plus what is needed to search and start them.
/// Entries are sorted by name (case-insensitive). `search` ties keep that
/// order.
pub struct Catalog {
    entries: Vec<Entry>,
    /// Command that runs `Terminal=true` entries, from the config file.
    terminal: String,
    matcher: Matcher,
    /// Scratch for `Utf32Str::new`, reused across scores.
    buf: Vec<char>,
}

impl Catalog {
    /// Reads every desktop entry on this system.
    pub fn load(terminal: String) -> Catalog {
        Catalog::new(discover(), terminal)
    }

    fn new(mut entries: Vec<Entry>, terminal: String) -> Catalog {
        entries.sort_by_cached_key(|e| e.name.to_lowercase());
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        Catalog {
            entries,
            terminal,
            matcher: Matcher::new(config),
            buf: Vec::new(),
        }
    }

    /// Indices of entries matching `query`, best first. Empty query:
    /// everything.
    pub fn search(&mut self, query: &str) -> Vec<usize> {
        if query.trim().is_empty() {
            return (0..self.entries.len()).collect();
        }
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut scored: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .score(&pattern, &mut self.matcher, &mut self.buf)
                    .map(|score| (score, index))
            })
            .collect();
        // Stable, so equal scores keep the catalog's name order.
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, index)| index).collect()
    }

    pub fn entry(&self, index: usize) -> &Entry {
        &self.entries[index]
    }

    /// Spawns the entry detached from this process.
    pub fn launch(&self, index: usize) -> Result<()> {
        let entry = &self.entries[index];
        let mut argv: Vec<&str> = Vec::with_capacity(entry.exec.len() + 2);
        if entry.terminal {
            if self.terminal.trim().is_empty() {
                bail!("no terminal configured for {}", entry.name);
            }
            argv.extend(self.terminal.split_whitespace());
        }
        argv.extend(entry.exec.iter().map(String::as_str));

        let mut command = Command::new(argv[0]);
        command
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Own process group: a Ctrl+C aimed at the launcher's terminal
            // must not reach the application.
            .process_group(0);
        if let Some(cwd) = &entry.cwd {
            command.current_dir(cwd);
        }
        // The child is never waited on. The launcher exits right after and
        // the child is reparented.
        command
            .spawn()
            .with_context(|| format!("launching {}", argv.join(" ")))?;
        Ok(())
    }
}

/// Every launchable entry from the XDG data directories. The first
/// occurrence of an id wins, so `$XDG_DATA_HOME/applications` overrides the
/// system directories.
fn discover() -> Vec<Entry> {
    let locales = freedesktop_desktop_entry::get_languages_from_env();
    let desktops = freedesktop_desktop_entry::current_desktop().unwrap_or_default();
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for entry in Iter::new(default_paths()).entries(Some(&locales)) {
        if !seen.insert(entry.id().to_owned()) || !launchable(&entry, &desktops) {
            continue;
        }
        let exec = match entry.parse_exec() {
            Ok(exec) if !exec.is_empty() => exec,
            Ok(_) => continue,
            Err(err) => {
                log::debug!("skipping {}: {err}", entry.path.display());
                continue;
            }
        };
        let Some(name) = entry.name(&locales) else {
            continue;
        };
        entries.push(Entry {
            name: name.into_owned(),
            generic_name: entry.generic_name(&locales).map(Cow::into_owned),
            keywords: entry
                .keywords(&locales)
                .unwrap_or_default()
                .into_iter()
                .map(Cow::into_owned)
                .collect(),
            exec,
            terminal: entry.terminal(),
            cwd: entry.path().map(PathBuf::from),
        });
    }
    entries
}

/// Whether a menu on the `desktops` (lowercased `$XDG_CURRENT_DESKTOP`
/// components) should offer `entry`.
fn launchable(entry: &DesktopEntry, desktops: &[String]) -> bool {
    if entry.type_() != Some("Application") || entry.no_display() || entry.hidden() {
        return false;
    }
    let listed = |list: Option<Vec<&str>>| {
        list.is_some_and(|list| {
            list.iter()
                .any(|d| desktops.iter().any(|current| current.eq_ignore_ascii_case(d)))
        })
    };
    if let Some(only) = entry.only_show_in()
        && !listed(Some(only))
    {
        return false;
    }
    if listed(entry.not_show_in()) {
        return false;
    }
    entry.try_exec().is_none_or(on_path)
}

/// `exe` names an existing file, either by path or through `$PATH`.
fn on_path(exe: &str) -> bool {
    if exe.contains('/') {
        return Path::new(exe).is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join(exe).is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, keywords: &[&str]) -> Entry {
        Entry {
            name: name.to_owned(),
            generic_name: None,
            keywords: keywords.iter().map(|k| (*k).to_owned()).collect(),
            exec: vec![name.to_lowercase()],
            terminal: false,
            cwd: None,
        }
    }

    fn catalog() -> Catalog {
        Catalog::new(
            vec![
                entry("Files", &[]),
                entry("Nautilus", &["files"]),
                entry("Terminal", &[]),
            ],
            String::new(),
        )
    }

    #[test]
    fn name_match_outranks_keyword_match() {
        assert_eq!(catalog().search("files"), [0, 1]);
    }

    #[test]
    fn empty_query_lists_everything_in_name_order() {
        assert_eq!(catalog().search(""), [0, 1, 2]);
    }

    #[test]
    fn no_match_is_empty() {
        assert_eq!(catalog().search("zzz"), Vec::<usize>::new());
    }

    fn desktop_entry(body: &str) -> DesktopEntry {
        let input = format!("[Desktop Entry]\nType=Application\nName=App\nExec=app\n{body}");
        DesktopEntry::from_str("/tmp/app.desktop", &input, None::<&[&str]>).unwrap()
    }

    #[test]
    fn hidden_entries_are_not_launchable() {
        let hyprland = ["hyprland".to_owned()];
        assert!(launchable(&desktop_entry(""), &hyprland));
        assert!(!launchable(&desktop_entry("NoDisplay=true\n"), &hyprland));
    }

    #[test]
    fn only_show_in_filters_by_current_desktop() {
        let entry = desktop_entry("OnlyShowIn=GNOME;\n");
        assert!(!launchable(&entry, &["hyprland".to_owned()]));
        assert!(launchable(&entry, &["gnome".to_owned()]));
    }
}
