//! Every aliases-mode tool, called through the server (docs/todo.md T1, T2,
//! T7, T8 and T9). One harness plants a corpus in everything the tools
//! read: a scripted Bridge, a calendar feed, the Proton Drive app's folder
//! and a stand-in Proton Drive CLI. It calls each tool the aliases-mode
//! server registers, with the calls that fail among them, since errors
//! leave through the same path; each test then checks every result.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization as _;

use super::*;
use crate::calendar::tests::{event, feed, one_calendar};
use crate::drive::tests::{answer, cli_drive};
use crate::mail::read::GroupBy;
use crate::mail::read::tests::{FakeBox, Stored, fake_imap};
use crate::privacy::canon;
use crate::privacy::ident::EntityType;
use crate::privacy::tests::privacy;

/// The privacy key both servers hold.
const KEY: [u8; 32] = [7; 32];

// People. Each name is in a header or an invitation, so the process
// dictionary (RFC Q22) holds it.
const ZOLTAN: &str = "Zoltan Kwiatkowski";
const ZOLTAN_EMAIL: &str = "z.kwiatkowski@kwiatkowski-legal.example";
const OLENA: &str = "Олена Коваленко";
const OLENA_EMAIL: &str = "olena.kovalenko@kovalenko-studio.example";
const NIKOS: &str = "Νίκος Παπαδόπουλος";
const NIKOS_EMAIL: &str = "nikos.papadopoulos@papadopoulos-shipping.example";
const WANG: &str = "王小明";
const WANG_EMAIL: &str = "wang.xiaoming@wang-trading.example";
/// In the calendar only: mail and Drive find her through the dictionary.
const INGRID: &str = "Ingrid Haugland";
const INGRID_EMAIL: &str = "ingrid.haugland@haugland-arkitekter.example";
/// The user.
const TOBIAS: &str = "Tobias Wennerholm";
const TOBIAS_EMAIL: &str = "t.wennerholm@wennerholm-home.example";
const PEOPLE: [(&str, &str); 6] = [
    (ZOLTAN, ZOLTAN_EMAIL),
    (OLENA, OLENA_EMAIL),
    (NIKOS, NIKOS_EMAIL),
    (WANG, WANG_EMAIL),
    (INGRID, INGRID_EMAIL),
    (TOBIAS, TOBIAS_EMAIL),
];

/// Phone numbers from four countries, each one the phone detector takes
/// as assigned.
const PHONES: [&str; 4] = [
    "+44 20 7946 0958",
    "+49 30 12345678",
    "+1 415 555 0132",
    "+33 1 42 68 53 00",
];
/// Card numbers that pass the Luhn check.
const CARDS: [&str; 2] = ["4111 1111 1111 1111", "5555 5555 5555 4444"];
/// IBANs that pass the mod-97 check.
const IBANS: [&str; 2] = ["DE89 3704 0044 0532 0130 00", "GB82 WEST 1234 5698 7654 32"];
const URLS: [&str; 2] = [
    "https://kwiatkowski-legal.example/cases/8841-zk?ref=olena",
    "https://haugland-arkitekter.example/projects/77-kw",
];
/// The SHA-1 the stand-in CLI claims for every file.
const CLAIMED_SHA1: &str = "9b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a3b5c";
/// Message-Id values: the roots that name the two threads.
const ROOT_ID: &str = "CAKw7zq3Qn8vR.lease@kwiatkowski-legal.example";
const DRAWINGS_ID: &str = "Pz4Lm8Rt2Wq6Ks.drawings@papadopoulos-shipping.example";
/// The first 16 characters of each message's `X-Pm-Internal-Id`, which an
/// off-mode messageId shows.
const MESSAGE_HEADS: [&str; 6] = [
    "Kq7Zt2Lm9Xw4Rb8N",
    "Vd6Hf1Jg5Sp0TyQe",
    "Lx3Mc8Nb2Vq7Wr4T",
    "Gh5Jk9Lp2Zx6Cv1B",
    "Qw8Er3Ty7Ui2Op5A",
    "Sd4Fg8Hj1Kl6Zx3C",
];
/// The messages' IMAP UIDs in All Mail, which a mail page token carries
/// until it is sealed.
const UIDS: [u32; 6] = [
    7_319_421, 7_319_422, 7_319_423, 7_319_424, 7_319_425, 7_319_426,
];
/// The replies' Message-Ids.
fn reply_id(n: usize) -> String {
    format!("Ry5Tm2Qk8Wb3Ze{n}.reply@kovalenko-studio.example")
}
/// iCalendar UIDs.
const VISIT_UID: &str = "Pq7Zr2Kx9WmL4tYv.visit@proton.me";
const REVIEW_UID: &str = "Wn5Bt8Qc3Rj6Lz1D.review@proton.me";

/// Planted values the Phase 2 detectors are documented not to catch, so
/// the leak test leaves them out; a test checks that they still show, so
/// a gap that closes moves its value into the corpus.
const GAPS: [(&str, &str); 2] = [
    // A name only in free text, in no header or invitation: RFC section 6,
    // Pipeline step 2, and security-privacy-review.md ("A name only free
    // text carries stays plaintext until GLiNER", Phase 5).
    ("Bartholomew Quist", "a name only in free text"),
    // RFC section 6, Pipeline step 3: "Street addresses have no Phase 2
    // detector."
    ("Storgata 17", "a street address"),
];

// Drive paths. The app's folder holds all but `CLOUD` and `CLOUD_CONTRACT`,
// which only the CLI lists.
const LEASE_FOLDER: &str = "/Clients/Zoltan Kwiatkowski";
const LEASE: &str = "/Clients/Zoltan Kwiatkowski/Lease – Zoltan Kwiatkowski.txt";
const NOTES: &str = "/Clients/Олена Коваленко/notes for t.wennerholm@wennerholm-home.example.txt";
const PLAN_PDF: &str = "/Clients/Νίκος Παπαδόπουλος.pdf";
const SCAN: &str = "/Clients/scan from Ingrid Haugland.pdf";
const INVOICE: &str = "/Clients/王小明 invoice.txt";
const PHOTO: &str = "/Photos/Ingrid Haugland site.png";
const CLOUD: &str = "/Ingrid Haugland – site plan.txt";
const CLOUD_CONTRACT: &str = "/Clients/z.kwiatkowski@kwiatkowski-legal.example contract.txt";

/// A 1x1 PNG.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\nIDATx\x9cc\0\x01\0\0\x05\0\x01\r\n-\xb4\0\0\0\0IEND\xaeB`\x82";
/// Bytes that are neither text nor a document nor an image.
const BINARY: &[u8] = b"\0\x01\x02\x03LEDGER\0\xff\xfe\x00\x10\x20\x30\x40\x50";

/// Lines of ordinary text no detector claims, to make bodies long.
fn filler() -> String {
    "The draft follows the usual form and nothing in it has changed.\r\n".repeat(140)
}

