//! Registering protonctl with a host: Claude Desktop's
//! configuration file, Claude Code's server list, and Claude Code's allow
//! rules for the tools that only read.
//!
//! Each file belongs to another program. A change sets one member, keeps
//! every other member as that program wrote it and in its order, leaves the
//! old file beside the new one as `.bak`, and refuses a file it cannot read
//! as a JSON object rather than replace it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value, json, value::RawValue};

use crate::config;

/// The server's name in every host; each tool's name there carries it.
const SERVER: &str = "proton";
/// How a tool's name starts in Claude Code when the server was added by hand.
const BY_HAND: &str = "mcp__proton__";
/// How it starts when the plugin added the server.
const BY_PLUGIN: &str = "mcp__plugin_protonctl_proton__";
/// What a dry run that found something to change says to do next.
const AFTER_DRY_RUN: &str = "run this again without --dry-run to make the change";
/// The tools that only read, as Claude Code allow rules without the server's
/// prefix: a tool's name, or the start of one and `*`. The three tools that
/// can save a file are left to ask. A rule such as `get_*` would cover
/// `get_attachment`, so a test holds this list to the tools' own hints.
pub const READ_ONLY_RULES: [&str; 9] = [
    "get_status",
    "get_event",
    "get_file_metadata",
    "get_message",
    "get_thread",
    "list_*",
    "search_*",
    "count_*",
    "read_*",
];

#[derive(clap::Subcommand, Debug)]
pub enum Host {
    /// Add protonctl to Claude Desktop's configuration file. Quit Claude Desktop first.
    ClaudeDesktop {
        #[command(flatten)]
        change: Change,
    },
    /// Add protonctl to Claude Code with `claude mcp add`.
    ClaudeCode {
        /// Also allow the tools that only read, in ~/.claude/settings.json; the three that can save a file still ask.
        #[arg(long)]
        allow_read_tools: bool,
        /// The plugin adds the server itself: only write the allow rules, under the plugin's tool names.
        #[arg(long, requires = "allow_read_tools")]
        plugin: bool,
        #[command(flatten)]
        change: Change,
    },
}

/// How a `connect` command changes its host.
#[derive(clap::Args, Clone, Copy, Debug, Default)]
pub struct Change {
    /// Take out what this command adds.
    #[arg(long)]
    remove: bool,
    /// Print what would change, and change nothing.
    #[arg(long)]
    dry_run: bool,
}

/// What a change did to the one thing it sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Done {
    Set,
    AlreadySet,
    Removed,
    NotThere,
}

/// Run `connect` for `host` and return what it did, for the terminal.
pub fn run(host: &Host) -> Result<Value> {
    match *host {
        Host::ClaudeDesktop { change } => {
            if !cfg!(target_os = "macos") {
                bail!(
                    "connect claude-desktop knows Claude Desktop's configuration file on macOS only"
                );
            }
            let file = config::home()
                .join("Library/Application Support/Claude/claude_desktop_config.json");
            let command = if change.remove {
                None
            } else {
                Some(own_path()?)
            };
            claude_desktop(&file, command.as_deref(), change, desktop_is_running)
        }
        Host::ClaudeCode {
            allow_read_tools,
            plugin,
            change,
        } => {
            let mut out = serde_json::Map::new();
            if !plugin {
                let command = if change.remove {
                    None
                } else {
                    Some(own_path()?)
                };
                let server = claude_code(OsStr::new("claude"), command.as_deref(), change)?;
                out.insert("server".into(), server);
            }
            if allow_read_tools {
                let file = config::home().join(".claude/settings.json");
                let prefix = if plugin { BY_PLUGIN } else { BY_HAND };
                out.insert("allowRules".into(), allow_rules(&file, prefix, change)?);
            }
            Ok(Value::Object(out))
        }
    }
}

/// The paths an install puts protonctl at, in the order the plugin's
/// launcher, `plugin/scripts/serve`, tries them: `scripts/install.sh`'s, then
/// Homebrew's on a Mac with Apple silicon and on Linux.
fn install_paths() -> [PathBuf; 3] {
    [
        config::home().join(".cargo/bin/protonctl"),
        PathBuf::from("/opt/homebrew/bin/protonctl"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin/protonctl"),
    ]
}

/// The install path that is this running protonctl.
fn own_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot find this program's own path")?;
    installed_as(&exe, &install_paths())
}

