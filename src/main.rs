//! protonctl: a local, read-only MCP server and CLI for Proton Mail, Drive
//! and Calendar, through Proton's own clients. Design and requirements:
//! docs/rfc-0001.md. Output is JSON on stdout; logs go to stderr.

mod calendar;
mod config;
mod content;
mod convert;
mod digest;
mod drive;
mod export;
mod extract;
mod mail;
mod platform;
mod privacy;
mod secret;
mod serve;
mod tool;

use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use secrecy::{ExposeSecret as _, zeroize::Zeroizing};
use serde_json::{Value, json};
use tokio::signal::unix::{SignalKind, signal};

use calendar::{Calendars, GetEventReq, ListEventsReq, SearchEventsReq, ics::Zone};
use drive::{Drive, FileMetadataReq, ListFolderReq, ReadReq, SearchFilesReq};
use secret::Account;

const DRIVE_NOT_SET_UP: &str = "not set up; run `protonctl setup drive`";

/// Operations shared by the CLI and the MCP server.
pub(crate) struct App {
    pub calendars: Calendars,
    drive: Result<Drive, String>,
    pub drive_cli: drive::cli::Cli,
    mail: Option<mail::Mail>,
    /// `[export] folder`, checked at load: `None` when it is not set.
    export: Option<Result<export::Export, String>>,
    /// The privacy setting and key (RFC R26).
    pub privacy: privacy::Privacy,
}

impl App {
    fn load() -> Result<Self> {
        Self::from_config(config::load()?)
    }

    /// Drive is on only once `setup drive` has written `[drive]` (RFC Q26),
    /// as mail and each calendar are on only once set up.
    fn from_config(cfg: config::Config) -> Result<Self> {
        Self::from_config_beside(cfg, &drive::app_folders())
    }

    /// `from_config`, given the Proton Drive app's folders on this machine,
    /// which the export folder must stay out of even when Drive is not set
    /// up, since the app uploads whatever is written there (R11).
    fn from_config_beside(cfg: config::Config, app_folders: &[PathBuf]) -> Result<Self> {
        let zone = Zone::parse(cfg.time_zone.as_deref())?;
        let drive = match &cfg.drive {
            Some(d) => Drive::new(Some(d)).map_err(|e| format!("{e:#}")),
            None => Err(DRIVE_NOT_SET_UP.to_string()),
        };
        let root = drive.as_ref().ok().and_then(Drive::root);
        let folders = app_folders.iter().map(PathBuf::as_path).chain(root);
        let export = cfg
            .export
            .as_ref()
            .map(|e| export::Export::new(e, folders).map_err(|e| format!("{e:#}")));
        Ok(Self {
            calendars: Calendars::new(cfg.calendar, zone),
            drive,
            drive_cli: drive::cli::Cli::new(cfg.drive.as_ref()),
            mail: cfg.mail.map(mail::Mail::new),
            export,
            privacy: privacy::Privacy::stored(),
        })
    }

    /// The user's own address, for the `you` hint (RFC section 6, Hints).
    pub fn you(&self) -> Option<&str> {
        self.mail.as_ref().map(|m| m.cfg().address.as_str())
    }

    /// The mode this process started in, for `status` and `get_status`.
    fn privacy_status(&self) -> Value {
        json!({ "mode": self.privacy.setting(), "aliasFormat": privacy::FORMAT })
    }

    pub fn mail(&self) -> Result<&mail::Mail> {
        self.mail
            .as_ref()
            .context("mail is not set up; run `protonctl setup mail --address <Bridge username>`")
    }

    pub fn export(&self) -> Result<&export::Export> {
        match &self.export {
            Some(Ok(e)) => Ok(e),
            Some(Err(e)) => Err(anyhow!("the export folder is unusable: {e}")),
            None => Err(anyhow!(
                "no export folder; set [export] folder in {} to an absolute path outside the Proton Drive folder",
                config::path().display()
            )),
        }
    }

    pub fn drive(&self) -> Result<&Drive> {
        self.drive
            .as_ref()
            .map_err(|e| anyhow!("Proton Drive is unavailable: {e}"))
    }

    /// Principle 4 of the RFC: one place lists every grant protonctl holds.
    /// A secret store that cannot be read is reported in the result, with
    /// everything else, rather than failing it.
    pub fn status(&self) -> Value {
        let held = secret::accounts().map_err(|e| format!("{e:#}"));
        self.status_with(held.as_deref().map_err(String::as_str))
    }

