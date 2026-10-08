//! The privacy setting (RFC R26) and aliases mode (section 6). `Privacy`
//! holds the mode the server started in, the key and the name model;
//! `check()` runs before every call, in both modes, and the pipeline
//! rewrites every aliases-mode result before it leaves (R13).

pub mod canon;
pub mod detect;
pub mod error;
#[cfg(test)]
mod eval;
pub mod fields;
pub mod ident;
pub mod key;
pub mod pipeline;
pub mod words;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{Result, bail};

use detect::model::{LoadError, Model};
pub use error::Fault;
use key::{KeySource, KeyWatch, Keys};

use crate::secret::{self, Account};

/// The alias format's version: the `v1` in the key labels (RFC Q19).
pub const FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Aliases,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Aliases => "aliases",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "off" => Ok(Self::Off),
            "aliases" => Ok(Self::Aliases),
            other => bail!("the privacy mode is {other:?}, neither \"off\" nor \"aliases\""),
        }
    }
}

/// Where the mode is read: the secret store's `privacy-mode` comment (Q28),
/// or a fake in tests.
pub trait ModeSource: Send + Sync {
    /// `None` when no mode is set (Q27).
    fn read(&self) -> Result<Option<Mode>>;
}

pub struct StoredMode;

impl ModeSource for StoredMode {
    fn read(&self) -> Result<Option<Mode>> {
        secret::comment(&Account::PrivacyMode)?
            .as_deref()
            .map(Mode::parse)
            .transpose()
    }
}

/// Write the mode, as `setup privacy` and `setup privacy --off` do.
pub fn store_mode(mode: Mode) -> Result<()> {
    let value = secret::Secret::from(mode.as_str());
    secret::set_with_comment(&Account::PrivacyMode, &value, mode.as_str())
}

/// Where the name model comes from (RFC Q23): its folder, or in tests
/// none.
pub trait ModelSource: Send + Sync {
    /// The model, loaded once per process. `Ok(None)` only in tests: the
    /// pipeline then runs the patterns and the dictionary alone, and
    /// `detectors` says so.
    fn load(&self) -> Result<Option<Arc<Model>>, LoadError>;
}

/// The model in `detect::model::dir()`, checked and tested on load.
pub struct StoredModel;

impl ModelSource for StoredModel {
    fn load(&self) -> Result<Option<Arc<Model>>, LoadError> {
        Model::load().map(|m| Some(Arc::new(m)))
    }
}

/// One call's view: the mode, and in aliases mode the keys as they were
/// when the call began, so a rotation never changes keys mid-call, and
/// the name model.
#[derive(Clone, Debug)]
pub struct Session {
    pub mode: Mode,
    pub keys: Option<Arc<Keys>>,
    /// `None` in off mode, and in tests that run without a model.
    pub model: Option<Arc<Model>>,
}

impl Session {
    /// The keys of an aliases-mode session.
    pub fn keys(&self) -> Result<&Keys, Fault> {
        self.keys.as_deref().ok_or(Fault::PrivacyKeyMissing)
    }
}

/// Held by `App`; one per process.
pub struct Privacy {
    /// The mode read at start; `Err` when it could not be read.
    started: Result<Option<Mode>, String>,
    /// Set once the configured mode differs from `started`; never cleared (R26).
    refusing: AtomicBool,
    source: Box<dyn ModeSource>,
    keys: KeyWatch,
    model_source: Box<dyn ModelSource>,
    /// The model, loaded on the first aliases-mode call and kept; a load
    /// that failed stays failed until the process restarts, as a mode
    /// change does (R26), so no call reads the files again.
    model: OnceLock<Result<Option<Arc<Model>>, String>>,
}

impl Privacy {
    /// With no model: the pipeline tests that need one pass it to the
    /// pipeline themselves.
    #[cfg(test)]
    pub fn new(source: Box<dyn ModeSource>, keys: Box<dyn KeySource>) -> Self {
        Self::with_model(source, keys, Box::new(tests::NoModel))
    }

    pub fn with_model(
        source: Box<dyn ModeSource>,
        keys: Box<dyn KeySource>,
        model_source: Box<dyn ModelSource>,
    ) -> Self {
        let started = source.read().map_err(|e| format!("{e:#}"));
        Self {
            started,
            refusing: AtomicBool::new(false),
            source,
            keys: KeyWatch::new(keys),
            model_source,
            model: OnceLock::new(),
        }
    }

    /// The secret store's mode and key.
    pub fn stored() -> Self {
        Self::with_model(
            Box::new(StoredMode),
            Box::new(key::StoredKey),
            Box::new(StoredModel),
        )
    }