/// Which of `installs` is the program at `exe`. A host holds this path, so
/// it is the one an upgrade keeps: Homebrew's link, never the versioned
/// folder the link points into, and never a build in a source tree.
fn installed_as(exe: &Path, installs: &[PathBuf]) -> Result<PathBuf> {
    let me = std::fs::canonicalize(exe)
        .with_context(|| format!("cannot find this program at {}", exe.display()))?;
    installs
        .iter()
        .find(|install| std::fs::canonicalize(install).is_ok_and(|at| at == me))
        .cloned()
        .with_context(|| {
            format!(
                "this protonctl runs from {}, which is not an installed one; install it with \
                 `brew install matt-w-horn/tap/protonctl` or `scripts/install.sh`, then run \
                 `protonctl connect` again",
                me.display()
            )
        })
}

/// One member of a JSON object in another program's file.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Member {
    /// As the file held it, byte for byte.
    Kept(Box<RawValue>),
    /// An object this change rewrites, its own members kept in turn.
    Object(Object),
    /// A value this change writes.
    New(Value),
}

/// A JSON object with its members in the file's order.
type Object = IndexMap<String, Member>;

/// The members of the JSON object `text`, each kept as written.
fn object(text: &str) -> serde_json::Result<Object> {
    let members: IndexMap<String, Box<RawValue>> = serde_json::from_str(text)?;
    Ok(members
        .into_iter()
        .map(|(name, value)| (name, Member::Kept(value)))
        .collect())
}

/// Another program's JSON file: the path that is written, the text it held,
/// and its members.
#[derive(Debug)]
struct Settings {
    /// The file itself, where `path` was a link to it.
    path: PathBuf,
    /// `None` when there is no file yet.
    old: Option<String>,
    members: Object,
}

impl Settings {
    /// Read the file at `path`; one that is not there has no members. A
    /// file that is not a JSON object is refused, since writing would
    /// replace it.
    fn read(path: &Path) -> Result<Self> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let old = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        let text = old.as_deref().unwrap_or_default();
        let members = if text.trim().is_empty() {
            Object::new()
        } else {
            object(text).map_err(|e| {
                let why = if e.is_data() {
                    "does not hold a JSON object".to_string()
                } else {
                    format!("is not valid JSON ({e})")
                };
                anyhow!("{} {why}; nothing was changed", path.display())
            })?
        };
        Ok(Self { path, old, members })
    }

    /// The object member `name`, opened for a change: an empty one when the
    /// file has none. A member that is not an object is refused.
    fn open(&mut self, name: &str) -> Result<&mut Object> {
        let member = self
            .members
            .entry(name.to_string())
            .or_insert_with(|| Member::Object(Object::new()));
        if let Member::Kept(raw) = member {
            let inner = object(raw.get()).ok().with_context(|| {
                format!(
                    "{name} in {} is not a JSON object; nothing was changed",
                    self.path.display()
                )
            })?;
            *member = Member::Object(inner);
        }
        match member {
            Member::Object(inner) => Ok(inner),
            Member::Kept(_) | Member::New(_) => unreachable!("opened as an object above"),
        }
    }

    /// Replace the file with the members as they are now, in one step, and
    /// return the copy of the old file left beside it.
    fn write(&self) -> Result<Option<PathBuf>> {
        let mut text = serde_json::to_string_pretty(&self.members)?;
        text.push('\n');
        let backup = match &self.old {
            Some(_) => {
                let mut name = self.path.clone().into_os_string();
                name.push(".bak");
                let backup = PathBuf::from(name);
                std::fs::copy(&self.path, &backup)
                    .with_context(|| format!("cannot write {}", backup.display()))?;
                Some(backup)
            }
            None => None,
        };
        config::replace(&self.path, &text)?;
        Ok(backup)
    }
}

/// The strings of the array member `name` of `permissions`; none when the
/// member is absent. One that is not an array of strings is refused.
fn rule_list(permissions: &Object, name: &str) -> Result<Vec<String>> {
    let Some(member) = permissions.get(name) else {
        return Ok(Vec::new());
    };
    let Member::Kept(raw) = member else {
        unreachable!("a member is rewritten only after every list is read");
    };
    serde_json::from_str(raw.get())
        .ok()
        .with_context(|| format!("permissions.{name} is not a list of rules; nothing was changed"))
}

