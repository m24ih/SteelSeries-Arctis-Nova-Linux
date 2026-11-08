//! User-selectable language preferences, independent of any UI framework.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Turkish,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preference {
    System,
    Turkish,
    English,
}

impl Preference {
    pub fn resolve(self, system: Language) -> Language {
        match self {
            Self::System => system,
            Self::Turkish => Language::Turkish,
            Self::English => Language::English,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u8,
    language: Preference,
}

/// Missing settings mean system language; malformed or newer settings are errors.
pub fn load(path: &Path) -> io::Result<Preference> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Preference::System),
        Err(error) => return Err(error),
    };
    let settings: Settings = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if settings.version != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unsupported language settings version",
        ));
    }
    Ok(settings.language)
}

/// Write through a temporary file in the same directory before replacing settings.
pub fn save(path: &Path, preference: Preference) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(
        &mut temporary,
        &Settings {
            version: 1,
            language: preference,
        },
    )
    .map_err(io::Error::other)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Apply the installer's language only when no user preference exists.
pub fn initialize(path: &Path, installer_choice: Preference) -> io::Result<Preference> {
    if path.exists() {
        return load(path);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(
        &mut temporary,
        &Settings {
            version: 1,
            language: installer_choice,
        },
    )
    .map_err(io::Error::other)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(installer_choice),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => load(path),
        Err(error) => Err(error.error),
    }
}

/// Select Turkish for Turkish UI language IDs and English for other languages.
pub fn from_windows_language_id(language_id: u16) -> Language {
    if language_id & 0x03ff == 0x001f {
        Language::Turkish
    } else {
        Language::English
    }
}

#[cfg(windows)]
pub fn system_language() -> Language {
    from_windows_language_id(unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_choice_overrides_system() {
        assert_eq!(
            Preference::System.resolve(Language::Turkish),
            Language::Turkish
        );
        assert_eq!(
            Preference::English.resolve(Language::Turkish),
            Language::English
        );
        assert_eq!(
            Preference::Turkish.resolve(Language::English),
            Language::Turkish
        );
        assert_eq!(from_windows_language_id(0x041f), Language::Turkish);
        assert_eq!(from_windows_language_id(0x0409), Language::English);
    }

    #[test]
    fn preferences_persist_and_can_return_to_system() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings").join("language.json");
        assert_eq!(load(&path).unwrap(), Preference::System);
        save(&path, Preference::Turkish).unwrap();
        assert_eq!(load(&path).unwrap(), Preference::Turkish);
        save(&path, Preference::English).unwrap();
        assert_eq!(load(&path).unwrap(), Preference::English);
        save(&path, Preference::System).unwrap();
        assert_eq!(load(&path).unwrap(), Preference::System);
    }

    #[test]
    fn malformed_and_future_settings_are_not_silently_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("language.json");
        for contents in [
            "{",
            r#"{"version":2,"language":"english"}"#,
            r#"{"version":1,"language":"german"}"#,
            r#"{"version":1,"language":"english","extra":1}"#,
        ] {
            fs::write(&path, contents).unwrap();
            assert!(load(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[test]
    fn installer_preserves_existing_user_choice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("language.json");
        assert_eq!(
            initialize(&path, Preference::Turkish).unwrap(),
            Preference::Turkish
        );
        save(&path, Preference::English).unwrap();
        assert_eq!(
            initialize(&path, Preference::Turkish).unwrap(),
            Preference::English
        );
        fs::write(&path, "invalid").unwrap();
        assert!(initialize(&path, Preference::Turkish).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "invalid");
    }
}