    /// The name model, loaded on first use; `serve` calls this at start
    /// in aliases mode so the first call does not wait for it.
    pub fn model(&self) -> Result<Option<Arc<Model>>, Fault> {
        #[expect(
            clippy::map_err_ignore,
            reason = "R13: the cause names the model's path; `doctor` shows it, a fault does not"
        )]
        self.model
            .get_or_init(|| self.model_source.load().map_err(|e| e.to_string()))
            .clone()
            .map_err(|_| Fault::ModelUnavailable)
    }

    /// The mode this process started in, as `get_status` and the tool list
    /// show it.
    pub fn started(&self) -> Option<Mode> {
        self.started.as_ref().ok().copied().flatten()
    }

    /// R26, before any I/O of a call: refuse when no mode is set, when the
    /// setting cannot be read, once the configured mode differs from the
    /// start mode (and from then on), or in aliases mode without a key.
    pub fn check(&self) -> Result<Session, Fault> {
        let started = match &self.started {
            Err(_) => return Err(Fault::PrivacyModeUnreadable),
            Ok(m) => *m,
        };
        if self.refusing.load(Ordering::SeqCst) {
            return Err(Fault::PrivacyModeChanged);
        }
        #[expect(
            clippy::map_err_ignore,
            reason = "R13: the store's error can name the item; the fault says what to run"
        )]
        let now = self
            .source
            .read()
            .map_err(|_| Fault::PrivacyModeUnreadable)?;
        if now != started {
            self.refusing.store(true, Ordering::SeqCst);
            return Err(Fault::PrivacyModeChanged);
        }
        match started {
            None => Err(Fault::PrivacyModeUnset),
            Some(Mode::Off) => Ok(Session {
                mode: Mode::Off,
                keys: None,
                model: None,
            }),
            Some(Mode::Aliases) => match self.keys.current() {
                Ok(Some(keys)) => Ok(Session {
                    mode: Mode::Aliases,
                    keys: Some(keys),
                    // Without a working model every call is refused (R13).
                    model: self.model()?,
                }),
                Ok(None) => Err(Fault::PrivacyKeyMissing),
                Err(_) => Err(Fault::PrivacyKeyUnreadable),
            },
        }
    }

    /// The model for `doctor`, in aliases mode: where it loaded from, or
    /// why it did not, with the path a fault leaves out.
    pub fn model_check(&self) -> anyhow::Result<String> {
        match self.model() {
            Ok(_) => Ok(format!("loaded from {}", detect::model::dir().display())),
            Err(_) => match self.model.get() {
                Some(Err(cause)) => anyhow::bail!("{cause}"),
                _ => anyhow::bail!("the name model could not be loaded a moment ago"),
            },
        }
    }

    /// The setting as `status` shows it.
    pub fn setting(&self) -> Setting {
        match &self.started {
            Err(_) => Setting::Unreadable,
            Ok(None) => Setting::Unset,
            Ok(Some(Mode::Off)) => Setting::Off,
            Ok(Some(Mode::Aliases)) => Setting::Aliases,
        }
    }

    /// `check` for `doctor`, which runs on this machine for its user, so it
    /// gives the cause a fault leaves out (R13 holds for results, not here).
    pub fn diagnose(&self) -> anyhow::Result<&'static str> {
        if let Err(cause) = &self.started {
            anyhow::bail!("the privacy setting cannot be read: {cause}");
        }
        match self.check() {
            Ok(session) => Ok(session.mode.as_str()),
            Err(Fault::PrivacyModeUnreadable) => match self.source.read() {
                Err(cause) => anyhow::bail!("the privacy setting cannot be read: {cause:#}"),
                Ok(_) => anyhow::bail!("the privacy setting could not be read a moment ago"),
            },
            Err(Fault::PrivacyKeyUnreadable) => match self.keys.current() {
                Err(cause) => anyhow::bail!(
                    "the privacy key cannot be used: {cause:#}; let protonctl read it, or replace it with `protonctl rotate-key`"
                ),
                Ok(_) => anyhow::bail!("the privacy key could not be read a moment ago"),
            },
            Err(Fault::ModelUnavailable) => self.model_check().map(|_| "aliases"),
            Err(f) => anyhow::bail!("{}", f.message()),
        }
    }
}