/// Whether Claude Desktop is running, by its process name. On the
/// maintainer's Mac on 2026-11-17, a change to its configuration file made
/// while it ran did not take effect, and one made after quitting it did.
fn desktop_is_running() -> bool {
    Command::new("/usr/bin/pgrep")
        .args(["-x", "Claude"])
        .output()
        .is_ok_and(|found| found.status.success())
}

/// Set `mcpServers.proton` in Claude Desktop's configuration `file` to run
/// `command`, or with `change.remove` take it out. `running` says whether
/// the app runs now; a change is refused while it does.
fn claude_desktop(
    file: &Path,
    command: Option<&Path>,
    change: Change,
    running: fn() -> bool,
) -> Result<Value> {
    let folder = file.parent().context("the file has no folder")?;
    if !folder.is_dir() {
        bail!(
            "{} is not there, so Claude Desktop looks not installed or never started; nothing was changed",
            folder.display()
        );
    }
    let mut settings = Settings::read(file)?;
    let servers = settings.open("mcpServers")?;
    let done = match command {
        Some(command) => {
            runs(command)?;
            let entry = json!({ "command": command, "args": ["serve"] });
            let held: Option<Value> = match servers.get(SERVER) {
                Some(Member::Kept(raw)) => serde_json::from_str(raw.get()).ok(),
                _ => None,
            };
            if held.as_ref() == Some(&entry) {
                Done::AlreadySet
            } else {
                let mut server = Object::new();
                server.insert("command".into(), Member::New(json!(command)));
                server.insert("args".into(), Member::New(json!(["serve"])));
                servers.insert(SERVER.into(), Member::Object(server));
                Done::Set
            }
        }
        None if servers.shift_remove(SERVER).is_some() => Done::Removed,
        None => Done::NotThere,
    };
    let changes = matches!(done, Done::Set | Done::Removed);
    if changes && !change.dry_run && running() {
        bail!(
            "Claude Desktop is running; quit it (Cmd+Q), then run this again. A change made while \
             it runs may not take effect. Nothing was changed"
        );
    }
    let backup = if changes && !change.dry_run {
        settings.write()?
    } else {
        None
    };
    let next = match (changes, change.dry_run) {
        (true, true) => AFTER_DRY_RUN,
        (true, false) => "start Claude Desktop",
        (false, _) => "nothing to do",
    };
    Ok(json!({
        "host": "Claude Desktop",
        "file": settings.path,
        "command": command,
        "done": done,
        "dryRun": change.dry_run,
        "backup": backup,
        "next": next,
    }))
}

/// Check that a host can start the program at `command`.
fn runs(command: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let executable = command.is_absolute()
        && std::fs::metadata(command)
            .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0);
    if !executable {
        bail!(
            "no protonctl to run at \"{}\"; nothing was changed",
            command.display()
        );
    }
    Ok(())
}