    /// `status` given the secret store's accounts, or why they could not be
    /// read, so tests need no secret store.
    fn status_with(&self, held: Result<&[Account], &str>) -> Value {
        let store = platform::SECRET_STORE;
        let calendars: Vec<Value> = self
            .calendars
            .configs
            .iter()
            .map(|c| {
                // null when the store could not be read: not known, not "no".
                let stored = held
                    .ok()
                    .map(|h| h.contains(&Account::Calendar(c.id.clone())));
                json!({ "calendarId": c.id, "name": c.name, "linkStored": stored })
            })
            .collect();
        let secrets = match held {
            Ok(held) => json!(held.iter().map(secret::shown).collect::<Vec<_>>()),
            Err(e) => json!({ "error": e }),
        };
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "config": config::path(),
            "mail": match self.mail.as_ref().map(mail::Mail::cfg) {
                Some(m) => json!({ "address": m.address, "port": m.port, "certificatePinned": true,
                    "access": "read-only" }),
                None => json!("not set up; run `protonctl setup mail --address <Bridge username>` with the Bridge password on stdin"),
            },
            "drive": match &self.drive {
                Ok(d) => json!({
                    "folder": match d.root() {
                        Some(root) => json!(root),
                        None => json!("none; list_folder goes through the Proton Drive CLI, and search_files is off"),
                    },
                    "access": "read-only",
                    "cli": self.drive_cli.path(),
                    // Linux (Q33); a Mac checks Proton's signature instead.
                    "cliSha256": match self.drive_cli.pin() {
                        Some(Ok(pin)) => json!(pin),
                        Some(Err(e)) => json!({ "error": e }),
                        None => Value::Null,
                    },
                }),
                Err(e) => json!({ "error": e }),
            },
            "calendars": calendars,
            "downloads": content::downloads_usage(),
            "export": match &self.export {
                Some(Ok(e)) => json!({ "folder": e.folder(), "cleaned": "never; files stay until deleted" }),
                Some(Err(e)) => json!({ "error": e }),
                None => json!("not set; export tools refuse until [export] folder is in the config"),
            },
            "secretsHeld": secrets,
            "privacy": self.privacy_status(),
            "cannot": format!("{} (protonctl is read-only)", serve::CANNOT),
            "revoke": format!("`protonctl logout` deletes protonctl's {store} items; to revoke a calendar link on \
                Proton's side, delete it in the calendar's sharing settings in the Proton web app"),
        })
    }
}

#[derive(Parser)]
#[command(
    name = "protonctl",
    version,
    about = "Scoped access to Proton for Claude, through Proton's own clients"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the MCP server on stdin/stdout.
    Serve,
    /// Store a secret in the system's secret store (the Keychain, or the Secret Service on Linux) and add the service to the config.
    Setup {
        #[command(subcommand)]
        what: Setup,
    },
    /// Check every configured service; exits 1 if anything fails.
    Doctor,
    /// Show what protonctl can reach and every secret it holds.
    Status,
    /// Delete protonctl's items in the secret store, then print how to revoke Proton-side access. Asks to confirm on a terminal.
    Logout,
    /// Replace the privacy key: every alias changes, and old refs and handles stop working. Asks to confirm on a terminal.
    RotateKey,
    /// Read Proton Calendar.
    #[command(subcommand)]
    Calendar(CalendarCmd),
    /// Read Proton Drive.
    #[command(subcommand)]
    Drive(DriveCmd),
    /// Read Proton Mail.
    #[command(subcommand)]
    Mail(MailCmd),
    /// Run one document reader on stdin, in a sandbox; the server starts this.
    #[command(subcommand, hide = true)]
    Convert(convert::Job),
}

#[derive(Subcommand)]
enum MailCmd {
    /// Search with a Gmail-style query.
    Search(mail::read::SearchThreadsReq),
    /// Count what a query matches, optionally grouped by sender or recipient.
    Count(mail::read::CountMessagesReq),
    /// Show one message.
    Message(mail::read::MessageReq),
    /// Show a whole thread.
    Thread(mail::read::ThreadReq),
    /// List folders and labels.
    Labels,
    /// Save one attachment.
    Attachment {
        #[command(flatten)]
        req: mail::read::AttachmentReq,
        /// Folder to save into.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum Setup {
    /// Add a calendar. Pipe its Full-view share link on stdin, e.g.: pbpaste | protonctl setup calendar --id personal
    Calendar {
        /// Short id: a-z, 0-9 and '-'.
        #[arg(long)]
        id: String,
        /// Display name. Default: the name in the feed.
        #[arg(long)]
        name: Option<String>,
    },
    /// Choose what Claude sees: aliases for names, IDs and numbers (the default of this command), or with --off results as they are.
    Privacy {
        /// Turn the privacy layer off: results carry names, IDs and digests as they are. Asks to confirm on a terminal.
        #[arg(long)]
        off: bool,
    },
    /// Add Proton Drive: check the Proton Drive CLI's signature and sign-in, then write [drive].
    Drive {
        /// The Proton Drive app's folder. Default: found under ~/Library/CloudStorage.
        #[arg(long)]
        folder: Option<PathBuf>,
        /// The Proton Drive CLI. Default: ~/bin/proton-drive.
        #[arg(long)]
        cli: Option<PathBuf>,
    },
    /// Add Proton Mail Bridge. Pipe the Bridge password on stdin, e.g.: pbpaste | protonctl setup mail --address you@proton.me
    Mail {
        /// The username Bridge shows when you select the account.
        #[arg(long)]
        address: String,
        /// Bridge's IMAP port.
        #[arg(long, default_value_t = config::DEFAULT_IMAP_PORT)]
        port: u16,
    },
}

#[derive(Subcommand)]
enum CalendarCmd {
    /// List configured calendars.
    List,
    /// List events in a window.
    Events(ListEventsReq),
    /// Search events by text.
    Search(SearchEventsReq),
    /// Show one event.
    Event(GetEventReq),
}

#[derive(Subcommand)]
enum DriveCmd {
    /// Search by name.
    Search(SearchFilesReq),
    /// List a folder.
    Ls(ListFolderReq),
    /// Show one path's details.
    Stat(FileMetadataReq),
    /// Print a page of a file's text.
    Cat(ReadReq),
    /// Save one file through the Proton Drive CLI, even if it is only in the cloud.
    Get {
        #[command(flatten)]
        req: drive::DownloadReq,
        /// Folder to save into.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// List everything under a folder, a page at a time.
    Tree(drive::TreeReq),
    /// Write an inventory of a folder into the export folder.
    Manifest(drive::ManifestReq),
}

#[expect(
    clippy::print_stdout,
    reason = "CLI output; the MCP server never calls this"
)]
fn print(v: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// One secret from stdin, trimmed. It must be piped (`pbpaste | protonctl ...`),
/// which keeps it off the screen and out of shell history; a terminal would
/// show it as it is typed.
fn read_secret(what: &str) -> Result<secret::Secret> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        bail!(
            "pipe {what} on stdin, for example `pbpaste | protonctl setup ...`; typed at a terminal it would show on screen"
        );
    }
    let mut line = Zeroizing::new(String::new());
    stdin.lock().read_line(&mut line)?;
    Ok(secret::Secret::from(line.trim()))
}