/// One-page PDF whose text layer holds `lines`, or none when there are
/// none: a scan, as far as a reader can tell.
fn pdf(lines: &[&str]) -> Vec<u8> {
    let mut text = String::new();
    if !lines.is_empty() {
        text.push_str("BT /F1 12 Tf 72 720 Td 16 TL");
        for l in lines {
            write!(text, " ({l}) '").unwrap();
        }
        text.push_str(" ET");
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn lease_text() -> String {
    format!(
        "Lease between {ZOLTAN} and {TOBIAS}.\nWitness: {OLENA} ({OLENA_EMAIL}), {}.\n\
         Rent goes to {}; the deposit by card {}.\nThe case file: {}\nCopied to {}.\n",
        PHONES[1], IBANS[1], CARDS[0], URLS[0], GAPS[0].0
    )
}

fn notes_text() -> String {
    format!(
        "{NIKOS} called from {} about {}; {WANG} and {INGRID} were told.\n",
        PHONES[2], IBANS[0]
    )
}

fn invoice_text() -> String {
    format!("Invoice for {WANG} ({WANG_EMAIL}): {}.\n", PHONES[3])
}

fn cloud_text() -> String {
    format!(
        "Site plan from {INGRID} for {ZOLTAN}: {INGRID_EMAIL}, {}, {}\n",
        PHONES[0], URLS[1]
    )
}

fn plan_pdf() -> Vec<u8> {
    let call = format!("Call {INGRID} on {}", PHONES[0]);
    let card = format!("Card {}", CARDS[1]);
    pdf(&[&format!("Plan for {ZOLTAN}"), &call, &card])
}

fn attachment_text() -> String {
    format!(
        "Notes for {ZOLTAN}: pay {} or card {}; ring {}; mail {INGRID_EMAIL}.\r\n",
        IBANS[1], CARDS[1], PHONES[2]
    )
}

/// An `X-Pm-Internal-Id` as Bridge shows one: 88 characters of base64.
fn proton_id(head: &str) -> String {
    let tail: String = head.chars().rev().cycle().take(70).collect();
    format!("{head}{tail}==")
}

/// A Proton Drive node UID, `volume~node`, as the CLI prints one.
fn node_uid(n: u8) -> String {
    format!("Vq3Kd8Zp1Lx7Wm2Ty5Rb{n:02}~Hc4Jf6Gs0Pa8Ue3Oi7Xk{n:02}")
}

fn revision_uid(n: u8) -> String {
    format!("Rv7Nq2Lp9Kx4Mz8Bw3Ct{n:02}~Yu6Ti1Re5Wq9Pl2Ok7J{n:02}")
}

/// Header lines, each ended as RFC 5322 ends them.
fn header(lines: &[&str]) -> String {
    lines.iter().fold(String::new(), |mut out, l| {
        write!(out, "{l}\r\n").unwrap();
        out
    })
}

fn stored(i: usize, date: &'static str, head: &str, body: &str) -> Stored {
    let raw = format!(
        "X-Pm-Internal-Id: {}\r\n{head}\r\n{body}",
        proton_id(MESSAGE_HEADS[i])
    );
    Stored {
        flags: "\\Seen",
        date,
        raw,
    }
}

/// Five messages of one thread, long enough that it comes in two pages,
/// and a message with four attachments: text, a PDF, an image and bytes.
fn messages() -> Vec<Stored> {
    let reply = |n: usize, from: &str| {
        header(&[
            &format!("Message-Id: <{}>", reply_id(n)),
            &format!("In-Reply-To: <{ROOT_ID}>"),
            &format!("References: <{ROOT_ID}>"),
            &format!("From: {from}"),
            &format!("To: {ZOLTAN} <{ZOLTAN_EMAIL}>"),
            &format!("Subject: Re: Lease for {ZOLTAN} and {OLENA}"),
            "Content-Type: text/plain; charset=utf-8",
        ])
    };
    let root = header(&[
        &format!("Message-Id: <{ROOT_ID}>"),
        &format!("From: {ZOLTAN} <{ZOLTAN_EMAIL}>"),
        &format!("To: {TOBIAS} <{TOBIAS_EMAIL}>"),
        &format!("Cc: {OLENA} <{OLENA_EMAIL}>"),
        &format!("Subject: Lease for {ZOLTAN} and {OLENA}"),
        &format!(
            "Authentication-Results: mx.proton.test; dkim=pass header.d=kwiatkowski-legal.example; spf=pass smtp.mailfrom={ZOLTAN_EMAIL}"
        ),
        "X-Pm-Origin: external",
        "Content-Type: text/plain; charset=utf-8",
    ]);
    let root_body = format!(
        "Dear {TOBIAS},\r\n{OLENA} signs for {ZOLTAN} on Friday. Call {} or write to {OLENA_EMAIL}.\r\n\
         The deposit goes to {}, or by card {}.\r\nThe file is at {}\r\n{}",
        PHONES[0],
        IBANS[0],
        CARDS[0],
        URLS[0],
        filler()
    );
    let olena = format!("{OLENA} <{OLENA_EMAIL}>");
    let tobias = format!("{TOBIAS} <{TOBIAS_EMAIL}>");
    let zoltan = format!("{ZOLTAN} <{ZOLTAN_EMAIL}>");
    let answer = format!(
        "{ZOLTAN}, I can sign on Friday. Ring {}.\r\n{}",
        PHONES[1],
        filler()
    );
    let drawings = header(&[
        &format!("Message-Id: <{DRAWINGS_ID}>"),
        &format!("From: {NIKOS} <{NIKOS_EMAIL}>"),
        &format!("To: {TOBIAS} <{TOBIAS_EMAIL}>"),
        &format!("Cc: {WANG} <{WANG_EMAIL}>"),
        &format!("Subject: Drawings for {ZOLTAN} from {NIKOS}"),
        "X-Attached: notes.txt",
        "X-Attached: plan.pdf",
        "X-Attached: site.png",
        "X-Attached: ledger.bin",
        "Content-Type: multipart/mixed; boundary=\"b\"",
    ]);
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let attached = format!(
        "--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{NIKOS} here; {WANG} has the originals. Call {}.\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=\"{ZOLTAN} notes.txt\"\r\n\r\n{}\
         --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"{INGRID} plan.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n\
         --b\r\nContent-Type: image/png\r\nContent-Disposition: attachment; filename=\"site.png\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n\
         --b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"ledger.bin\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--b--\r\n",
        PHONES[3],
        attachment_text(),
        b64(&plan_pdf()),
        b64(PNG),
        b64(BINARY),
    );
    vec![
        stored(0, "01-Oct-2026 09:00:00 +0000", &root, &root_body),
        stored(1, "01-Oct-2026 12:00:00 +0000", &reply(1, &olena), &answer),
        stored(
            2,
            "02-Oct-2026 08:00:00 +0000",
            &reply(2, &tobias),
            &filler(),
        ),
        stored(
            3,
            "02-Oct-2026 15:00:00 +0000",
            &reply(3, &zoltan),
            &filler(),
        ),
        stored(
            4,
            "03-Oct-2026 10:00:00 +0000",
            &reply(4, &olena),
            &filler(),
        ),
        stored(5, "04-Oct-2026 09:30:00 +0000", &drawings, &attached),
    ]
}

/// Bridge's mailboxes, with a folder and a label named after people.
fn boxes() -> Vec<FakeBox> {
    let b = |name, attrs, uidvalidity, held: &[(u32, usize)]| FakeBox {
        name,
        attrs,
        uidvalidity,
        status: true,
        held: held.to_vec(),
    };
    let all: Vec<(u32, usize)> = UIDS.iter().copied().zip(0..).collect();
    vec![
        b("INBOX", "\\HasNoChildren", 41, &[(201, 0), (202, 5)]),
        b("Sent", "\\HasNoChildren \\Sent", 42, &[(301, 2)]),
        b("Drafts", "\\HasNoChildren \\Drafts", 43, &[]),
        b("Archive", "\\HasNoChildren \\Archive", 44, &[]),
        b("Starred", "\\HasNoChildren \\Flagged", 45, &[]),
        b("Spam", "\\HasNoChildren \\Junk", 46, &[]),
        b("Trash", "\\HasNoChildren \\Trash", 47, &[]),
        b("All Mail", "\\HasNoChildren \\All", 48, &all),
        FakeBox {
            status: false,
            ..b("Folders", "\\HasChildren", 49, &[])
        },
        b("Folders/Clients", "\\HasChildren", 50, &[]),
        b(
            "Folders/Clients/Zoltan Kwiatkowski",
            "\\HasNoChildren",
            51,
            &[(501, 0)],
        ),
        b("Labels", "\\HasChildren \\Noselect", 52, &[]),
        b("Labels/Ingrid Haugland", "\\HasNoChildren", 53, &[(601, 5)]),
    ]
}

/// A session logged in to a new scripted Bridge that holds `messages()`.
async fn bridge() -> crate::mail::Session {
    let (port, fingerprint, _) = fake_imap(boxes(), messages()).await;
    let (tls, _) = crate::mail::connect(port, Some(fingerprint)).await.unwrap();
    crate::mail::login(tls, TOBIAS_EMAIL, "password")
        .await
        .unwrap()
}

/// Two events whose titles, places, descriptions, organizer and attendees
/// hold the corpus.
fn events() -> crate::calendar::ics::Feed {
    feed(&[
        event(
            VISIT_UID,
            "20261012T160000Z",
            &header(&[
                "DTEND:20261012T170000Z",
                &format!("SUMMARY:Site visit with {ZOLTAN}"),
                &format!("LOCATION:{}\\, 0184 Oslo", GAPS[1].0),
                &format!(
                    "DESCRIPTION:{INGRID} brings the plans. Dial {} or see {}",
                    PHONES[3], URLS[1]
                ),
                &format!("ORGANIZER;CN={INGRID}:mailto:{INGRID_EMAIL}"),
                &format!("ATTENDEE;CN={OLENA};PARTSTAT=ACCEPTED:mailto:{OLENA_EMAIL}"),
                &format!("ATTENDEE;CN={TOBIAS};PARTSTAT=NEEDS-ACTION:mailto:{TOBIAS_EMAIL}"),
            ]),
        ),
        event(
            REVIEW_UID,
            "20261020T150000Z",
            &header(&[
                "DTEND:20261020T160000Z",
                &format!("SUMMARY:Review with {NIKOS} and {WANG}"),
                &format!("DESCRIPTION:Bring the receipts for card {}.", CARDS[1]),
                &format!("ORGANIZER;CN={NIKOS}:mailto:{NIKOS_EMAIL}"),
                &format!("ATTENDEE;CN={WANG};PARTSTAT=TENTATIVE:mailto:{WANG_EMAIL}"),
            ]),
        ),
    ])
}

/// A stand-in for the Proton Drive CLI, as `drive::tests::fake_cli` is,
/// that serves the planted files: it answers `list` and `info` from the
/// JSON files `answer` writes (or with "Node not found"), and a download
/// copies the file of that name from `files/` into the folder named last.
/// It writes nothing anywhere else.
fn stand_in_cli(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let script = dir.join("proton-drive");
    std::fs::write(
        &script,
        r#"#!/bin/sh
here=$(dirname "$0")
case "$2" in
list|info)
  f="$here/$2$(printf '%s' "$4" | tr / _).json"
  if [ -e "$f" ]; then cat "$f"; exit 0; fi
  echo "Node not found: $(basename "$4")" >&2
  exit 1
  ;;
download)
  for dest; do :; done
  cp "$here/files/$(basename "$8")" "$dest/" || exit 1
  echo '{"transferredItems":1,"transferredBytes":1,"skippedItems":0,"failedItems":0,"failures":[]}'
  ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// One node as the CLI prints it: a file when it has a size.
fn node(name: &str, n: u8, size: Option<usize>) -> Value {
    let mut v = json!({
        "uid": node_uid(n),
        "parentUid": node_uid(0),
        "name": { "ok": true, "value": name },
        "type": if size.is_some() { "file" } else { "folder" },
        "modificationTime": "2026-09-28T17:04:05.123Z",
    });
    if let Some(size) = size {
        v["activeRevision"] = json!({
            "uid": revision_uid(n),
            "claimedSize": size,
            "claimedDigests": { "sha1": CLAIMED_SHA1 },
            "claimedModificationTime": "2026-09-01T08:00:00.000Z",
        });
    }
    v
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or_default()
}

/// Everything the tools read, in a new folder in `base`: the app's folder,
/// and the stand-in CLI with what it lists and serves. Returns the folder,
/// the app's folder, the CLI, and every file's bytes.
fn drive_fixtures(base: &Path) -> (tempfile::TempDir, PathBuf, PathBuf, Vec<Vec<u8>>) {
    let dir = tempfile::tempdir_in(base).unwrap();
    let (root, bin) = (dir.path().join("drive"), dir.path().join("bin"));
    let files: Vec<(&str, Vec<u8>)> = vec![
        (LEASE, lease_text().into_bytes()),
        (NOTES, notes_text().into_bytes()),
        (PLAN_PDF, plan_pdf()),
        (SCAN, pdf(&[])),
        (INVOICE, invoice_text().into_bytes()),
        (PHOTO, PNG.to_vec()),
    ];
    for (path, bytes) in &files {
        let f = root.join(&path[1..]);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, bytes).unwrap();
    }
    std::fs::create_dir_all(bin.join("files")).unwrap();
    let cli = stand_in_cli(&bin);
    let cloud = cloud_text().into_bytes();
    std::fs::write(bin.join("files").join(name_of(CLOUD)), &cloud).unwrap();
    let size = |path: &str| {
        files
            .iter()
            .find(|(p, _)| *p == path)
            .map_or(cloud.len(), |(_, b)| b.len())
    };
    let undecryptable =
        json!({ "uid": node_uid(4), "name": { "ok": false, "error": "x" }, "type": "file" });
    let listings = [
        (
            "/",
            vec![
                node("Clients", 1, None),
                node("Photos", 2, None),
                node(name_of(CLOUD), 3, Some(size(CLOUD))),
                undecryptable,
            ],
        ),
        (
            "/Clients",
            vec![
                node("Zoltan Kwiatkowski", 5, None),
                node(OLENA, 6, None),
                node(name_of(PLAN_PDF), 7, Some(size(PLAN_PDF))),
                node(name_of(SCAN), 8, Some(size(SCAN))),
                node(name_of(INVOICE), 9, Some(size(INVOICE))),
                node(name_of(CLOUD_CONTRACT), 10, Some(40)),
            ],
        ),
        (
            LEASE_FOLDER,
            vec![node(name_of(LEASE), 11, Some(size(LEASE)))],
        ),
        (
            "/Clients/Олена Коваленко",
            vec![node(name_of(NOTES), 12, Some(size(NOTES)))],
        ),
        ("/Photos", vec![node(name_of(PHOTO), 13, Some(size(PHOTO)))]),
    ];
    for (path, nodes) in listings {
        answer(&bin, "list", path, &json!(nodes));
    }
    answer(
        &bin,
        "info",
        CLOUD,
        &node(name_of(CLOUD), 3, Some(size(CLOUD))),
    );
    answer(
        &bin,
        "info",
        LEASE,
        &node(name_of(LEASE), 11, Some(size(LEASE))),
    );
    let mut all: Vec<Vec<u8>> = files.into_iter().map(|(_, b)| b).collect();
    all.extend([cloud, attachment_text().into_bytes(), BINARY.to_vec()]);
    (dir, root, cli, all)
}

/// An app in aliases mode with mail, one calendar, and Drive through the
/// app's folder `local`, or through the CLI alone when there is none.
async fn app(local: Option<&Path>, cli: &Path) -> App {
    let cfg = crate::config::Config {
        mail: Some(crate::config::MailConfig {
            address: TOBIAS_EMAIL.into(),
            port: 1,
            cert_sha256: crate::digest::Sha256([0; 32]),
        }),
        drive: local.map(|f| crate::config::DriveConfig {
            folder: Some(f.to_path_buf()),
            cli: Some(cli.to_path_buf()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut app = App::from_config(cfg).unwrap();
    app.calendars = one_calendar(events()).await;
    app.drive_cli = crate::drive::cli::Cli::trusted(cli.to_path_buf());
    if local.is_none() {
        app.drive = Ok(cli_drive(&[]));
    }
    app.privacy = privacy(Some(Mode::Aliases), Some(KEY)).0;
    app
}

/// Each value of the corpus, case-folded and in NFKC, with what it is.
struct Corpus(Vec<(String, String)>);

/// `s` in NFKC, case-folded as `canon` folds names, and in NFKC again.
fn fold(s: &str) -> String {
    let nfkc: String = s.nfkc().collect();
    caseless::default_case_fold_str(&nfkc).nfkc().collect()
}

impl Corpus {
    fn add(&mut self, what: &str, value: &str) {
        if !value.is_empty() {
            self.0.push((format!("{what} {value:?}"), fold(value)));
        }
    }

    /// Every planted value, in each form a result could show it in: names
    /// whole, canonical and in parts; addresses, their local parts and
    /// domains; numbers as written and without spaces; IDs, UIDs and
    /// digests; and the local paths of the fixtures.
    fn new(files: &[Vec<u8>], paths: &[&Path]) -> Self {
        let mut c = Self(Vec::new());
        for (name, email) in PEOPLE {
            c.add("name", name);
            c.add("canonical name", &canon::name(name));
            for part in name.split_whitespace() {
                c.add("part of a name", part);
                c.add("part of a canonical name", &canon::name(part));
            }
            c.add("address", email);
            let (local, domain) = email.split_once('@').unwrap();
            c.add("address's local part", local);
            c.add("domain", domain);
            c.add("domain's name", domain.trim_end_matches(".example"));
        }
        for n in PHONES.iter().chain(&CARDS).chain(&IBANS) {
            c.add("number", n);
            c.add("number", &n.replace(' ', ""));
        }
        for p in PHONES {
            let (_, national) = p.split_once(' ').unwrap();
            c.add("national number", national);
        }
        for u in URLS {
            c.add("link", u);
            let path = u.split_once(".example").unwrap().1;
            c.add("link's path", path.split_once('?').map_or(path, |(p, _)| p));
        }
        let replies = (1..=4).map(reply_id);
        let ids = [ROOT_ID, DRAWINGS_ID, VISIT_UID, REVIEW_UID].map(String::from);
        for id in ids.into_iter().chain(replies) {
            c.add("RFC 5322 or iCalendar ID", &id);
            c.add("ID's first part", id.split_once('.').unwrap().0);
        }
        for head in MESSAGE_HEADS {
            c.add("Proton message ID", head);
        }
        for uid in UIDS {
            c.add("IMAP UID", &uid.to_string());
        }
        for n in 0..=13 {
            for uid in [node_uid(n), revision_uid(n)] {
                c.add("Proton node or revision UID", &uid);
                for half in uid.split('~') {
                    c.add("half of a Proton UID", half);
                }
            }
        }
        c.add("claimed SHA-1", CLAIMED_SHA1);
        for bytes in files {
            let sums = crate::digest::bytes(bytes);
            c.add("SHA-256 of a planted file", &sums.sha256);
            c.add("SHA-1 of a planted file", &sums.sha1);
        }
        for path in paths {
            c.add("local path", &path.to_string_lossy());
        }
        c.add("local path", &crate::config::path().to_string_lossy());
        c
    }

    /// Each planted value in `text` or in what a token in it decodes to.
    fn found_in(&self, text: &str) -> Vec<&str> {
        let mut seen = vec![fold(text)];
        seen.extend(
            decoded(text)
                .iter()
                .map(|b| fold(&String::from_utf8_lossy(b))),
        );
        self.0
            .iter()
            .filter(|(_, needle)| seen.iter().any(|s| s.contains(needle.as_str())))
            .map(|(what, _)| what.as_str())
            .collect()
    }
}

static BASE64URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9_-]{8,}").expect("a fixed pattern"));
static BASE64: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/]{8,}=*").expect("a fixed pattern"));
static HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[0-9A-Fa-f]{8,}").expect("a fixed pattern"));

/// A run of base64 without its padding, or the last character no encoder
/// leaves.
fn whole(s: &str) -> &str {
    let s = s.trim_end_matches('=');
    &s[..s.len() - usize::from(s.len() % 4 == 1)]
}

/// What every run of base64url, base64 or hex in `text` decodes to: refs,
/// handles and page tokens are base64url, and a Drive tree token held its
/// folder in hex until it was sealed (R16).
fn decoded(text: &str) -> Vec<Vec<u8>> {
    let lenient = GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent);
    let url = GeneralPurpose::new(&base64::alphabet::URL_SAFE, lenient);
    let standard = GeneralPurpose::new(&base64::alphabet::STANDARD, lenient);
    let mut out = Vec::new();
    for m in BASE64URL.find_iter(text) {
        out.extend(url.decode(whole(m.as_str())).ok());
    }
    for m in BASE64.find_iter(text) {
        out.extend(standard.decode(whole(m.as_str())).ok());
    }
    for m in HEX.find_iter(text) {
        let s = m.as_str();
        out.extend(hex::decode(&s[..s.len() & !1]).ok());
    }
    out
}

/// One call: the tool, what it was for, the error code it must give
/// (`None` when it must succeed), and its result.
struct Call {
    tool: &'static str,
    what: &'static str,
    expect: Option<&'static str>,
    /// The query, when it names a planted value as typed: R13 lets that
    /// call's result show it, as a key of `queryEntities`.
    typed: Option<String>,
    out: CallToolResult,
}

impl Call {
    /// The result's text blocks, together.
    fn text(&self) -> String {
        let texts: Vec<&str> = self
            .out
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .map(|t| t.text.as_str())
            .collect();
        texts.join("\n")
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.text()).unwrap_or(Value::Null)
    }

    /// Every string the result shows: each key and string value of its
    /// JSON, or its text when that is not JSON. A typed name stays out
    /// where R13 lets it stand: as a key of `queryEntities`.
    fn shown(&self) -> Vec<String> {
        fn walk(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::String(s) => out.push(s.clone()),
                Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
                Value::Object(map) => {
                    for (k, child) in map {
                        out.push(k.clone());
                        walk(child, out);
                    }
                }
                _ => {}
            }
        }
        let mut v = self.json();
        if v.is_null() {
            return vec![self.text()];
        }
        if let (Some(typed), Some(Value::Object(names))) = (&self.typed, v.get_mut("queryEntities"))
        {
            names.retain(|name, _| !typed.contains(name.as_str()));
        }
        let mut out = Vec::new();
        walk(&v, &mut out);
        out
    }
}