/// Add the server to Claude Code through its own command, `claude`, so
/// that it runs `command`; or with `change.remove` take it out. Claude Code
/// keeps its server list in a file whose format it does not document, so
/// protonctl does not write that file.
fn claude_code(claude: &OsStr, command: Option<&Path>, change: Change) -> Result<Value> {
    let remove: Vec<&OsStr> = ["mcp", "remove", "--scope", "user", SERVER]
        .map(OsStr::new)
        .to_vec();
    let add: Option<Vec<&OsStr>> = match command {
        Some(command) => {
            runs(command)?;
            let mut add = ["mcp", "add", "--scope", "user", SERVER, "--"]
                .map(OsStr::new)
                .to_vec();
            add.extend([command.as_os_str(), OsStr::new("serve")]);
            Some(add)
        }
        None => None,
    };
    let shown = |args: &[&OsStr]| {
        let args: Vec<_> = args.iter().map(|a| a.to_string_lossy()).collect();
        format!("claude {}", args.join(" "))
    };
    let wanted = shown(add.as_deref().unwrap_or(&remove));
    if change.dry_run {
        return Ok(json!({ "host": "Claude Code", "wouldRun": wanted, "dryRun": true }));
    }
    // An entry at another path is replaced, so one that is absent is no error.
    let removed = match Command::new(claude).args(&remove).output() {
        Ok(out) => out.status.success(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Claude Code's `claude` command was not found on your PATH; nothing was changed. \
                 With Claude Code installed, run: {wanted}"
            );
        }
        Err(e) => return Err(e).context("cannot run `claude`"),
    };
    let done = match &add {
        Some(add) => {
            let out = Command::new(claude)
                .args(add)
                .output()
                .context("cannot run `claude`")?;
            if !out.status.success() {
                bail!(
                    "`{wanted}` failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Done::Set
        }
        None if removed => Done::Removed,
        None => Done::NotThere,
    };
    Ok(json!({
        "host": "Claude Code",
        "ran": wanted,
        "command": command,
        "done": done,
        "dryRun": false,
        "next": "start a new Claude Code session",
    }))
}

/// Add each of `READ_ONLY_RULES`, under `prefix`, to `permissions.allow` in
/// Claude Code's settings `file`, or with `change.remove` take them out. A
/// rule the user has put in `permissions.deny` or `permissions.ask` stays
/// theirs: it is skipped. Nothing else in the file changes.
fn allow_rules(file: &Path, prefix: &str, change: Change) -> Result<Value> {
    let mut settings = Settings::read(file)?;
    let permissions = settings.open("permissions")?;
    let mut allow = rule_list(permissions, "allow")?;
    let theirs = [
        ("deny", rule_list(permissions, "deny")?),
        ("ask", rule_list(permissions, "ask")?),
    ];
    let mut said = Vec::new();
    let mut changes = false;
    for rule in READ_ONLY_RULES.map(|rule| format!("{prefix}{rule}")) {
        let listed = allow.iter().position(|held| *held == rule);
        let theirs = theirs
            .iter()
            .find(|(_, rules)| rules.contains(&rule))
            .map(|(list, _)| *list);
        let action = match (change.remove, listed, theirs) {
            (true, Some(at), _) => {
                allow.remove(at);
                changes = true;
                "remove".to_string()
            }
            (true, None, _) => "absent".to_string(),
            (false, Some(_), _) => "there".to_string(),
            (false, None, Some(list)) => format!("skip: it is in permissions.{list}"),
            (false, None, None) => {
                allow.push(rule.clone());
                changes = true;
                "add".to_string()
            }
        };
        said.push(json!({ "rule": rule, "action": action }));
    }
    let backup = if changes && !change.dry_run {
        permissions.insert("allow".into(), Member::New(json!(allow)));
        if let Some(folder) = settings.path.parent() {
            use std::os::unix::fs::DirBuilderExt as _;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(folder)
                .with_context(|| format!("cannot make {}", folder.display()))?;
        }
        settings.write()?
    } else {
        None
    };
    let next = match (changes, change.dry_run) {
        (true, true) => AFTER_DRY_RUN,
        (true, false) => "start a new Claude Code session for the rules to apply",
        (false, _) => "nothing to do",
    };
    Ok(json!({
        "file": settings.path,
        "rules": said,
        "changed": changes && !change.dry_run,
        "dryRun": change.dry_run,
        "backup": backup,
        "next": next,
    }))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    const SET: Change = Change {
        remove: false,
        dry_run: false,
    };
    const REMOVE: Change = Change {
        remove: true,
        dry_run: false,
    };
    const DRY: Change = Change {
        remove: false,
        dry_run: true,
    };

    /// A folder standing in for Claude Desktop's, with a program a host can
    /// start in it, and the configuration file's path.
    fn desktop() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("protonctl");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = dir.path().join("claude_desktop_config.json");
        (dir, program, file)
    }

    fn quit() -> bool {
        false
    }

    fn open() -> bool {
        true
    }

    fn json_of(file: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
    }

    /// The file's other members stay as written and in order, the entry is
    /// set, the old file is kept as `.bak`, and a second run changes nothing.
    #[test]
    fn claude_desktop_gets_the_entry_and_keeps_the_rest() {
        let (_dir, program, file) = desktop();
        let old = "{\n  \"zeta\": {\"b\": 1, \"a\": [1,2]},\n  \"mcpServers\": {\n    \"other\": {\"command\": \"/bin/true\"},\n    \"proton\": {\"command\": \"/gone/protonctl\", \"args\": [\"serve\"]}\n  },\n  \"alpha\": true\n}\n";
        std::fs::write(&file, old).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();

        let out = claude_desktop(&file, Some(&program), SET, quit).unwrap();
        assert_eq!(out["done"], "set", "{out}");
        let new = std::fs::read_to_string(&file).unwrap();
        let expected = format!(
            "{{\n  \"zeta\": {{\"b\": 1, \"a\": [1,2]}},\n  \"mcpServers\": {{\n    \"other\": {{\"command\": \"/bin/true\"}},\n    \"proton\": {{\n      \"command\": \"{}\",\n      \"args\": [\n        \"serve\"\n      ]\n    }}\n  }},\n  \"alpha\": true\n}}\n",
            program.display()
        );
        assert_eq!(new, expected);
        let backup = PathBuf::from(out["backup"].as_str().unwrap());
        assert_eq!(std::fs::read_to_string(backup).unwrap(), old);
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "the file keeps its mode");

        let again = claude_desktop(&file, Some(&program), SET, open).unwrap();
        assert_eq!(
            (&again["done"], &again["backup"]),
            (&json!("alreadySet"), &Value::Null)
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), new);
    }

    /// A file that is not there yet is made, with no `.bak`; `--remove`
    /// takes the entry out and leaves the other servers.
    #[test]
    fn claude_desktop_makes_a_missing_file_and_removes_its_entry() {
        let (_dir, program, file) = desktop();
        let out = claude_desktop(&file, Some(&program), SET, quit).unwrap();
        assert_eq!(
            (&out["done"], &out["backup"]),
            (&json!("set"), &Value::Null)
        );
        assert_eq!(
            json_of(&file),
            json!({ "mcpServers": { "proton": { "command": program, "args": ["serve"] } } })
        );

        std::fs::write(
            &file,
            r#"{"mcpServers": {"proton": {"command": "x"}, "other": {"command": "y"}}}"#,
        )
        .unwrap();
        let out = claude_desktop(&file, None, REMOVE, quit).unwrap();
        assert_eq!(out["done"], "removed");
        assert_eq!(
            json_of(&file),
            json!({ "mcpServers": { "other": { "command": "y" } } })
        );
        let out = claude_desktop(&file, None, REMOVE, quit).unwrap();
        assert_eq!(out["done"], "notThere");
    }

    /// Each refusal says nothing was changed, and the file is the same
    /// bytes after it, with no `.bak` beside it.
    #[test]
    fn claude_desktop_refuses_and_changes_nothing() {
        let (dir, program, file) = desktop();
        let plain = dir.path().join("not-executable");
        std::fs::write(&plain, "x").unwrap();
        let good = r#"{"mcpServers": {}}"#;
        let refused = |text: &str, command: &Path, running: fn() -> bool, names: &str| {
            std::fs::write(&file, text).unwrap();
            let err = claude_desktop(&file, Some(command), SET, running)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(names) && err.to_lowercase().contains("nothing was changed"),
                "{err}"
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "{names}");
            assert!(!file.with_extension("json.bak").exists(), "{names}");
        };
        refused(good, Path::new(""), quit, "no protonctl to run at \"\"");
        refused(
            good,
            &dir.path().join("absent"),
            quit,
            "no protonctl to run",
        );
        refused(good, &plain, quit, "no protonctl to run");
        refused(good, Path::new("protonctl"), quit, "no protonctl to run");
        refused(good, &program, open, "Claude Desktop is running");
        refused("{\"mcpServers\": ", &program, quit, "is not valid JSON");
        refused("[1, 2]", &program, quit, "does not hold a JSON object");
        refused(r#"{"mcpServers": []}"#, &program, quit, "mcpServers in");

        let nowhere = dir.path().join("no-such-folder/claude_desktop_config.json");
        let err = claude_desktop(&nowhere, Some(&program), SET, quit).unwrap_err();
        assert!(err.to_string().contains("nothing was changed"), "{err}");
        assert!(!nowhere.parent().unwrap().exists());
    }

    /// `--dry-run` says what it would do and writes nothing, even while the
    /// app runs.
    #[test]
    fn a_dry_run_changes_nothing() {
        let (_dir, program, file) = desktop();
        let text = r#"{"mcpServers": {}}"#;
        std::fs::write(&file, text).unwrap();
        let out = claude_desktop(&file, Some(&program), DRY, open).unwrap();
        assert_eq!(
            (&out["done"], &out["dryRun"]),
            (&json!("set"), &json!(true))
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        assert!(!file.with_extension("json.bak").exists());

        let settings = file.with_file_name("settings.json");
        let out = allow_rules(&settings, BY_HAND, DRY).unwrap();
        assert_eq!(out["changed"], false);
        assert_eq!(out["rules"][0]["action"], "add");
        assert!(!settings.exists());
    }

    /// The path a host is given is the install path that is this program:
    /// a link to it counts, a build elsewhere is refused with the ways to
    /// install, and the list is the launcher's.
    #[test]
    fn the_host_gets_the_install_path_that_is_this_program() {
        let dir = tempfile::tempdir().unwrap();
        let versioned = dir.path().join("Cellar/protonctl/0.1.7/bin/protonctl");
        std::fs::create_dir_all(versioned.parent().unwrap()).unwrap();
        std::fs::write(&versioned, "x").unwrap();
        let link = dir.path().join("bin-protonctl");
        std::os::unix::fs::symlink(&versioned, &link).unwrap();
        let other = dir.path().join("other-protonctl");
        std::fs::write(&other, "y").unwrap();
        let installs = [dir.path().join("absent"), other.clone(), link.clone()];

        assert_eq!(installed_as(&versioned, &installs).unwrap(), link);
        assert_eq!(installed_as(&link, &installs).unwrap(), link);
        assert_eq!(installed_as(&other, &installs).unwrap(), other);
        let built = dir.path().join("target-protonctl");
        std::fs::write(&built, "z").unwrap();
        let err = installed_as(&built, &installs).unwrap_err().to_string();
        assert!(
            err.contains("brew install matt-w-horn/tap/protonctl")
                && err.contains("scripts/install.sh"),
            "{err}"
        );

        let launcher = include_str!("../plugin/scripts/serve");
        let tried = launcher
            .lines()
            .find_map(|line| line.strip_prefix("for bin in ")?.strip_suffix("; do"))
            .unwrap();
        let home = config::home();
        let listed: Vec<PathBuf> = tried
            .split(' ')
            .map(|path| match path.trim_matches('"').strip_prefix("$HOME/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(path),
            })
            .collect();
        assert_eq!(listed, install_paths());
    }

    /// A stand-in `claude` that records its arguments, one call a line.
    fn stand_in_claude(dir: &Path, exit_of_remove: i32) -> (PathBuf, PathBuf) {
        let log = dir.join("calls");
        let claude = dir.join("claude");
        std::fs::write(
            &claude,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\n[ \"$2\" = remove ] && exit {exit_of_remove}\nexit 0\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        (claude, log)
    }

    /// Claude Code is given the server through its own command: an entry at
    /// another path is removed first, and one that is absent is no error.
    #[test]
    fn claude_code_is_told_through_its_own_command() {
        let (dir, program, _) = desktop();
        let (claude, log) = stand_in_claude(dir.path(), 1);
        let out = claude_code(claude.as_os_str(), Some(&program), SET).unwrap();
        assert_eq!(out["done"], "set", "{out}");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!(
                "mcp remove --scope user proton\nmcp add --scope user proton -- {} serve\n",
                program.display()
            )
        );

        std::fs::remove_file(&log).unwrap();
        let out = claude_code(claude.as_os_str(), None, REMOVE).unwrap();
        assert_eq!(out["done"], "notThere");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "mcp remove --scope user proton\n"
        );

        std::fs::remove_file(&log).unwrap();
        let out = claude_code(claude.as_os_str(), Some(&program), DRY).unwrap();
        assert!(
            out["wouldRun"].as_str().unwrap().ends_with(" serve"),
            "{out}"
        );
        assert!(!log.exists(), "a dry run runs nothing");

        let err = claude_code(OsStr::new("/no/such/claude"), Some(&program), SET)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("was not found") && err.contains("claude mcp add --scope user proton --"),
            "{err}"
        );
        let err = claude_code(claude.as_os_str(), Some(Path::new("")), SET).unwrap_err();
        assert!(err.to_string().contains("no protonctl to run"), "{err}");
    }

    /// The rules are appended to `permissions.allow` only: a rule in `deny`
    /// or `ask` is skipped, one that is there is left, every other member
    /// stays, and `--remove` takes out these rules and no other.
    #[test]
    fn the_allow_rules_are_added_and_removed_without_touching_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude/settings.json");
        let out = allow_rules(&file, BY_HAND, SET).unwrap();
        assert_eq!(
            (&out["changed"], &out["backup"]),
            (&json!(true), &Value::Null)
        );
        let all: Vec<String> = READ_ONLY_RULES
            .iter()
            .map(|rule| format!("mcp__proton__{rule}"))
            .collect();
        assert_eq!(json_of(&file), json!({ "permissions": { "allow": all } }));
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(file.parent().unwrap()), mode(&file)), (0o700, 0o600));

        let old = r#"{
  "model": "x",
  "permissions": {
    "deny": ["mcp__proton__read_*"],
    "allow": ["Bash(ls:*)", "mcp__proton__get_status"],
    "ask": ["mcp__proton__list_*"],
    "defaultMode": "auto"
  },
  "hooks": {"b": 1, "a": 2}
}
"#;
        std::fs::write(&file, old).unwrap();
        let out = allow_rules(&file, BY_HAND, SET).unwrap();
        let action = |out: &Value, rule: &str| {
            out["rules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|said| said["rule"] == format!("mcp__proton__{rule}"))
                .unwrap()["action"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(action(&out, "get_status"), "there");
        assert_eq!(action(&out, "read_*"), "skip: it is in permissions.deny");
        assert_eq!(action(&out, "list_*"), "skip: it is in permissions.ask");
        assert_eq!(action(&out, "search_*"), "add");
        let new = std::fs::read_to_string(&file).unwrap();
        let (before, rest) = new.split_once("    \"allow\": [").unwrap();
        let (allow, after) = rest.split_once("],\n").unwrap();
        assert_eq!(
            before,
            "{\n  \"model\": \"x\",\n  \"permissions\": {\n    \"deny\": [\"mcp__proton__read_*\"],\n"
        );
        assert_eq!(
            after,
            "    \"ask\": [\"mcp__proton__list_*\"],\n    \"defaultMode\": \"auto\"\n  },\n  \"hooks\": {\"b\": 1, \"a\": 2}\n}\n"
        );
        let allowed: Vec<String> = serde_json::from_str(&format!("[{allow}]")).unwrap();
        assert_eq!(allowed.len(), 2 + 9 - 3, "{}", allowed.join(", "));
        assert_eq!(&allowed[..2], ["Bash(ls:*)", "mcp__proton__get_status"]);
        let backup = PathBuf::from(out["backup"].as_str().unwrap());
        assert_eq!(std::fs::read_to_string(backup).unwrap(), old);

        let out = allow_rules(&file, BY_HAND, REMOVE).unwrap();
        assert_eq!(action(&out, "read_*"), "absent");
        assert_eq!(action(&out, "search_*"), "remove");
        let left = json_of(&file);
        assert_eq!(left["permissions"]["allow"], json!(["Bash(ls:*)"]));
        assert_eq!(left["permissions"]["deny"], json!(["mcp__proton__read_*"]));

        let plugin = allow_rules(&file, BY_PLUGIN, DRY).unwrap();
        assert_eq!(
            plugin["rules"][0]["rule"],
            "mcp__plugin_protonctl_proton__get_status"
        );
    }

    /// A settings file this cannot read as it expects is left as it is.
    #[test]
    fn the_allow_rules_refuse_a_file_they_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        for (text, names) in [
            ("{ not json", "is not valid JSON"),
            ("\"text\"", "does not hold a JSON object"),
            (r#"{"permissions": 3}"#, "permissions in"),
            (
                r#"{"permissions": {"allow": "all"}}"#,
                "permissions.allow is not a list",
            ),
            (
                r#"{"permissions": {"deny": [1]}}"#,
                "permissions.deny is not a list",
            ),
        ] {
            std::fs::write(&file, text).unwrap();
            let err = allow_rules(&file, BY_HAND, SET).unwrap_err().to_string();
            assert!(
                err.contains(names) && err.contains("nothing was changed"),
                "{err}"
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
            assert!(!file.with_extension("json.bak").exists());
        }
    }

    /// A settings file that is a link is changed where it points, and the
    /// link stays a link.
    #[test]
    fn a_linked_settings_file_is_changed_where_it_points() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-settings.json");
        std::fs::write(&real, "{}").unwrap();
        let link = dir.path().join("settings.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let out = allow_rules(&link, BY_HAND, SET).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(
            json_of(&real)["permissions"]["allow"][0],
            "mcp__proton__get_status"
        );
        let real = std::fs::canonicalize(&real).unwrap();
        assert_eq!(
            out["backup"],
            json!(real.with_extension("json.bak")),
            "the copy is beside the real file"
        );
    }
}