async fn setup_calendar(id: &str, name: Option<String>) -> Result<Value> {
    config::check_calendar_id(id)?;
    let link = read_secret("the calendar's Full-view share link")?;
    calendar::check_link(link.expose_secret())?;
    let feed = calendar::ics::parse(&calendar::fetch(link.expose_secret()).await?)
        .context("the link did not return a calendar")?;
    let name = name.or(feed.name.clone()).unwrap_or_else(|| id.to_string());
    secret::set(&Account::Calendar(id.to_string()), &link)?;
    // An id already set up keeps its name; the link is replaced.
    let name = config::add_calendar(id, &name)?;
    Ok(
        json!({ "calendarId": id, "name": name, "events": feed.events.len(), "skipped": feed.skipped, "linkStored": true }),
    )
}

/// Ask on a terminal before a command that changes what every later call
/// does (RFC Q28): a plain call through Claude Code's Bash tool, which has
/// no terminal, is refused. Phase 3 asks for user presence instead.
fn confirm(what: &str) -> Result<()> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        bail!("{what}, so it asks first, on a terminal; run it in one");
    }
    eprint!("{what}. Type yes to go on: ");
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    if line.trim() != "yes" {
        bail!("not confirmed; nothing changed");
    }
    Ok(())
}

/// `setup privacy`: make the key if there is none, then set the mode, so
/// aliases mode is never set without a key. `--off` keeps the key, so the
/// aliases are the same if aliases mode comes back.
fn setup_privacy(off: bool) -> Result<Value> {
    let held = secret::comment(&Account::PrivacyKey)?.is_some();
    if off {
        confirm(
            "Turning the privacy layer off sends names, IDs and digests to the model provider as they are",
        )?;
        privacy::store_mode(privacy::Mode::Off)?;
        return Ok(
            json!({ "mode": "off", "key": if held { "kept" } else { "none" },
            "next": "restart Claude Code and Claude Desktop so their servers start in this mode" }),
        );
    }
    if held {
        // A key kept must be one this process can use; "kept" would
        // otherwise leave a broken key in place.
        privacy::key::check_stored()?;
    } else {
        privacy::key::create()?;
    }
    privacy::store_mode(privacy::Mode::Aliases)?;
    Ok(
        json!({ "mode": "aliases", "key": if held { "kept" } else { "created" },
        "next": "restart Claude Code and Claude Desktop so their servers start in this mode" }),
    )
}

fn rotate_key() -> Result<Value> {
    if secret::comment(&Account::PrivacyKey)?.is_none() {
        bail!("there is no privacy key; `protonctl setup privacy` makes one");
    }
    confirm(
        "Replacing the privacy key changes every alias, and old refs and handles stop working",
    )?;
    let id = privacy::key::create()?;
    Ok(json!({ "rotated": true, "keyId": &id[..8] }))
}