/// The first server reads Drive through the app's folder, the second
/// through the CLI alone.
const LOCAL: usize = 0;
const REMOTE: usize = 1;

struct Harness<'a> {
    servers: [Server; 2],
    /// Whether each server holds a Bridge session no failed mail call has
    /// spent: a failure drops the session, as it would a real one.
    fresh: [bool; 2],
    typed: Option<String>,
    calls: Vec<Call>,
    after_each: &'a mut dyn FnMut(&str),
}

impl Harness<'_> {
    async fn before(&mut self, server: usize) {
        if !self.fresh[server] {
            let mail = self.servers[server].app.mail().unwrap();
            mail.use_session(bridge().await).await.unwrap();
            self.fresh[server] = true;
        }
    }

    /// Record a call, check that it succeeded or failed as it must, and
    /// return its JSON.
    fn after(
        &mut self,
        server: usize,
        tool: &'static str,
        what: &'static str,
        expect: Option<&'static str>,
        out: CallToolResult,
    ) -> Value {
        let failed = out.is_error == Some(true);
        let mail = tool.parse::<Tool>().unwrap().service() == Some(Service::Mail);
        if failed && mail {
            self.fresh[server] = false;
        }
        (self.after_each)(tool);
        let call = Call {
            tool,
            what,
            expect,
            typed: self.typed.take(),
            out,
        };
        assert_eq!(
            failed,
            expect.is_some(),
            "{tool} ({what}) should {}: {}",
            if failed { "succeed" } else { "fail" },
            call.text()
        );
        let v = call.json();
        self.calls.push(call);
        v
    }
}

