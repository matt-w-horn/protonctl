//! Proton Mail through Proton Mail Bridge, which serves IMAP on 127.0.0.1 and
//! does all the decryption (RFC R3). Bridge's TLS certificate is self-signed,
//! so `protonctl setup mail` pins its SHA-256 the first time it connects and
//! every later connection refuses any other certificate (RFC R9): another
//! process listening on the port cannot collect the Bridge password.

use std::net::{IpAddr, Ipv4Addr};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use secrecy::ExposeSecret as _;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

mod body;
mod query;
pub mod read;

pub use read::Mail;

use crate::config::MailConfig;
use crate::secret;

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const CONNECT_LIMIT: Duration = Duration::from_secs(5);
const REPLY_LIMIT: Duration = Duration::from_secs(20);
/// In SSL mode Bridge waits for the client to speak first; in STARTTLS mode it
/// greets at once. Silence for this long means SSL mode.
const GREETING_WAIT: Duration = Duration::from_millis(700);
/// Bridge's app, opened hidden when nothing answers on its port (RFC Q2). On
/// 2026-10-03 it took the login about 10 s after opening.
const BRIDGE_APP: &str = "com.protonmail.bridge";
const BRIDGE_START: Duration = Duration::from_secs(40);

/// Nothing answered on Bridge's port: the cue to open the Bridge app.
#[derive(Debug)]
pub struct NotListening(u16);

impl std::fmt::Display for NotListening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "nothing answers on 127.0.0.1:{}; is Proton Mail Bridge running and signed in?",
            self.0
        )
    }
}

impl std::error::Error for NotListening {}

pub type Fingerprint = [u8; 32];

fn parse_hex(s: &str) -> Result<Fingerprint> {
    let mut fp = [0u8; 32];
    hex::decode_to_slice(s, &mut fp).context("cert_sha256 in the config is not 64 hex digits")?;
    Ok(fp)
}

fn sha256(der: &[u8]) -> Fingerprint {
    let mut fp = [0; 32];
    fp.copy_from_slice(ring::digest::digest(&ring::digest::SHA256, der).as_ref());
    fp
}