/// Check that the CLI is Proton's and signed in, find the app's folder, then
/// write `[drive]`. On Linux the user first confirms the CLI's SHA-256 on a
/// terminal, and it is pinned in the secret store, not the config, which
/// the model can edit in Claude Code (Q33, #72); a later run pins an updated
/// CLI, or with `--cli` another one. Signing in is the CLI's own step
/// (`proton-drive auth login`), so protonctl never sees the Proton password.
async fn setup_drive(folder: Option<PathBuf>, cli: Option<PathBuf>) -> Result<Value> {
    let set = config::load()?.drive;
    if set.is_some() && (cfg!(target_os = "macos") || folder.is_some()) {
        bail!(
            "Drive is already set up; edit [drive] in {} to change it",
            config::path().display()
        );
    }
    // The server resolves a path from its own working directory, so each is
    // written whole; a symlink stays one, so a CLI updated behind it is used.
    let folder = folder.map(std::path::absolute).transpose()?;
    let cli = cli.map(std::path::absolute).transpose()?;
    if let Some(set) = set {
        return repin_drive_cli(set, cli).await;
    }
    if let Some(f) = &folder
        && !f.is_dir()
    {
        bail!("--folder {} is not a folder", f.display());
    }
    let wanted = config::DriveConfig {
        folder: folder.clone(),
        cli: cli.clone(),
        ..Default::default()
    };
    let drive_cli = if cfg!(target_os = "macos") {
        drive::cli::Cli::new(Some(&wanted))
    } else {
        let pin = confirmed_pin(&drive::cli::path(Some(&wanted)), None).await?;
        drive::cli::Cli::pinned(Some(&wanted), pin)
    };
    let version = drive_cli.check().await?;
    drive_cli
        .json(&["filesystem", "list", "-j", "/my-files"].map(std::ffi::OsStr::new))
        .await
        .context(
            "the Proton Drive CLI is not signed in; run `proton-drive auth login`, then try again",
        )?;
    let root = Drive::new(Some(&wanted))?.root().map(Path::to_path_buf);
    let pin = drive_cli.pin().transpose().map_err(|e| anyhow!("{e}"))?;
    if let Some(pin) = pin {
        drive::cli::store_pin(pin)?;
    }
    config::add_drive(folder.as_deref(), cli.as_deref())?;
    Ok(json!({
        "folder": match root {
            Some(r) => json!(r),
            None => json!("none; list_folder goes through the CLI, and search_files is off"),
        },
        "cli": drive_cli.path(),
        "cliVersion": version,
        "cliSha256": pin,
        "signedIn": true,
    }))
}

/// The SHA-256 of the CLI at `path` as it is now, which the user confirms on
/// a terminal unless it is the pin already `held` (Q33). A first setup asks
/// too, so a call through Claude Code's Bash tool, which has no terminal,
/// cannot pin a file (#72). Nothing runs the CLI before this: the run that
/// follows is held to the pin it returns.
async fn confirmed_pin(path: &Path, held: Option<digest::Sha256>) -> Result<digest::Sha256> {
    let found = {
        let path = path.to_path_buf();
        content::blocking(move || digest::Sha256::of_file(&path)).await??
    };
    if held != Some(found) {
        confirm(&format!(
            "{} has SHA-256 {found}; pin it only if you installed this CLI from Proton",
            path.display()
        ))?;
    }
    Ok(found)
}

/// `setup drive` once Drive is set up, on Linux: pin the CLI as it is now,
/// at `cli` when given, after the user confirms it is a build from Proton
/// (Q33). A pin left in the config from before goes.
async fn repin_drive_cli(mut set: config::DriveConfig, cli: Option<PathBuf>) -> Result<Value> {
    if cli.is_some() {
        set.cli.clone_from(&cli);
    }
    let path = drive::cli::path(Some(&set));
    let pin = confirmed_pin(&path, drive::cli::stored_pin()?).await?;
    let version = drive::cli::Cli::pinned(Some(&set), pin).check().await?;
    drive::cli::store_pin(pin)?;
    config::set_cli(cli.as_deref())?;
    Ok(json!({
        "cli": path,
        "cliVersion": version,
        "cliSha256": pin,
        "next": "restart Claude Code and Claude Desktop so their servers use this pin",
    }))
}

/// Pin Bridge's certificate on first contact, prove the login works, then
/// store the password and write `[mail]`.
async fn setup_mail(address: &str, port: u16) -> Result<Value> {
    config::check_address(address)?;
    if config::load()?.mail.is_some() {
        bail!(
            "mail is already set up; to redo it, delete [mail] from {} first",
            config::path().display()
        );
    }
    let password = read_secret("the Bridge password Bridge shows for the account")?;
    let (tls, fingerprint) = mail::connect(port, None).await?;
    mail::login(tls, address, password.expose_secret())
        .await?
        .logout()
        .await
        .ok();
    secret::set(&Account::Bridge(address.to_string()), &password)?;
    config::add_mail(address, port, fingerprint)?;
    Ok(json!({
        "address": address,
        "port": port,
        "certificateSha256": fingerprint,
        "passwordStored": true,
    }))
}