/// Call `$method` on server `$server` with `$req`, if it takes one, and
/// record it under the name aliases mode registers it by: the method's
/// name, less `_aliases` for the methods only aliases mode registers.
macro_rules! call {
    ($h:ident, $server:expr, $what:expr, $expect:expr, $method:ident $(, $req:expr)?) => {{
        let method = stringify!($method);
        let tool = method.strip_suffix("_aliases").unwrap_or(method);
        assert_eq!(
            Server::aliases_router().has_route(tool),
            method != tool,
            "{method} is not the method aliases mode registers as {tool}"
        );
        $h.before($server).await;
        let out = $h.servers[$server]
            .$method($(Parameters($req))?)
            .await
            .expect("a tool's failure is a result");
        $h.after($server, tool, $what, $expect, out)
    }};
}

/// Every call, every planted value, and what the tests compare them with.
struct Run {
    calls: Vec<Call>,
    corpus: Corpus,
    keys: Arc<Keys>,
    /// The names each server's router registers.
    registered: Vec<BTreeSet<String>>,
    /// Every planted file's bytes.
    files: Vec<Vec<u8>>,
    _fixtures: tempfile::TempDir,
}

impl Run {
    fn call(&self, what: &str) -> &Call {
        self.calls
            .iter()
            .find(|c| c.what == what)
            .unwrap_or_else(|| panic!("no call {what}"))
    }

