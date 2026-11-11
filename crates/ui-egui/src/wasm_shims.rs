//! Stand-ins for the desktop updater and assistant when those crates are not linked (wasm).
//! UI code still type-checks; nothing here talks to the network or installs software.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use bimcraft_core::bridge::Access;
use bimcraft_core::engine::Session;

pub mod bimcraft_assistant {
    pub use super::assistant_shim::*;
}

pub mod bimcraft_update {
    pub use super::update_shim::*;
}

pub mod assistant_shim {
    use super::*;

    pub const LABEL: &str = "Assistant";

    pub fn key_ok(key: &str) -> bool {
        !key.is_empty()
    }

    pub fn model_ok(model: &str) -> bool {
        !model.is_empty()
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Provider {
        Anthropic,
        OpenAi,
        Gemini,
    }

    impl Provider {
        pub const ALL: [Provider; 3] = [Provider::Anthropic, Provider::OpenAi, Provider::Gemini];

        pub fn name(self) -> &'static str {
            match self {
                Provider::Anthropic => "Anthropic Claude",
                Provider::OpenAi => "OpenAI",
                Provider::Gemini => "Google Gemini",
            }
        }

        pub fn key(self) -> &'static str {
            match self {
                Provider::Anthropic => "anthropic",
                Provider::OpenAi => "openai",
                Provider::Gemini => "gemini",
            }
        }

        pub fn from_key(key: &str) -> Option<Provider> {
            Provider::ALL.into_iter().find(|p| p.key() == key)
        }

        pub fn env(self) -> &'static str {
            match self {
                Provider::Anthropic => "ANTHROPIC_API_KEY",
                Provider::OpenAi => "OPENAI_API_KEY",
                Provider::Gemini => "GEMINI_API_KEY",
            }
        }

        pub fn models(self) -> &'static [&'static str] {
            match self {
                Provider::Anthropic => &["claude-sonnet-5-5"],
                Provider::OpenAi => &["gpt-5.5"],
                Provider::Gemini => &["gemini-3.5-flash"],
            }
        }

        pub fn default_model(self) -> &'static str {
            self.models()[0]
        }
    }

    pub enum Line {
        You(String),
        Says(String),
        Used { name: String, args: String, result: String, error: bool },
        Note(String),
    }

    pub struct Chat {
        pub provider: Provider,
        pub model: String,
        pub log: Vec<Line>,
        pub wake: Arc<dyn Fn() + Send + Sync>,
    }

    impl Chat {
        pub fn new(_net: Arc<dyn net::Net>) -> Chat {
            Chat {
                provider: Provider::Anthropic,
                model: Provider::Anthropic.default_model().into(),
                log: Vec::new(),
                wake: Arc::new(|| {}),
            }
        }

        pub fn busy(&self) -> bool {
            false
        }

        pub fn clear(&mut self) {
            self.log.clear();
        }

        pub fn send(&mut self, _text: &str, _key: &str) {}

        pub fn poll(&mut self, _session: &mut Session, _access: Access) -> bool {
            false
        }

        pub fn stop(&mut self, _session: &mut Session) {}

        pub fn reset(&mut self) {
            self.log.clear();
        }
    }

    pub mod net {
        pub trait Net: Send + Sync {}
    }

    pub mod seal {
        pub const PASSPHRASE_MIN: usize = 8;

        pub fn needs_passphrase(_: &str) -> bool {
            false
        }

        pub fn well_formed(_: &str) -> bool {
            false
        }

        pub fn open(_: &str) -> Result<String, String> {
            Err("assistant is not in this build".into())
        }

        pub fn open_passphrase(_: &str, _: &str) -> Result<String, String> {
            Err("assistant is not in this build".into())
        }

        pub fn seal(_: &str) -> Result<String, String> {
            Err("assistant is not in this build".into())
        }

        pub fn seal_passphrase(_: &str, _: &str) -> Result<String, String> {
            Err("assistant is not in this build".into())
        }
    }
}

pub mod update_shim {
    use super::*;

    #[derive(Clone)]
    pub struct Version(String);

    impl std::fmt::Display for Version {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    #[derive(Clone)]
    pub struct Installer {
        pub size: u64,
    }

    #[derive(Clone)]
    pub struct Release {
        pub version: Version,
        pub page: String,
        pub installer: Option<Installer>,
    }

    pub enum State {
        Idle,
        Available(Release),
        Downloading { release: Release, done: Arc<AtomicU64> },
        Ready { release: Release, file: PathBuf },
        Installing { release: Release, file: PathBuf },
    }

    pub enum Note {
        UpToDate,
        CheckFailed(String),
        DownloadFailed(String),
        InstallFailed(String),
        Installed,
    }

    pub struct Updater;

    impl Updater {
        pub fn poll(&mut self) -> Option<Note> {
            None
        }

        pub fn state(&self) -> State {
            State::Idle
        }

        pub fn current(&self) -> Version {
            Version(env!("CARGO_PKG_VERSION").into())
        }

        pub fn checking(&self) -> bool {
            false
        }

        pub fn up_to_date(&self) -> bool {
            false
        }

        pub fn update_now(&mut self) {}

        pub fn later(&mut self) {}

        pub fn skip(&mut self) -> Result<(), String> {
            Ok(())
        }

        pub fn checks_at_startup(&self) -> bool {
            false
        }

        pub fn set_checks_at_startup(&mut self, _: bool) -> Result<(), String> {
            Ok(())
        }

        pub fn check_now(&mut self) {}

        pub fn install(&mut self, _: Option<&std::path::Path>) {}
    }
}
