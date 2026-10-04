//! protonctl: a local MCP server and CLI that gives Claude scoped access to
//! Proton through Proton's own clients. Design and requirements:
//! docs/rfc-0001.md. Output is JSON on stdout; logs go to stderr.

mod calendar;
mod config;
mod content;
mod digest;
mod drive;
mod export;
mod mail;
mod secret;
mod serve;

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use secrecy::{ExposeSecret as _, zeroize::Zeroizing};
use serde_json::{Value, json};
use tokio::signal::unix::{SignalKind, signal};

use calendar::{Calendars, GetEventReq, ListEventsReq, SearchEventsReq, ics::Zone};
use drive::{Drive, FileMetadataReq, ListFolderReq, PathReq, SearchFilesReq};

/// Operations shared by the CLI and the MCP server.
pub(crate) struct App {
    pub calendars: Calendars,
    drive: Result<Drive, String>,
    pub drive_cli: drive::cli::Cli,
    mail: Option<mail::Mail>,
    /// `[export] folder`, checked at load: `None` when it is not set.
    export: Option<Result<export::Export, String>>,
}

impl App {
    fn load() -> Result<Self> {
        let cfg = config::load()?;
        let zone = Zone::parse(cfg.time_zone.as_deref())?;
        let drive = Drive::new(cfg.drive.as_ref()).map_err(|e| format!("{e:#}"));
        let root = drive.as_ref().ok().and_then(Drive::root);
        let export = cfg
            .export
            .as_ref()
            .map(|e| export::Export::new(e, root).map_err(|e| format!("{e:#}")));
        Ok(Self {
            calendars: Calendars::new(cfg.calendar, zone),
            drive,
            drive_cli: drive::cli::Cli::new(drive::cli::path(cfg.drive.as_ref())),
            mail: cfg.mail.map(mail::Mail::new),
            export,
        })
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
    pub fn status(&self) -> Result<Value> {
        Ok(self.status_with(&secret::accounts()?))
    }

    /// `status` given the Keychain accounts held, so tests need no Keychain.
    fn status_with(&self, held: &[String]) -> Value {
        let calendars: Vec<Value> = self
            .calendars
            .configs
            .iter()
            .map(|c| {
                let stored = held.contains(&secret::calendar_account(&c.id));
                json!({ "calendarId": c.id, "name": c.name, "linkStored": stored })
            })
            .collect();
        let secrets: Vec<String> = held
            .iter()
            .map(|a| format!("Keychain protonctl/{a}"))
            .collect();
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "config": config::path(),
            "mail": match self.mail.as_ref().map(mail::Mail::cfg) {
                Some(m) => json!({ "address": m.address, "port": m.port, "certificatePinned": true,
                    "access": "read-only in this version" }),
                None => json!("not set up; run `protonctl setup mail --address <Bridge username>` with the Bridge password on stdin"),
            },
            "drive": match &self.drive {
                Ok(d) => json!({
                    "folder": match d.root() {
                        Some(root) => json!(root),
                        None => json!("none; list_folder goes through the Proton Drive CLI, and search_files is off"),
                    },
                    "access": "read-only in this version",
                    "cli": self.drive_cli.path(),
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
            "cannot": "send email, share files, create links or invitations, or delete anything permanently",
            "revoke": "`protonctl logout` deletes protonctl's Keychain items; to revoke a calendar link on Proton's \
                side, delete it in the calendar's sharing settings in the Proton web app",
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
    /// Store a secret in the Keychain and add the service to the config.
    Setup {
        #[command(subcommand)]
        what: Setup,
    },
    /// Check every configured service; exits 1 if anything fails.
    Doctor,
    /// Show what protonctl can reach and every secret it holds.
    Status,
    /// Delete protonctl's Keychain items, then print how to revoke Proton-side access.
    Logout,
    /// Read Proton Calendar.
    #[command(subcommand)]
    Calendar(CalendarCmd),
    /// Read Proton Drive.
    #[command(subcommand)]
    Drive(DriveCmd),
    /// Read Proton Mail.
    #[command(subcommand)]
    Mail(MailCmd),
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
    /// Print a text file's content.
    Cat(PathReq),
    /// Save one file through the Proton Drive CLI, even if it is only in the cloud.
    Get {
        #[command(flatten)]
        req: drive::DownloadReq,
        /// Folder to save into.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
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
    secret::set(&secret::calendar_account(id), &link)?;
    config::add_calendar(id, &name)?;
    Ok(
        json!({ "calendarId": id, "name": name, "events": feed.events.len(), "skipped": feed.skipped, "linkStored": true }),
    )
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
    secret::set(&secret::bridge_account(address), &password)?;
    config::add_mail(address, port, &hex::encode(fingerprint))?;
    Ok(json!({
        "address": address,
        "port": port,
        "certificateSha256": hex::encode(fingerprint),
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
    check(
        "drive folder",
        app.drive().map(|d| match d.root() {
            Some(root) => root.display().to_string(),
            None => "none; listing goes through the CLI, and search is off".into(),
        }),
    );
    check(
        "drive cli",
        drive::cli::verify(app.drive_cli.path())
            .map(|v| format!("{} {v}", app.drive_cli.path().display())),
    );
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

/// Deletes everything under protonctl's Keychain service, plus the configured
/// calendars' items by name. It needs neither a readable config nor a working
/// Keychain search, so revoking never waits on anything else being healthy
/// (RFC principle 4); whatever went wrong is listed under `problems`.
fn logout() -> Value {
    let mut problems = Vec::new();
    let mut accounts = secret::accounts().unwrap_or_else(|e| {
        problems.push(format!("{e:#}"));
        Vec::new()
    });
    match config::load() {
        Ok(cfg) => {
            accounts.extend(cfg.calendar.iter().map(|c| secret::calendar_account(&c.id)));
            accounts.extend(cfg.mail.iter().map(|m| secret::bridge_account(&m.address)));
        }
        Err(e) => problems.push(format!("{e:#}")),
    }
    accounts.sort();
    accounts.dedup();
    let mut deleted = Vec::new();
    for account in accounts {
        match secret::delete(&account) {
            Ok(true) => deleted.push(format!("Keychain protonctl/{account}")),
            Ok(false) => {}
            Err(e) => problems.push(format!("{e:#}")),
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

fn main() -> Result<()> {
    // Not from RUST_LOG: at debug, rmcp logs every request and tool result,
    // and hosts keep a server's stderr in log files (RFC R10).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing_subscriber::filter::LevelFilter::WARN)
        .init();
    let cli = Cli::parse();
    if matches!(cli.command, Cmd::Logout) {
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
                Setup::Mail { address, port } => setup_mail(&address, port).await?,
            },
            Cmd::Doctor => std::process::exit(i32::from(!doctor(&app).await)),
            Cmd::Status => app.status()?,
            Cmd::Logout => unreachable!("handled before the config is loaded"),
            Cmd::Calendar(cmd) => match cmd {
                CalendarCmd::List => app.calendars.list_calendars().await?,
                CalendarCmd::Events(r) => app.calendars.list_events(&r).await?,
                CalendarCmd::Search(r) => app.calendars.search_events(&r).await?,
                CalendarCmd::Event(r) => app.calendars.get_event(&r).await?,
            },
            Cmd::Mail(cmd) => {
                let m = app.mail()?;
                match cmd {
                    MailCmd::Search(r) => m.search_threads(&r).await?,
                    MailCmd::Count(r) => m.count_messages(&r).await?,
                    MailCmd::Message(r) => m.get_message(&r).await?,
                    MailCmd::Thread(r) => m.get_thread(&r).await?,
                    MailCmd::Labels => m.list_labels().await?,
                    MailCmd::Attachment { req, out } => {
                        m.get_attachment(&req, Some(&out), app.export()).await?
                    }
                }
            }
            Cmd::Drive(cmd) => {
                let d = app.drive()?;
                match cmd {
                    DriveCmd::Search(r) => d.search_files(&r).await?,
                    DriveCmd::Ls(r) => d.list_folder(&r, &app.drive_cli).await?,
                    DriveCmd::Stat(r) => d.get_file_metadata(&r, &app.drive_cli).await?,
                    DriveCmd::Cat(r) => d.read_file_content(&r, &app.drive_cli).await?,
                    DriveCmd::Get { req, out } => {
                        d.download_file(&req, &app.drive_cli, Some(&out), app.export())
                            .await?
                    }
                    DriveCmd::Manifest(r) => {
                        d.export_manifest(&r, &app.drive_cli, app.export()?).await?
                    }
                }
            }
        })
    });
    // A cloud-only file read by `drive cat` was fetched into the download folder.
    content::remove_downloads();
    print(&out?)
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

    #[test]
    fn status_counts_this_process_s_downloads() {
        let app = App {
            calendars: Calendars::new(Vec::new(), Zone::Local),
            drive: Err("not set up".into()),
            drive_cli: drive::cli::Cli::new(PathBuf::from("/nonexistent")),
            mail: None,
            export: None,
        };
        let dir = tempfile::Builder::new()
            .prefix("mail-")
            .tempdir_in(content::downloads().unwrap())
            .unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        // Other tests download into the same folder, so at least this file.
        let shown = &app.status_with(&[])["downloads"];
        let count = |k: &str| shown[k].as_u64().unwrap_or(0);
        assert!(count("files") >= 1 && count("bytes") >= 5, "{shown}");
    }
}