    /// The nextPageToken of every result, with the tool that gave it.
    fn tokens(&self) -> Vec<(&Call, String)> {
        self.calls
            .iter()
            .filter_map(|c| Some((c, c.json()["nextPageToken"].as_str()?.to_string())))
            .collect()
    }
}

/// A ref to a person's name, as an `entities` table gives it.
fn person_ref(keys: &Keys, name: &str) -> String {
    keys.reference(EntityType::Person, &canon::name(name))
}

/// The page token in `v`, if there is one.
fn token(v: &Value) -> Option<String> {
    v["nextPageToken"].as_str().map(String::from)
}

/// Plant the corpus in `base` and call every tool the aliases-mode server
/// registers, through the server, with `after_each` run after each call.
/// Each tool runs where it reads the corpus and where it fails: a missing
/// item, a bad label, a refused handle or ref, a forged page token.
#[expect(
    clippy::too_many_lines,
    reason = "one list of every call, in the order they run"
)]
async fn every_tool(base: &Path, after_each: &mut dyn FnMut(&str)) -> Run {
    let (fixtures, root, cli, files) = drive_fixtures(base);
    let servers = [
        Server::new(Arc::new(app(Some(&root), &cli).await)),
        Server::new(Arc::new(app(None, &cli).await)),
    ];
    let registered = servers
        .iter()
        .map(|s| {
            s.tool_router
                .list_all()
                .iter()
                .map(|t| t.name.to_string())
                .collect()
        })
        .collect();
    let keys = servers[LOCAL].app.privacy.check().unwrap().keys.unwrap();
    let handle = |kind, id: &str| keys.handle(kind, id);
    let file = |path: &str| handle(ItemKind::DrivePath, path);
    let mut h = Harness {
        servers,
        fresh: [false; 2],
        typed: None,
        calls: Vec::new(),
        after_each,
    };

    call!(h, LOCAL, "status", None, get_status);
    // Unit tests reach no secret store on Linux, and list_calendars lists
    // its items.
    let unlisted = cfg!(target_os = "linux").then_some("unavailable");
    call!(h, LOCAL, "list calendars", unlisted, list_calendars);

    // Calendar.
    let window = |page_size, page_token| ListEventsReq {
        start_time: Some("2026-10-01".into()),
        end_time: Some("2026-11-01".into()),
        page_size,
        page_token,
        ..Default::default()
    };
    let page = call!(
        h,
        LOCAL,
        "list events, one a page",
        None,
        list_events,
        window(Some(1), None)
    );
    let next = window(Some(1), token(&page));
    call!(
        h,
        LOCAL,
        "list events, the next page",
        None,
        list_events,
        next
    );
    let req = SearchEventsReq {
        query: format!("ref:{}", person_ref(&keys, ZOLTAN)),
        window: window(None, None),
    };
    call!(h, LOCAL, "search events by ref", None, search_events, req);
    for (uid, what) in [
        (VISIT_UID, "get an event"),
        (REVIEW_UID, "get another event"),
    ] {
        let req = GetEventReq {
            event_id: handle(ItemKind::Event, uid),
            ..Default::default()
        };
        call!(h, LOCAL, what, None, get_event, req);
    }
    let req = GetEventReq {
        event_id: handle(ItemKind::Event, "Xr9Tq2Wm5Lp8Kz3N.gone@proton.me"),
        ..Default::default()
    };
    let what = "get a missing event";
    call!(h, LOCAL, what, Some("not_found"), get_event, req);
    let req = SearchEventsReq {
        window: window(None, None),
        ..Default::default()
    };
    let what = "search events with no query";
    call!(h, LOCAL, what, Some("invalid_argument"), search_events, req);
    let req = ListEventsReq {
        calendar_id: Some("archive".into()),
        ..window(None, None)
    };
    let what = "list events of a missing calendar";
    call!(h, LOCAL, what, Some("not_found"), list_events, req);
    let req = window(None, Some("1.1790000000.1790600000".into()));
    let what = "list events with a page token that is not sealed";
    call!(h, LOCAL, what, Some("invalid_page_token"), list_events, req);

    // Mail.
    let search = |query: String, page_token| SearchThreadsReq {
        query,
        page_size: Some(1),
        page_token,
        snippets: true,
        ..Default::default()
    };
    let page = call!(
        h,
        LOCAL,
        "search mail, one row a page",
        None,
        search_threads,
        search(String::new(), None)
    );
    let mail_token = token(&page);
    let req = search(String::new(), mail_token.clone());
    call!(
        h,
        LOCAL,
        "search mail, the next page",
        None,
        search_threads,
        req
    );
    let typed = format!("from:\"{ZOLTAN}\"");
    h.typed = Some(typed.clone());
    call!(
        h,
        LOCAL,
        "search mail by a typed name",
        None,
        search_threads,
        search(typed, None)
    );
    let req = search(format!("label:ref:{}", person_ref(&keys, INGRID)), None);
    call!(h, LOCAL, "search a label by ref", None, search_threads, req);
    let req = search(format!("label:ref:{}", person_ref(&keys, NIKOS)), None);
    let what = "search a label that does not exist, by ref";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_argument"),
        search_threads,
        req
    );
    let req = search(String::new(), Some(format!("n.48.1790000000.{}", UIDS[5])));
    let what = "search mail with a page token that is not sealed";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_page_token"),
        search_threads,
        req
    );
    for by in [
        GroupBy::From,
        GroupBy::FromDomain,
        GroupBy::To,
        GroupBy::ToDomain,
    ] {
        let req = CountMessagesReq {
            by: Some(by),
            ..Default::default()
        };
        call!(h, LOCAL, "count mail by group", None, count_messages, req);
    }
    let req = CountMessagesReq {
        query: "\"lease".into(),
        ..Default::default()
    };
    let what = "count mail with an unbalanced quote";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_argument"),
        count_messages,
        req
    );
    let message = |head: &str| MessageReq {
        message_id: handle(ItemKind::Message, head),
        ..Default::default()
    };
    call!(
        h,
        LOCAL,
        "get a message",
        None,
        get_message,
        message(MESSAGE_HEADS[0])
    );
    call!(
        h,
        LOCAL,
        "get a reply",
        None,
        get_message,
        message(MESSAGE_HEADS[1])
    );
    let what = "get the message with attachments";
    call!(h, LOCAL, what, None, get_message, message(MESSAGE_HEADS[5]));
    let what = "get a missing message";
    call!(
        h,
        LOCAL,
        what,
        Some("not_found"),
        get_message,
        message("Zz9Yy8Xx7Ww6Vv5U")
    );
    let req = MessageReq {
        message_id: MESSAGE_HEADS[0].into(),
        ..Default::default()
    };
    let what = "get a message by a value that is not a handle";
    call!(h, LOCAL, what, Some("invalid_handle"), get_message, req);
    let thread = |root: &str, page_token| ThreadReq {
        thread_id: handle(ItemKind::Thread, root),
        page_token,
        ..Default::default()
    };
    let page = call!(
        h,
        LOCAL,
        "get a thread",
        None,
        get_thread,
        thread(ROOT_ID, None)
    );
    let next = thread(ROOT_ID, token(&page));
    call!(h, LOCAL, "get a thread's next page", None, get_thread, next);
    call!(
        h,
        LOCAL,
        "get another thread",
        None,
        get_thread,
        thread(DRAWINGS_ID, None)
    );
    let what = "get a thread with another tool's page token";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_page_token"),
        get_thread,
        thread(ROOT_ID, mail_token)
    );
    call!(h, LOCAL, "list labels", None, list_labels);
    for (index, what) in [
        (0, "get a text attachment"),
        (1, "get a PDF attachment"),
        (2, "get an image attachment"),
        (3, "get a binary attachment"),
    ] {
        let req = AttachmentAliases {
            message_id: handle(ItemKind::Message, MESSAGE_HEADS[5]),
            index,
            ..Default::default()
        };
        call!(h, LOCAL, what, None, get_attachment_aliases, req);
    }
    let req = AttachmentAliases {
        message_id: handle(ItemKind::Message, MESSAGE_HEADS[5]),
        index: 9,
        ..Default::default()
    };
    let what = "get an attachment that is not there";
    call!(
        h,
        LOCAL,
        what,
        Some("not_found"),
        get_attachment_aliases,
        req
    );

    // Drive, through the app's folder.
    let folder = |path: Option<&str>, page_size, page_token| ListFolderAliases {
        folder_id: path.map(file),
        page_size,
        page_token,
    };
    call!(
        h,
        LOCAL,
        "list the top folder",
        None,
        list_folder_aliases,
        folder(None, None, None)
    );
    for path in [LEASE_FOLDER, "/Clients/Олена Коваленко", "/Photos"] {
        let req = folder(Some(path), None, None);
        call!(h, LOCAL, "list a folder", None, list_folder_aliases, req);
    }
    let req = folder(Some("/Clients"), Some(2), None);
    let page = call!(
        h,
        LOCAL,
        "list a folder, two a page",
        None,
        list_folder_aliases,
        req
    );
    let req = folder(Some("/Clients"), Some(2), token(&page));
    call!(
        h,
        LOCAL,
        "list a folder, the next page",
        None,
        list_folder_aliases,
        req
    );
    let req = folder(Some("/Clients"), None, Some("2".into()));
    let what = "list a folder with a page token that is not sealed";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_page_token"),
        list_folder_aliases,
        req
    );
    let req = SearchFilesAliases {
        query: "*".into(),
        page_size: Some(4),
        ..Default::default()
    };
    let page = call!(
        h,
        LOCAL,
        "search Drive, four a page",
        None,
        search_files_aliases,
        req
    );
    let req = SearchFilesAliases {
        query: "*".into(),
        page_size: Some(4),
        page_token: token(&page),
        ..Default::default()
    };
    call!(
        h,
        LOCAL,
        "search Drive, the next page",
        None,
        search_files_aliases,
        req
    );
    let address = keys.reference(EntityType::Email, &canon::email(TOBIAS_EMAIL));
    for (query, what) in [
        (address, "search Drive by an address's ref"),
        (person_ref(&keys, ZOLTAN), "search Drive by a name's ref"),
    ] {
        let req = SearchFilesAliases {
            query: format!("ref:{query}"),
            folder_id: Some(file("/Clients")),
            ..Default::default()
        };
        call!(h, LOCAL, what, None, search_files_aliases, req);
    }
    let req = SearchFilesAliases {
        query: "ref:nope".into(),
        ..Default::default()
    };
    let what = "search Drive by a ref that does not open";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_ref"),
        search_files_aliases,
        req
    );
    let details = |path: &str, digests| FileMetadataAliases {
        file_id: file(path),
        digests,
    };
    let what = "file details and digests";
    call!(
        h,
        LOCAL,
        what,
        None,
        get_file_metadata_aliases,
        details(LEASE, true)
    );
    // The CLI does not find this one, so its error is in the result.
    let what = "file details whose Proton view fails";
    call!(
        h,
        LOCAL,
        what,
        None,
        get_file_metadata_aliases,
        details(NOTES, true)
    );
    let req = details(&format!("{LEASE_FOLDER}/Lease draft.txt"), false);
    let what = "details of a missing file";
    call!(
        h,
        LOCAL,
        what,
        Some("not_found"),
        get_file_metadata_aliases,
        req
    );
    let read = |path: &str| ReadAliases {
        file_id: file(path),
        ..Default::default()
    };
    for (path, what) in [
        (LEASE, "read a text file"),
        (NOTES, "read a file whose name holds an address"),
        (INVOICE, "read the invoice"),
        (PLAN_PDF, "read a PDF"),
        (SCAN, "read a scan"),
        (PHOTO, "read an image"),
    ] {
        call!(h, LOCAL, what, None, read_file_content_aliases, read(path));
    }
    let req = ReadAliases {
        file_id: file(LEASE),
        offset: Some(20),
        max_chars: Some(40),
    };
    call!(
        h,
        LOCAL,
        "read a page of a text file",
        None,
        read_file_content_aliases,
        req
    );
    let what = "read a folder";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_argument"),
        read_file_content_aliases,
        read(LEASE_FOLDER)
    );
    let req = ReadAliases {
        file_id: "/Photos/site.png".into(),
        ..Default::default()
    };
    let what = "read a path that is not a handle";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_handle"),
        read_file_content_aliases,
        req
    );
    // Two rows a page, so a page ends in each folder, those named after
    // people among them.
    let mut page_token = None;
    for _ in 0..20 {
        let req = TreeAliases {
            page_size: Some(2),
            page_token: page_token.take(),
            ..Default::default()
        };
        let page = call!(
            h,
            LOCAL,
            "list the tree, two rows a page",
            None,
            list_drive_tree_aliases,
            req
        );
        page_token = token(&page);
        if page_token.is_none() {
            break;
        }
    }
    let req = TreeAliases {
        with_sha1: true,
        ..Default::default()
    };
    let what = "list the tree with the CLI's view";
    call!(h, LOCAL, what, None, list_drive_tree_aliases, req);
    let req = TreeAliases {
        folder_id: Some(file(LEASE)),
        ..Default::default()
    };
    let what = "list the tree of a file";
    call!(
        h,
        LOCAL,
        what,
        Some("invalid_argument"),
        list_drive_tree_aliases,
        req
    );

    // Drive through the CLI alone.
    let what = "list the top folder through the CLI";
    call!(
        h,
        REMOTE,
        what,
        None,
        list_folder_aliases,
        folder(None, None, None)
    );
    let what = "list a folder through the CLI";
    call!(
        h,
        REMOTE,
        what,
        None,
        list_folder_aliases,
        folder(Some("/Clients"), None, None)
    );
    let what = "file details through the CLI";
    call!(
        h,
        REMOTE,
        what,
        None,
        get_file_metadata_aliases,
        details(CLOUD, true)
    );
    let req = details(&format!("{LEASE_FOLDER}/Lease draft.txt"), true);
    let what = "details of a file the CLI does not find";
    call!(
        h,
        REMOTE,
        what,
        Some("not_found"),
        get_file_metadata_aliases,
        req
    );
    // On a Mac this would attach a RAM disk, which the drive tests make
    // only where they run alone.
    #[cfg(target_os = "linux")]
    call!(
        h,
        REMOTE,
        "read a cloud-only file",
        None,
        read_file_content_aliases,
        read(CLOUD)
    );
    let req = SearchFilesAliases {
        query: "plan".into(),
        ..Default::default()
    };
    let what = "search Drive without the app's folder";
    call!(
        h,
        REMOTE,
        what,
        Some("invalid_argument"),
        search_files_aliases,
        req
    );
    let what = "list the tree without the app's folder";
    call!(
        h,
        REMOTE,
        what,
        Some("invalid_argument"),
        list_drive_tree_aliases,
        TreeAliases::default()
    );

    let canonical = root.canonicalize().unwrap();
    let paths = [base, root.as_path(), canonical.as_path(), cli.as_path()];
    Run {
        calls: h.calls,
        corpus: Corpus::new(&files, &paths),
        keys,
        registered,
        files,
        _fixtures: fixtures,
    }
}