/// Accepts one certificate: the pinned one or, when nothing is pinned yet,
/// whatever Bridge presents (recorded for setup). The handshake signature is
/// still verified, so the server must hold that certificate's private key.
#[derive(Debug)]
struct Pin {
    pinned: Option<Fingerprint>,
    seen: Mutex<Option<Fingerprint>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = sha256(end_entity);
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(fp);
        }
        match self.pinned {
            Some(pinned) if pinned != fp => Err(rustls::Error::General(
                "Bridge's certificate does not match the one pinned at setup".into(),
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algorithms = &self.provider.signature_verification_algorithms;
        verify_tls12_signature(message, cert, dss, algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algorithms = &self.provider.signature_verification_algorithms;
        verify_tls13_signature(message, cert, dss, algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// TLS to Bridge on 127.0.0.1:`port`, and the fingerprint of its certificate.
pub async fn connect(
    port: u16,
    pinned: Option<Fingerprint>,
) -> Result<(TlsStream<TcpStream>, Fingerprint)> {
    let tcp = tokio::time::timeout(CONNECT_LIMIT, TcpStream::connect((LOOPBACK, port)))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or(NotListening(port))?;
    let mut first = [0u8; 1];
    match tokio::time::timeout(GREETING_WAIT, tcp.peek(&mut first)).await {
        Err(_) => {} // silence: Bridge expects TLS first, as in SSL mode
        Ok(Ok(0)) => bail!("Bridge closed the connection on 127.0.0.1:{port}"),
        Ok(Ok(_)) => bail!(
            "Bridge greeted in plain text, so it is in STARTTLS mode; in Bridge, set \
             Advanced settings → Connection mode to SSL"
        ),
        Ok(Err(e)) => return Err(e).context("cannot read from Bridge"),
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let pin = Arc::new(Pin {
        pinned,
        seen: Mutex::new(None),
        provider: provider.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(pin.clone())
        .with_no_client_auth();
    let handshake =
        TlsConnector::from(Arc::new(config)).connect(ServerName::IpAddress(LOOPBACK.into()), tcp);
    let tls = tokio::time::timeout(CONNECT_LIMIT, handshake)
        .await
        .context("the TLS handshake with Bridge timed out")?
        .context("the TLS handshake with Bridge failed")?;
    let seen = pin.seen.lock().ok().and_then(|s| *s);
    Ok((tls, seen.context("Bridge presented no certificate")?))
}

pub type Session = async_imap::Session<TlsStream<TcpStream>>;

/// Log in over a connection from `connect`, reading Bridge's greeting first.
pub async fn login(tls: TlsStream<TcpStream>, user: &str, password: &str) -> Result<Session> {
    let mut client = async_imap::Client::new(tls);
    tokio::time::timeout(REPLY_LIMIT, client.read_response())
        .await
        .context("Bridge sent no greeting within 20 s")??
        .context("Bridge closed the connection before greeting")?;
    tokio::time::timeout(REPLY_LIMIT, client.login(user, password))
        .await
        .context("Bridge did not answer the login within 20 s")?
        .map_err(|(e, _)| {
            anyhow::anyhow!(
                "Bridge refused the login ({e}); check the username and the Bridge password, \
                 which is not your Proton password"
            )
        })
}

/// A logged-in session with the stored password, over the pinned connection.
/// If nothing answers on Bridge's port, the Bridge app is opened hidden first.
pub async fn open(cfg: &MailConfig) -> Result<Session> {
    let pinned = parse_hex(&cfg.cert_sha256)?;
    let account = secret::bridge_account(&cfg.address);
    let password = tokio::task::spawn_blocking(move || secret::get(&account))
        .await??
        .context("no Bridge password in the Keychain; run `protonctl setup mail` again")?;
    let attempt = async || -> Result<Session> {
        let (tls, _) = connect(cfg.port, Some(pinned)).await.context(
            "if Bridge was reinstalled, its certificate changed: delete [mail] from the config \
             and run `protonctl setup mail` again",
        )?;
        login(tls, &cfg.address, password.expose_secret()).await
    };
    match attempt().await {
        Err(e) if e.downcast_ref::<NotListening>().is_some() => {}
        result => return result,
    }
    // Until Bridge has loaded the account it refuses logins ("no such user"),
    // so every failure is retried until the deadline.
    // No inherited stdio: in `serve`, stdout is the MCP stream (RFC section 8).
    let status = tokio::process::Command::new("/usr/bin/open")
        .args(["-g", "-j", "-b", BRIDGE_APP])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .context("cannot run /usr/bin/open")?;
    if !status.success() {
        bail!(
            "nothing answers on Bridge's port, and Proton Mail Bridge ({BRIDGE_APP}) could not be opened; is it installed?"
        );
    }
    let deadline = tokio::time::Instant::now() + BRIDGE_START;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        match attempt().await {
            Ok(session) => return Ok(session),
            Err(_) if tokio::time::Instant::now() < deadline => {}
            Err(e) => {
                return Err(e.context(
                    "opened Proton Mail Bridge, but it did not take the login within 40 s",
                ));
            }
        }
    }
}

/// For `protonctl doctor`: the pinned certificate still matches and the stored
/// password still logs in.
pub async fn check(cfg: &MailConfig) -> Result<String> {
    open(cfg).await?.logout().await.ok();
    Ok(format!(
        "Bridge on 127.0.0.1:{} with the pinned certificate; login accepted for {}",
        cfg.port, cfg.address
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::PrivateKeyDer;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    /// A listener on 127.0.0.1 that speaks TLS with a fresh self-signed
    /// certificate, as Bridge does, and that certificate's fingerprint.
    pub(super) async fn tls_listener() -> (TcpListener, TlsAcceptor, u16, Fingerprint) {
        let issued = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        let cert = issued.cert.der().clone();
        let key = PrivateKeyDer::Pkcs8(issued.signing_key.serialize_der().into());
        let fingerprint = sha256(&cert);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
        let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (
            listener,
            TlsAcceptor::from(Arc::new(config)),
            port,
            fingerprint,
        )
    }

    /// A fake Bridge: TLS with a fresh self-signed certificate, one IMAP session.
    async fn fake_bridge(accept_login: bool) -> (u16, Fingerprint) {
        let (listener, acceptor, port, fingerprint) = tls_listener().await;
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let Ok(tls) = acceptor.accept(tcp).await else {
                return;
            };
            let mut io = BufReader::new(tls);
            let greeting = b"* OK [CAPABILITY IMAP4rev1] Proton Mail Bridge ready\r\n";
            io.get_mut().write_all(greeting).await.unwrap();
            // Whatever tag the client uses, the credentials must arrive quoted and escaped.
            let mut login = String::new();
            io.read_line(&mut login).await.unwrap();
            let (tag, command) = login.split_once(' ').unwrap();
            let expected = "LOGIN \"user@example.test\" \"pa\\\"ss\\\\word\"\r\n";
            let reply = if accept_login && command == expected {
                format!("{tag} OK LOGIN completed\r\n")
            } else {
                format!("{tag} NO [AUTHENTICATIONFAILED] Invalid username or password\r\n")
            };
            io.get_mut().write_all(reply.as_bytes()).await.unwrap();
        });
        (port, fingerprint)
    }

    const USER: &str = "user@example.test";
    const PASSWORD: &str = "pa\"ss\\word";

    #[tokio::test]
    async fn first_connection_records_the_certificate_and_logs_in() {
        let (port, fingerprint) = fake_bridge(true).await;
        let (tls, seen) = connect(port, None).await.unwrap();
        assert_eq!(seen, fingerprint);
        login(tls, USER, PASSWORD).await.unwrap();
    }

    #[tokio::test]
    async fn the_pinned_certificate_is_accepted_and_any_other_refused() {
        let (port, fingerprint) = fake_bridge(true).await;
        assert!(connect(port, Some(fingerprint)).await.is_ok());
        let (port, _) = fake_bridge(true).await;
        let err = connect(port, Some([7; 32])).await.unwrap_err();
        assert!(format!("{err:#}").contains("does not match"), "{err:#}");
    }

    #[tokio::test]
    async fn a_refused_login_says_so() {
        let (port, _) = fake_bridge(false).await;
        let (tls, _) = connect(port, None).await.unwrap();
        let err = login(tls, USER, PASSWORD).await.unwrap_err();
        assert!(err.to_string().contains("refused the login"), "{err}");
    }

    #[tokio::test]
    async fn a_plain_text_greeting_means_starttls_mode() {
        let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            tcp.write_all(b"* OK [CAPABILITY IMAP4rev1 STARTTLS] ready\r\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        let err = connect(port, None).await.unwrap_err();
        assert!(err.to_string().contains("STARTTLS mode"), "{err}");
    }

    #[tokio::test]
    async fn nothing_listening_is_reported_plainly() {
        let port = TcpListener::bind((LOOPBACK, 0))
            .await
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = connect(port, None).await.unwrap_err();
        assert!(
            err.to_string().contains("is Proton Mail Bridge running"),
            "{err}"
        );
        // The typed cue that makes `open` start the Bridge app.
        assert!(err.downcast_ref::<NotListening>().is_some(), "{err:#}");
    }

    #[test]
    fn fingerprints_round_trip_through_hex() {
        let fp = sha256(b"x");
        assert_eq!(parse_hex(&hex::encode(fp)).unwrap(), fp);
        assert!(parse_hex("abc").is_err());
        assert!(parse_hex(&"zz".repeat(32)).is_err());
        assert!(parse_hex(&format!("+{}", "0".repeat(63))).is_err());
    }
}