#[expect(
    clippy::print_stdout,
    reason = "CLI output; the MCP server never calls this"
)]
async fn doctor(app: &App) -> bool {
    let mut ok = true;
    let mut check = |name: &str, result: Result<String>| match result {
        Ok(detail) => println!("ok    {name}: {detail}"),
        Err(e) => {
            ok = false;
            println!("FAIL  {name}: {e:#}");
        }
    };
    check("config", Ok(config::path().display().to_string()));
    check("privacy", app.privacy.diagnose().map(str::to_string));
    if app.privacy.started() == Some(privacy::Mode::Aliases) {
        check("name model", app.privacy.model_check());
    }
    check("document readers", convert::check().await);
    if matches!(&app.drive, Err(e) if e == DRIVE_NOT_SET_UP) {
        println!("skip  drive: not set up (protonctl setup drive)");
    } else {
        check(
            "drive folder",
            app.drive().map(|d| match d.root() {
                Some(root) => root.display().to_string(),
                None => "none; listing goes through the CLI, and search is off".into(),
            }),
        );
        check(
            "drive cli",
            app.drive_cli.check().await.map(|v| {
                let pinned = match app.drive_cli.pin() {
                    Some(Ok(pin)) => format!(", pinned SHA-256 {pin}"),
                    _ => String::new(),
                };
                format!("{} {v}{pinned}", app.drive_cli.path().display())
            }),
        );
        // Every Linux read goes through the CLI, so the folder in memory is
        // checked here; a Mac makes its RAM disk only for a cloud-only file.
        if cfg!(target_os = "linux") && app.privacy.started() == Some(privacy::Mode::Aliases) {
            let made = content::memory_folder();
            content::remove_downloads();
            check(
                "drive reads in aliases mode",
                made.map(|f| {
                    let at = f.parent().unwrap_or(&f);
                    format!("through {}, in memory and private", at.display())
                }),
            );
        }
    }
    for c in &app.calendars.configs {
        check(
            &format!("calendar {}", c.id),
            app.calendars.check(&c.id).await,
        );
    }
    match &app.mail {
        Some(m) => check("mail", mail::check(m.cfg()).await),
        None => println!("skip  mail: not set up (protonctl setup mail)"),
    }
    ok
}

/// Deletes everything under protonctl's service in the secret store, plus the configured
/// calendars' items by name. It needs neither a readable config nor a working
/// Keychain search, so revoking never waits on anything else being healthy
/// (RFC principle 4); whatever went wrong is listed under `problems`.
fn logout() -> Value {
    let mut problems = Vec::new();
    let mut accounts = secret::accounts().unwrap_or_else(|e| {
        problems.push(format!("{e:#}"));
        Vec::new()
    });
    // By name too, so a search that fails still deletes what is known.
    accounts.extend([
        Account::PrivacyKey,
        Account::PrivacyMode,
        Account::DriveCliPin,
    ]);
    match config::load() {
        Ok(cfg) => {
            accounts.extend(cfg.calendar.iter().map(|c| Account::Calendar(c.id.clone())));
            accounts.extend(cfg.mail.iter().map(|m| Account::Bridge(m.address.clone())));
        }
        Err(e) => problems.push(format!("{e:#}")),
    }
    // In the order of their names, as `deleted` lists them.
    accounts.sort_by_key(ToString::to_string);
    accounts.dedup();
    let mut deleted = Vec::new();
    for account in accounts {
        match secret::delete(&account) {
            Ok(true) => deleted.push(secret::shown(&account)),
            Ok(false) => {}
            Err(e) => {
                // Without a secret store each account fails alike; say it once.
                let problem = format!("{e:#}");
                if !problems.contains(&problem) {
                    problems.push(problem);
                }
            }
        }
    }
    json!({
        "deleted": deleted,
        "problems": problems,
        "stillToDo": [
            "Delete each calendar's share link in its sharing settings in the Proton web app; until then the link still works for anyone holding it.",
            "The Proton Drive CLI keeps its own session: run `proton-drive auth logout` to end it.",
            "Proton Mail Bridge keeps its own signed-in session: sign out in Bridge to end local IMAP access.",
        ],
    })
}

/// What aliases mode refuses of a Mail or Drive command, since it would
/// write into the export folder, or return bytes, which no CLI output
/// carries (RFC R10, M2.7).
fn refused_in_aliases_mode(cmd: &Cmd) -> Option<&'static str> {
    match cmd {
        Cmd::Drive(DriveCmd::Manifest(_)) => Some("`drive manifest`"),
        Cmd::Drive(DriveCmd::Get { req, .. }) if req.export => Some("`drive get --export`"),
        Cmd::Drive(DriveCmd::Get { req, .. }) if req.inline => Some("`drive get --inline`"),
        Cmd::Mail(MailCmd::Attachment { req, .. }) if req.export => {
            Some("`mail attachment --export`")
        }
        Cmd::Mail(MailCmd::Attachment { req, .. }) if req.inline => {
            Some("`mail attachment --inline`")
        }
        _ => None,
    }
}