/// RFC I9 (T2): every tool the aliases-mode server registers is called
/// through the server, a call that fails counting too, and nothing else
/// is called.
#[tokio::test]
async fn every_aliases_tool_is_called_through_the_server() {
    let base = tempfile::tempdir().unwrap();
    let run = every_tool(base.path(), &mut |_| {}).await;
    let called: BTreeSet<String> = run.calls.iter().map(|c| c.tool.to_string()).collect();
    let routers = Server::shared_router() + Server::aliases_router();
    let aliases_mode: BTreeSet<String> = routers
        .list_all()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for registered in &run.registered {
        assert_eq!(registered, &aliases_mode);
    }
    assert_eq!(called, aliases_mode, "called, then registered");
}

/// RFC R13, R16 and R17 (T1): no planted value leaves any tool in aliases
/// mode, as text or inside a ref, handle or page token decoded: not names
/// in headers, bodies, subjects, file names or event titles, in Latin,
/// Cyrillic, Greek or Chinese; not addresses, phone, card or IBAN numbers,
/// links, Proton IDs, UIDs, digests or local paths. The values were seen:
/// every type has an alias in some `entities` table.
#[tokio::test]
async fn no_planted_value_leaves_any_tool() {
    let base = tempfile::tempdir().unwrap();
    let run = every_tool(base.path(), &mut |_| {}).await;
    let mut leaks = Vec::new();
    for c in &run.calls {
        for s in c.shown() {
            for what in run.corpus.found_in(&s) {
                leaks.push(format!("{} ({}): {what} in {s:?}", c.tool, c.what));
            }
        }
    }
    assert!(leaks.is_empty(), "{}", leaks.join("\n"));
    let mut types = BTreeSet::new();
    for c in &run.calls {
        if let Some(Value::Object(entities)) = c.json().get("entities") {
            types.extend(
                entities
                    .values()
                    .filter_map(|e| e["type"].as_str().map(String::from)),
            );
        }
    }
    for t in ["person", "email", "domain", "phone", "card", "iban"] {
        assert!(types.contains(t), "no {t} among {types:?}");
    }
    let all: String = run.calls.iter().map(Call::text).collect();
    assert!(all.contains("link 1 ("), "no link was replaced");
    let found = run.call("search Drive by an address's ref").json();
    assert_eq!(found["files"].as_array().map(Vec::len), Some(1), "{found}");
    // The documented gaps still show; once one closes, its value joins
    // the corpus.
    for (value, what) in GAPS {
        assert!(
            all.contains(value),
            "{what} no longer shows: add it to the corpus"
        );
    }
}