/// The privacy setting as this process read it at start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Setting {
    Off,
    Aliases,
    Unset,
    Unreadable,
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use key::tests::FakeKey;
    use std::sync::Mutex;

    /// A mode a test can change; `Err` stands for an unreadable setting.
    pub struct FakeMode(pub Mutex<Result<Option<Mode>, ()>>);

    /// No model: the pipeline runs without one, and `detectors` says so.
    pub struct NoModel;

    impl ModelSource for NoModel {
        fn load(&self) -> Result<Option<Arc<Model>>, LoadError> {
            Ok(None)
        }
    }

    /// A model folder with nothing in it.
    struct NotInstalled;

    impl ModelSource for NotInstalled {
        fn load(&self) -> Result<Option<Arc<Model>>, LoadError> {
            Err(LoadError::NotInstalled("/nowhere/model".into()))
        }
    }

    impl ModeSource for Arc<FakeMode> {
        fn read(&self) -> Result<Option<Mode>> {
            self.0
                .lock()
                .unwrap()
                .map_err(|()| anyhow::anyhow!("unreadable"))
        }
    }

    pub fn privacy(
        mode: Option<Mode>,
        key: Option<[u8; 32]>,
    ) -> (Privacy, Arc<FakeMode>, Arc<FakeKey>) {
        let m = Arc::new(FakeMode(Mutex::new(Ok(mode))));
        let k = Arc::new(FakeKey(Mutex::new(key)));
        (
            Privacy::new(Box::new(Arc::clone(&m)), Box::new(Arc::clone(&k))),
            m,
            k,
        )
    }

    #[test]
    fn no_mode_refuses_every_call() {
        let (p, _, _) = privacy(None, None);
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyModeUnset);
    }

    #[test]
    fn a_mode_change_refuses_until_restart_even_when_changed_back() {
        let (p, m, _) = privacy(Some(Mode::Off), None);
        assert_eq!(p.check().unwrap().mode, Mode::Off);
        *m.0.lock().unwrap() = Ok(Some(Mode::Aliases));
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyModeChanged);
        *m.0.lock().unwrap() = Ok(Some(Mode::Off));
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyModeChanged);
        // Choosing a mode while a server runs with none also needs a restart.
        let (p, m, _) = privacy(None, None);
        *m.0.lock().unwrap() = Ok(Some(Mode::Off));
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyModeChanged);
    }

    /// M5.2, R13: aliases mode with its key but no working model refuses
    /// every call, as it does without its key, and keeps refusing after
    /// the files appear, until a restart; `doctor` names the folder.
    #[test]
    fn aliases_mode_refuses_without_a_working_model() {
        let mode = Arc::new(FakeMode(Mutex::new(Ok(Some(Mode::Aliases)))));
        let key = Arc::new(FakeKey(Mutex::new(Some([3; 32]))));
        let p = Privacy::with_model(Box::new(mode), Box::new(key), Box::new(NotInstalled));
        assert_eq!(p.check().unwrap_err(), Fault::ModelUnavailable);
        assert_eq!(p.check().unwrap_err(), Fault::ModelUnavailable);
        let said = format!("{:#}", p.diagnose().unwrap_err());
        assert!(
            said.contains("not installed") && said.contains("/nowhere/model"),
            "{said}"
        );
        assert!(format!("{:#}", p.model_check().unwrap_err()).contains("/nowhere/model"));
        let message = Fault::ModelUnavailable.message();
        assert!(
            message.contains("protonctl/model") && message.contains("not installed"),
            "{message}"
        );
        // Off mode needs no model.
        let mode = Arc::new(FakeMode(Mutex::new(Ok(Some(Mode::Off)))));
        let p = Privacy::with_model(Box::new(mode), Box::new(Arc::new(FakeKey::default())), Box::new(NotInstalled));
        assert_eq!(p.check().unwrap().mode, Mode::Off);
    }

    #[test]
    fn aliases_mode_refuses_without_a_key_and_never_falls_back() {
        let (p, _, k) = privacy(Some(Mode::Aliases), None);
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyKeyMissing);
        *k.0.lock().unwrap() = Some([3; 32]);
        let s = p.check().unwrap();
        assert_eq!(
            (s.mode, s.keys.unwrap().id.clone()),
            (Mode::Aliases, Keys::derive(&[3; 32]).id)
        );
        *k.0.lock().unwrap() = None;
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyKeyMissing);
    }

    /// A key that is stored but cannot be read is not "missing", and doctor
    /// names the cause rather than pointing back at itself.
    #[test]
    fn an_unreadable_key_is_not_a_missing_one() {
        struct Refused;
        impl KeySource for Refused {
            fn key_id(&self) -> anyhow::Result<Option<String>> {
                Ok(Some("an id".into()))
            }
            fn load(&self) -> anyhow::Result<Option<zeroize::Zeroizing<[u8; 32]>>> {
                anyhow::bail!("the Keychain prompt was denied")
            }
        }
        let mode = Arc::new(FakeMode(Mutex::new(Ok(Some(Mode::Aliases)))));
        let p = Privacy::new(Box::new(mode), Box::new(Refused));
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyKeyUnreadable);
        let said = format!("{:#}", p.diagnose().unwrap_err());
        assert!(
            said.contains("prompt was denied") && !said.contains("doctor"),
            "{said}"
        );
        let unreadable = Privacy::new(
            Box::new(Arc::new(FakeMode(Mutex::new(Err(()))))),
            Box::new(Refused),
        );
        assert_eq!(unreadable.setting(), Setting::Unreadable);
        let said = format!("{:#}", unreadable.diagnose().unwrap_err());
        assert!(
            said.contains("unreadable") && !said.contains("doctor"),
            "{said}"
        );
        assert_eq!(privacy(None, None).0.setting(), Setting::Unset);
    }

    #[test]
    fn an_unreadable_setting_refuses_in_either_mode() {
        let (p, m, _) = privacy(Some(Mode::Off), None);
        *m.0.lock().unwrap() = Err(());
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyModeUnreadable);
        let m = Arc::new(FakeMode(Mutex::new(Err(()))));
        let p = Privacy::new(Box::new(m), Box::new(Arc::new(FakeKey::default())));
        assert_eq!(p.check().unwrap_err(), Fault::PrivacyModeUnreadable);
        assert!(Mode::parse("alias").is_err());
    }
}