/// Run a Mail or Drive command. In aliases mode it writes no content to
/// disk but where `--out` names (RFC R10): what would write elsewhere is
/// refused, and the rest runs as the server's aliases-mode calls do, so a
/// cloud-only file goes through the folder in memory (M2.8). The output is
/// not tokenized until Phase 3 (M3.3).
async fn read(app: &App, cmd: Cmd) -> Result<Value> {
    if app.privacy.started() != Some(privacy::Mode::Aliases) {
        return read_as_asked(app, cmd).await;
    }
    if let Some(what) = refused_in_aliases_mode(&cmd) {
        bail!(
            "{what} is off in aliases mode, which writes no content to disk (RFC R10); \
            `protonctl setup privacy --off` turns aliases mode off"
        );
    }
    content::restricted(content::no_mentions(), read_as_asked(app, cmd)).await
}

/// A Mail or Drive command as asked, in off mode or inside `restricted`.
async fn read_as_asked(app: &App, cmd: Cmd) -> Result<Value> {
    Ok(match cmd {
        Cmd::Mail(cmd) => {
            let m = app.mail()?;
            match cmd {
                MailCmd::Search(r) => m.search_threads(&r).await?,
                MailCmd::Count(r) => m.count_messages(&r).await?,
                MailCmd::Message(r) => m.get_message(&r).await?,
                MailCmd::Thread(r) => m.get_thread(&r).await?,
                MailCmd::Labels => m.list_labels().await?,
                MailCmd::Attachment { req, out } => {
                    m.get_attachment(&req, Some(&out), app.export()).await?.json
                }
            }
        }
        Cmd::Drive(cmd) => {
            let d = app.drive()?;
            match cmd {
                DriveCmd::Search(r) => d.search_files(&r).await?,
                DriveCmd::Ls(r) => d.list_folder(&r, &app.drive_cli).await?,
                DriveCmd::Stat(r) => d.get_file_metadata(&r, &app.drive_cli).await?,
                DriveCmd::Cat(r) => d.read_file_content(&r, &app.drive_cli).await?.json,
                DriveCmd::Get { req, out } => {
                    d.download_file(&req, &app.drive_cli, Some(&out), app.export())
                        .await?
                        .json
                }
                DriveCmd::Tree(r) => d.list_tree(&r, &app.drive_cli).await?,
                DriveCmd::Manifest(r) => {
                    d.export_manifest(&r, &app.drive_cli, app.export()?).await?
                }
            }
        }
        _ => unreachable!("only Mail and Drive commands are read here"),
    })
}

/// Serve until the host closes stdin or a signal arrives. Hosts stop a server
/// by closing stdin, then with SIGTERM if it lingers; closing a terminal sends
/// SIGHUP, and a person presses Ctrl-C.
async fn serve_until_stopped(app: Arc<App>) -> Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut hup = signal(SignalKind::hangup())?;
    tokio::select! {
        served = serve::run(app) => served,
        _ = term.recv() => Ok(()),
        _ = hup.recv() => Ok(()),
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

/// What a panic prints (RFC R25, M2.9): where it happened, and nothing of
/// its message, which can quote data (a slice off a character boundary
/// prints up to 256 bytes of the string), since hosts keep stderr in logs.
fn panic_line(info: &std::panic::PanicHookInfo<'_>) -> String {
    let at = info
        .location()
        .map_or_else(String::new, |l| format!(" at {}:{}", l.file(), l.line()));
    format!("protonctl: internal error{at}; its message is withheld, as it can quote data")
}

fn main() -> Result<()> {
    std::panic::set_hook(Box::new(|info| eprintln!("{}", panic_line(info))));
    // Not from RUST_LOG: at debug, rmcp logs every request and tool result,
    // and hosts keep a server's stderr in log files (RFC R10).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing_subscriber::filter::LevelFilter::WARN)
        .init();
    let cli = Cli::parse();
    // Before anything else is read: a reader's sandbox holds no config,
    // secret or session (R21).
    if let Cmd::Convert(job) = cli.command {
        if let Err(e) = convert::run(job) {
            eprintln!("protonctl convert: {e:#}");
            std::process::exit(convert::SANDBOX_FAILED);
        }
        std::process::exit(0);
    }
    if matches!(cli.command, Cmd::Logout) {
        confirm("Logging out deletes every secret protonctl holds, the privacy key among them")?;
        let out = logout();
        print(&out)?;
        let clean = out["problems"].as_array().is_some_and(Vec::is_empty);
        std::process::exit(i32::from(!clean));
    }
    let app = Arc::new(App::load()?);
    let rt = tokio::runtime::Runtime::new()?;
    if matches!(cli.command, Cmd::Serve) {
        let served = rt.block_on(serve_until_stopped(app));
        // Drop every task still running, so a Drive download in flight ends its
        // CLI (kill_on_drop) before the download folder goes (RFC R10). tokio's
        // stdin reader can be blocked in a read nothing cancels, so wait at most
        // a second for it, then exit rather than return.
        rt.shutdown_timeout(Duration::from_secs(1));
        content::remove_downloads();
        #[expect(
            clippy::use_debug,
            reason = "main's own format for every other command's error"
        )]
        if let Err(e) = &served {
            eprintln!("Error: {e:?}");
        }
        std::process::exit(i32::from(served.is_err()));
    }
    let out: Result<Value> = rt.block_on(async {
        Ok(match cli.command {
            Cmd::Serve => unreachable!("served above"),
            Cmd::Setup { what } => match what {
                Setup::Calendar { id, name } => setup_calendar(&id, name).await?,
                Setup::Drive { folder, cli } => setup_drive(folder, cli).await?,
                Setup::Privacy { off } => setup_privacy(off)?,
                Setup::Mail { address, port } => setup_mail(&address, port).await?,
            },
            Cmd::Doctor => std::process::exit(i32::from(!doctor(&app).await)),
            Cmd::Status => app.status(),
            Cmd::Logout | Cmd::Convert(_) => {
                unreachable!("handled before the config is loaded")
            }
            Cmd::RotateKey => rotate_key()?,
            Cmd::Calendar(cmd) => match cmd {
                CalendarCmd::List => app.calendars.list_calendars().await?,
                CalendarCmd::Events(r) => app.calendars.list_events(&r).await?,
                CalendarCmd::Search(r) => app.calendars.search_events(&r).await?,
                CalendarCmd::Event(r) => app.calendars.get_event(&r).await?,
            },
            cmd @ (Cmd::Mail(_) | Cmd::Drive(_)) => read(&app, cmd).await?,
        })
    });
    // A cloud-only file read by `drive cat` was fetched into the download
    // folder, or in aliases mode the folder in memory.
    content::remove_downloads();
    print(&out?)
}