/// RFC R22 (T8): in aliases mode no result carries image content, an
/// embedded resource or a file's bytes in base64; the image, the scan and
/// the file that is not text each come back as a reason.
#[tokio::test]
async fn no_tool_returns_images_or_file_bytes_in_aliases_mode() {
    const SIGNATURES: [&[u8]; 5] = [
        b"\x89PNG",
        b"%PDF-",
        b"GIF8",
        b"\xFF\xD8\xFF",
        b"PK\x03\x04",
    ];
    let base = tempfile::tempdir().unwrap();
    let run = every_tool(base.path(), &mut |_| {}).await;
    for c in &run.calls {
        let at = format!("{} ({})", c.tool, c.what);
        assert!(c.out.structured_content.is_none(), "{at}");
        for block in &c.out.content {
            assert!(
                block.as_text().is_some(),
                "{at} returned a block that is not text"
            );
        }
        for s in c.shown() {
            for bytes in decoded(&s) {
                let signed = SIGNATURES.iter().any(|sig| bytes.starts_with(sig));
                let file = run.files.iter().any(|f| {
                    let head = &f[..f.len().min(12)];
                    bytes.windows(head.len()).any(|w| w == head)
                });
                assert!(!signed && !file, "{at} returned file bytes in {s:?}");
            }
        }
    }
    for what in [
        "read an image",
        "read a scan",
        "get an image attachment",
        "get a binary attachment",
    ] {
        let v = run.call(what).json();
        let reason = v["reason"].as_str().or(v["image"]["reason"].as_str());
        assert!(reason.is_some(), "{what}: {v}");
        assert!(
            v["content"].is_null() && v["image"]["text"].is_null(),
            "{what}: {v}"
        );
    }
}

