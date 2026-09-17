use std::fmt::{self, Write as _};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

pub type WorkspaceId = i32;

/// The Hyprland instance this process runs under, located through
/// `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`.
#[derive(Clone, Debug)]
pub struct Instance {
    request: PathBuf,
    events: PathBuf,
}

impl Instance {
    /// Fails when not running under Hyprland.
    pub fn from_env() -> Result<Instance> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
            .context("HYPRLAND_INSTANCE_SIGNATURE is not set, not running under Hyprland")?;
        let dir = PathBuf::from(runtime).join("hypr").join(signature);
        let request = dir.join(".socket.sock");
        if !request.exists() {
            bail!("Hyprland socket {} does not exist", request.display());
        }
        Ok(Instance {
            request,
            events: dir.join(".socket2.sock"),
        })
    }

    pub fn request(&self, command: &str) -> Result<String> {
        let mut stream = UnixStream::connect(&self.request)
            .with_context(|| format!("connecting to {}", self.request.display()))?;
        stream
            .write_all(command.as_bytes())
            .context("sending Hyprland IPC request")?;
        let mut reply = String::new();
        stream
            .read_to_string(&mut reply)
            .context("reading Hyprland IPC reply")?;
        Ok(reply)
    }

    fn json<T: DeserializeOwned>(&self, command: &str) -> Result<T> {
        let reply = self.request(&format!("j/{command}"))?;
        serde_json::from_str(&reply).with_context(|| format!("parsing reply to {command}"))
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        self.json("workspaces")
    }

    pub fn monitors(&self) -> Result<Vec<Monitor>> {
        self.json("monitors")
    }

    /// Runs a Lua dispatcher expression, e.g. `hl.dsp.window.close()`.
    pub fn dispatch(&self, dispatcher: &str) -> Result<()> {
        let reply = self.request(&format!("dispatch {dispatcher}"))?;
        if reply != "ok" {
            bail!("{}", reply.trim());
        }
        Ok(())
    }

    /// Switches the focused monitor to a workspace.
    pub fn focus_workspace(&self, workspace: WorkspaceSelector<'_>) -> Result<()> {
        self.dispatch(&format!("hl.dsp.focus({{ workspace = {workspace} }})"))
    }

    /// Connects to the event socket. The stream ends when Hyprland exits.
    pub fn events(&self) -> Result<Events> {
        let stream = UnixStream::connect(&self.events)
            .with_context(|| format!("connecting to {}", self.events.display()))?;
        Ok(Events {
            reader: BufReader::new(stream),
            line: String::new(),
        })
    }
}

/// How Hyprland's Lua API identifies a workspace. Displays as the Lua
/// literal: a number for `Id`, a quoted string for `Name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceSelector<'a> {
    Id(WorkspaceId),
    /// A workspace string as accepted by the classic `workspace` dispatcher,
    /// such as `special:magic`, `name:web`, `empty`, or `previous`.
    Name(&'a str),
}

impl fmt::Display for WorkspaceSelector<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceSelector::Id(id) => write!(f, "{id}"),
            WorkspaceSelector::Name(name) => {
                f.write_char('"')?;
                for c in name.chars() {
                    match c {
                        '\\' => f.write_str("\\\\")?,
                        '"' => f.write_str("\\\"")?,
                        '\n' => f.write_str("\\n")?,
                        '\r' => f.write_str("\\r")?,
                        c => f.write_char(c)?,
                    }
                }
                f.write_char('"')
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub monitor: String,
    pub windows: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub focused: bool,
    pub active_workspace: WorkspaceRef,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRef {
    pub id: WorkspaceId,
    pub name: String,
}

/// One line of the event socket, `name>>data`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub data: String,
}

pub struct Events {
    reader: BufReader<UnixStream>,
    line: String,
}

impl Iterator for Events {
    type Item = io::Result<Event>;

    fn next(&mut self) -> Option<io::Result<Event>> {
        loop {
            self.line.clear();
            match self.reader.read_line(&mut self.line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(err) => return Some(Err(err)),
            }
            let line = self.line.trim_end_matches(['\n', '\r']);
            // Hyprland only emits `name>>data`. Anything else is noise.
            if let Some((name, data)) = line.split_once(">>") {
                return Some(Ok(Event {
                    name: name.to_owned(),
                    data: data.to_owned(),
                }));
            }
        }
    }
}