/// An app with nothing set up, for tests.
#[cfg(test)]
impl App {
    pub fn bare(privacy: privacy::Privacy) -> Self {
        App {
            calendars: Calendars::new(Vec::new(), Zone::Local),
            drive: Err("not set up".into()),
            drive_cli: drive::cli::Cli::new(Some(&config::DriveConfig {
                cli: Some("/nonexistent".into()),
                ..Default::default()
            })),
            mail: None,
            export: None,
            privacy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_queries_may_start_with_a_hyphen() {
        // Proton IDs are base64url, and a Gmail-style query can open with a negation.
        for line in [
            "mail thread -Pabc=@proton.me",
            "mail message -abc==",
            "mail attachment -abc== 0 --out /tmp",
            "mail search -has:attachment --page-size 5",
            "calendar event -uid@x.test",
            "calendar search -note",
            "drive search -draft",
        ] {
            let argv = std::iter::once("protonctl").chain(line.split(' '));
            assert!(Cli::try_parse_from(argv).is_ok(), "{line}");
        }
        let help = Cli::try_parse_from(["protonctl", "mail", "thread", "-h"]);
        assert_eq!(
            help.err().map(|e| e.kind()),
            Some(clap::error::ErrorKind::DisplayHelp)
        );
    }

    /// RFC M1.4: Drive is off without a `[drive]` table and on with one.
    #[test]
    fn drive_is_on_only_once_set_up() {
        let off = App::from_config(config::Config::default()).unwrap();
        assert_eq!(
            off.drive.as_ref().err().map(String::as_str),
            Some(DRIVE_NOT_SET_UP)
        );
        assert_eq!(off.status_with(Ok(&[]))["drive"]["error"], DRIVE_NOT_SET_UP);
        let dir = tempfile::tempdir().unwrap();
        let cfg = config::Config {
            drive: Some(config::DriveConfig {
                folder: Some(dir.path().to_path_buf()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut on = App::from_config(cfg).unwrap();
        let root = on.drive().unwrap().root().map(Path::to_path_buf);
        assert_eq!(root, Some(dir.path().canonicalize().unwrap()));
        // Q33, #72: `status` shows the pin on Linux, which is not in the
        // config, or why there is none; a Mac checks the signature.
        let unread = &on.status_with(Ok(&[]))["drive"]["cliSha256"];
        let pin = digest::Sha256::of(b"cli");
        on.drive_cli = drive::cli::Cli::pinned(None, pin);
        let pinned = &on.status_with(Ok(&[]))["drive"]["cliSha256"];
        if cfg!(target_os = "macos") {
            assert_eq!((unread, pinned), (&Value::Null, &Value::Null));
        } else {
            let why = unread["error"].as_str().unwrap_or_default();
            assert!(why.contains("drive-cli-pin"), "{unread}");
            assert_eq!(pinned, &json!(pin.to_string()));
        }
    }

    /// RFC R11: an export folder inside the Proton Drive app's folder is
    /// refused even before `setup drive`, since the app uploads it.
    #[test]
    fn exports_stay_out_of_the_drive_app_folder_without_drive_set_up() {
        let app_folder = tempfile::tempdir().unwrap();
        let app_folder = app_folder.path().canonicalize().unwrap();
        let cfg = |folder: PathBuf| config::Config {
            export: Some(config::ExportConfig { folder }),
            ..Default::default()
        };
        let inside = App::from_config_beside(
            cfg(app_folder.join("Exports")),
            std::slice::from_ref(&app_folder),
        );
        assert!(inside.unwrap().export.unwrap().is_err());
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = App::from_config_beside(cfg(elsewhere.path().join("x")), &[app_folder]);
        assert!(outside.unwrap().export.unwrap().is_ok());
    }

    #[test]
    fn status_counts_this_process_s_downloads() {
        let app = App::bare(privacy::tests::privacy(None, None).0);
        let dir = tempfile::Builder::new()
            .prefix("mail-")
            .tempdir_in(content::downloads().unwrap())
            .unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        // Other tests download into the same folder, so at least this file.
        let shown = &app.status_with(Ok(&[]))["downloads"];
        let count = |k: &str| shown[k].as_u64().unwrap_or(0);
        assert!(count("files") >= 1 && count("bytes") >= 5, "{shown}");
    }

    #[test]
    fn a_panic_prints_where_and_withholds_what() {
        let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = lines.clone();
        let previous = std::panic::take_hook();
        // Other tests' panics can land here too while it is set.
        std::panic::set_hook(Box::new(move |info| {
            seen.lock().unwrap().push(panic_line(info));
        }));
        let caught = std::panic::catch_unwind(|| panic!("secret-7Q in the message"));
        std::panic::set_hook(previous);
        assert!(caught.is_err());
        let lines = lines.lock().unwrap();
        assert!(lines.iter().any(|l| l.contains("main.rs:")), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("secret-7Q")), "{lines:?}");
    }

    /// RFC R1 where the model reads it: the server's instructions and
    /// `get_status` say what protonctl cannot do, in the same words.
    #[test]
    fn the_model_reads_that_protonctl_is_read_only() {
        let app = Arc::new(App::bare(privacy::tests::privacy(None, None).0));
        let cannot = app.status_with(Ok(&[]))["cannot"].clone();
        let aliases = privacy::tests::privacy(Some(privacy::Mode::Aliases), Some([7; 32])).0;
        for app in [app, Arc::new(App::bare(aliases))] {
            let info = rmcp::ServerHandler::get_info(&serve::Server::new(app));
            let instructions = info.instructions.unwrap();
            assert!(
                instructions.contains(&format!("read-only: it cannot {}.", serve::CANNOT)),
                "{instructions}"
            );
        }
        assert_eq!(
            cannot,
            format!("{} (protonctl is read-only)", serve::CANNOT)
        );
    }

    /// RFC R26: `status` (as `get_status`), `doctor` and the server's
    /// instructions name the mode the process started in, or say that
    /// there is none, or that it cannot be read.
    #[test]
    fn status_doctor_and_the_instructions_name_the_mode() {
        use privacy::key::tests::FakeKey;
        use privacy::tests::FakeMode;
        use privacy::{Mode, Privacy};
        let with = |mode: Result<Option<Mode>, ()>| {
            Privacy::new(
                Box::new(Arc::new(FakeMode(std::sync::Mutex::new(mode)))),
                Box::new(Arc::new(FakeKey(std::sync::Mutex::new(Some([7; 32]))))),
            )
        };
        let cases = [
            (Ok(Some(Mode::Off)), "off", "The privacy setting is off"),
            (
                Ok(Some(Mode::Aliases)),
                "aliases",
                "The privacy setting is on",
            ),
            (Ok(None), "unset", "No privacy mode is set"),
            (Err(()), "unreadable", "privacy setting cannot be read"),
        ];
        for (mode, word, says) in cases {
            let app = Arc::new(App::bare(with(mode)));
            assert_eq!(app.status_with(Ok(&[]))["privacy"]["mode"], word);
            let doctor = app.privacy.diagnose().map_err(|e| format!("{e:#}"));
            match word {
                "off" | "aliases" => assert_eq!(doctor, Ok(word)),
                _ => assert!(
                    doctor.as_ref().is_err_and(|e| e.contains(says)),
                    "{doctor:?}"
                ),
            }
            let info = rmcp::ServerHandler::get_info(&serve::Server::new(app));
            let instructions = info.instructions.unwrap();
            assert!(instructions.contains(says), "{word}: {instructions}");
        }
    }

    #[test]
    fn status_says_when_the_secret_store_cannot_be_read() {
        let calendar = config::CalendarConfig {
            id: "personal".into(),
            name: "Personal".into(),
        };
        let app = App {
            calendars: Calendars::new(vec![calendar], Zone::Local),
            ..App::bare(privacy::tests::privacy(None, None).0)
        };
        let held = [Account::Calendar("personal".into())];
        let read = app.status_with(Ok(&held));
        assert_eq!(read["calendars"][0]["linkStored"], true);
        let store = platform::SECRET_STORE;
        assert_eq!(
            read["secretsHeld"],
            json!([format!("{store} protonctl/calendar/personal")])
        );
        let unread = app.status_with(Err("the store is locked"));
        assert_eq!(unread["calendars"][0]["linkStored"], Value::Null);
        assert_eq!(
            unread["secretsHeld"],
            json!({ "error": "the store is locked" })
        );
    }
}