/// RFC R13 and R16 (T9): every error a tool returns is a fault from the
/// aliases-mode path, a code and fixed text with the detectors that ran,
/// and every page token is sealed; neither holds a planted name, path or
/// UID, as text or decoded. The tokens are worth sealing: opened, the mail
/// cursor holds a UID and a tree token a folder named after a person.
#[tokio::test]
async fn errors_and_page_tokens_carry_no_name_path_or_uid() {
    let base = tempfile::tempdir().unwrap();
    let run = every_tool(base.path(), &mut |_| {}).await;
    let errors: Vec<&Call> = run.calls.iter().filter(|c| c.expect.is_some()).collect();
    let mut leaks = Vec::new();
    for c in &errors {
        for what in run.corpus.found_in(&c.text()) {
            leaks.push(format!("{} ({}): {what} in {}", c.tool, c.what, c.text()));
        }
    }
    let mut opened = Vec::new();
    for (c, t) in run.tokens() {
        for what in run.corpus.found_in(&t) {
            leaks.push(format!(
                "{} ({}): {what} in the page token {t:?}",
                c.tool, c.what
            ));
        }
        let tool: Tool = c.tool.parse().unwrap();
        let raw = run.keys.open_token(TokenKind::of(tool), &t);
        opened.push((c, tool, raw.ok()));
    }
    assert!(leaks.is_empty(), "{}", leaks.join("\n"));
    for c in errors {
        let v = c.json();
        let at = format!("{} ({})", c.tool, c.what);
        assert_eq!(c.out.content.len(), 1, "{at}");
        let keys: BTreeSet<&str> = v
            .as_object()
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(
            keys,
            BTreeSet::from(["detectors", "error", "message"]),
            "{at}: {}",
            c.text()
        );
        assert_eq!(v["error"].as_str(), c.expect, "{at}: {v}");
        assert_eq!(v["detectors"], json!(pipeline::DETECTORS), "{at}: {v}");
    }
    for (c, _, raw) in &opened {
        assert!(
            raw.is_some(),
            "{} ({}): the page token is not sealed",
            c.tool,
            c.what
        );
    }
    // Opened, and decoded where a part is hex, as a tree token's folder is.
    let holds = |kind: Tool, planted: &[&str]| {
        let raws = opened.iter().filter(|(_, t, _)| *t == kind);
        raws.filter_map(|(_, _, raw)| raw.as_ref()).any(|raw| {
            let mut seen = vec![raw.clone()];
            seen.extend(
                decoded(raw)
                    .iter()
                    .map(|b| String::from_utf8_lossy(b).into_owned()),
            );
            planted.iter().any(|p| seen.iter().any(|s| s.contains(p)))
        })
    };
    let uids: Vec<String> = UIDS.iter().map(u32::to_string).collect();
    let uids: Vec<&str> = uids.iter().map(String::as_str).collect();
    assert!(
        holds(Tool::SearchThreads, &uids),
        "no mail cursor held a UID"
    );
    assert!(
        holds(Tool::ListDriveTree, &[ZOLTAN, OLENA]),
        "no tree token held a folder named after a person"
    );
}

/// What T7's child test reads its fixture folder from.
const CHILD: &str = "PROTONCTL_TEST_EVERY_TOOL_FIXTURES";

/// RFC R10 (T7): aliases mode writes no file. Every tool runs in a child
/// test process whose home and temporary folder are new and empty, with
/// its fixtures in a folder beside them; after every call the memory folder
/// a cloud-only read uses is empty again, and at the end no file is new.
#[test]
fn aliases_mode_writes_no_file() {
    let root = tempfile::tempdir().unwrap();
    let [home, tmp, fixtures] = ["home", "tmp", "fixtures"].map(|d| root.path().join(d));
    for d in [&home, &tmp, &fixtures] {
        std::fs::create_dir(d).unwrap();
    }
    let test = format!(
        "{}::every_tool_in_a_throwaway_home",
        module_path!().split_once("::").unwrap().1
    );
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test.as_str(), "--exact", "--ignored", "--nocapture"])
        .env("HOME", &home)
        .env("TMPDIR", &tmp)
        .env(CHILD, &fixtures)
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("PROTONCTL_CONFIG")
        // No D-Bus session, so nothing reaches the user's keyring.
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{said}");
    assert!(
        said.contains("1 passed"),
        "the child test did not run: {said}"
    );
}

/// Every file and folder under `dir`, by path.
fn files_under(dir: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            out.extend(files_under(&path));
        }
        out.insert(path);
    }
    out
}

/// T7's child: run by `aliases_mode_writes_no_file` with a throwaway home
/// and temporary folder, and a no-op anywhere else.
#[tokio::test]
#[ignore = "aliases_mode_writes_no_file runs it, in a throwaway home"]
async fn every_tool_in_a_throwaway_home() {
    let Some(fixtures) = std::env::var_os(CHILD) else {
        return;
    };
    let watched = [crate::config::home(), std::env::temp_dir()];
    let before: BTreeSet<PathBuf> = watched.iter().flat_map(|d| files_under(d)).collect();
    // The folder in memory (here /dev/shm) a cloud-only read fetches into:
    // allowed, but empty again after every call.
    #[cfg(target_os = "linux")]
    let memory = crate::platform::memory_disk(Path::new("unused")).unwrap();
    let mut emptied = |tool: &str| {
        #[cfg(target_os = "linux")]
        assert_eq!(
            files_under(&memory),
            BTreeSet::new(),
            "{tool} left these in memory"
        );
        #[cfg(not(target_os = "linux"))]
        let _ = tool;
    };
    let run = every_tool(Path::new(&fixtures), &mut emptied).await;
    drop(run);
    let after: BTreeSet<PathBuf> = watched.iter().flat_map(|d| files_under(d)).collect();
    // The Drive CLI's lock file, which every protonctl process takes
    // around a run of the CLI: empty, so it holds no content (R10).
    let lock = std::env::temp_dir().join("protonctl-test/cli.lock");
    let lock_empty = std::fs::metadata(&lock).is_ok_and(|m| m.len() == 0);
    let new: Vec<&PathBuf> = after
        .difference(&before)
        .filter(|p| !(lock_empty && (**p == lock || Some(p.as_path()) == lock.parent())))
        .collect();
    // As `serve` does when it stops.
    crate::content::remove_downloads();
    assert!(new.is_empty(), "aliases mode wrote {new:?}");
}
