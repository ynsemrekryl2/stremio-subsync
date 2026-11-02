//! The AIOStreams link and the API keys entered on the configure page
//! (SS-061). They are kept in one small JSON file (`SUBSYNC_SECRETS_FILE`,
//! readable by the service only) and win over the environment variables of
//! the same meaning; removing one on the page is saved too, so the
//! environment's value does not come back. Values never leave this module
//! except to build the clients and in a backup the page downloads (SS-063):
//! the page otherwise only learns whether each is set and where it came
//! from, and nothing here prints them. A restored backup also brings its
//! install token, which then wins over `SUBSYNC_TOKEN`, so the Stremio
//! install link stays the same on a new server.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::TranslateSettings;
use crate::streams::{AioStreams, ManifestUrlError};

/// A link or key the add-on reads at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Name {
    AioStreams,
    SubDl,
    AltyaziDb,
    SubSource,
    Wyzie,
    OpenRouter,
}

impl Name {
    pub const ALL: [Name; 6] = [
        Name::AioStreams,
        Name::SubDl,
        Name::AltyaziDb,
        Name::SubSource,
        Name::Wyzie,
        Name::OpenRouter,
    ];

    /// The environment variable that holds it when the page does not.
    pub fn env(self) -> &'static str {
        match self {
            Name::AioStreams => "SUBSYNC_AIOSTREAMS_URL",
            Name::SubDl => "SUBDL_API_KEY",
            Name::AltyaziDb => "ALTYAZIDB_API_KEY",
            Name::SubSource => "SUBSOURCE_API_KEY",
            Name::Wyzie => "WYZIE_API_KEY",
            Name::OpenRouter => "OPENROUTER_API_KEY",
        }
    }
}

/// Where a value in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Page,
    Environment,
}

/// What the page shows for one name: never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: Name,
    pub set: bool,
    pub from: Option<Origin>,
}

/// Why a change was refused. Carries no part of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A key holds whitespace, a quote or a control character.
    Value(Name),
    AioStreams(ManifestUrlError),
    /// The AIOStreams field holds SubSync's own install link.
    OwnLink,
    /// A backup's token is short or holds characters a path cannot.
    Token,
    /// Not a SubSync backup, or one from a later version.
    Backup,
    /// No file to save into.
    Off,
    Save(std::io::ErrorKind),
}

/// The values in use: the page's, then the environment's.
pub struct Secrets {
    file: Option<PathBuf>,
    /// From a restored backup; wins over the environment's.
    token: Option<String>,
    /// From the file; an empty value was removed on the page.
    saved: BTreeMap<Name, String>,
    env: BTreeMap<Name, String>,
}

impl Secrets {
    /// Reads `file` (missing is empty) and takes each name's environment
    /// value from `env`. A file that cannot be read or parsed is ignored, so
    /// a broken file never stops the server; the error is returned to log.
    pub fn load(
        file: Option<PathBuf>,
        env: impl Fn(&str) -> Option<String>,
    ) -> (Self, Option<LoadError>) {
        let env = Name::ALL
            .into_iter()
            .filter_map(|name| {
                let value = env(name.env())?.trim().to_owned();
                (!value.is_empty()).then_some((name, value))
            })
            .collect();
        let (saved, error) = match file.as_deref().map(read) {
            Some(Ok(saved)) => (saved, None),
            Some(Err(err)) => (File::default(), Some(err)),
            None => (File::default(), None),
        };
        let token = saved.token.filter(|t| valid_token(t));
        let secrets = Self {
            file,
            token,
            saved: saved.values,
            env,
        };
        (secrets, error)
    }

    /// The install token a restored backup brought, if any.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// The value to use, if any.
    pub fn get(&self, name: Name) -> Option<&str> {
        match self.saved.get(&name) {
            Some(value) => Some(value.as_str()).filter(|v| !v.is_empty()),
            None => self.env.get(&name).map(String::as_str),
        }
    }

    fn origin(&self, name: Name) -> Option<Origin> {
        match self.saved.get(&name) {
            Some(value) if value.is_empty() => None,
            Some(_) => Some(Origin::Page),
            None => self.env.get(&name).map(|_| Origin::Environment),
        }
    }

    pub fn view(&self) -> Vec<Entry> {
        Name::ALL
            .into_iter()
            .map(|name| {
                let from = self.origin(name);
                Entry {
                    name,
                    set: from.is_some(),
                    from,
                }
            })
            .collect()
    }

    /// Checks and saves `changes`: a value sets the name, `None` removes
    /// it. Names left out keep what they had. `token` is the install
    /// token, refused inside the AIOStreams link. Nothing is saved when
    /// any change is refused.
    pub fn change(
        &mut self,
        changes: &BTreeMap<Name, Option<String>>,
        token: &str,
    ) -> Result<(), Refused> {
        let Some(file) = self.file.clone() else {
            return Err(Refused::Off);
        };
        let mut saved = self.saved.clone();
        for (&name, value) in changes {
            let value = match value.as_deref().map(str::trim) {
                None | Some("") => String::new(),
                Some(value) => {
                    check(name, value, token)?;
                    value.to_owned()
                }
            };
            saved.insert(name, value);
        }
        write(&file, self.token.as_deref(), &saved).map_err(|err| Refused::Save(err.kind()))?;
        self.saved = saved;
        Ok(())
    }

    /// Everything a new server needs: `token`, every value in use (from the
    /// page or the environment) and the saved translation choice.
    pub fn backup(&self, token: &str, settings: Option<TranslateSettings>) -> Backup {
        Backup {
            kind: BACKUP_KIND.to_owned(),
            version: BACKUP_VERSION,
            token: token.to_owned(),
            values: Name::ALL
                .into_iter()
                .filter_map(|name| Some((name, self.get(name)?.to_owned())))
                .collect(),
            settings,
        }
    }

    /// Takes `backup` whole: its token, and its values in place of all
    /// others; a name it lacks is saved removed, so the environment's does
    /// not come back. Nothing is saved when any part is refused. The
    /// translation choice is the caller's to save.
    pub fn restore(&mut self, backup: &Backup) -> Result<(), Refused> {
        let Some(file) = self.file.clone() else {
            return Err(Refused::Off);
        };
        if backup.kind != BACKUP_KIND || backup.version > BACKUP_VERSION {
            return Err(Refused::Backup);
        }
        if !valid_token(&backup.token) {
            return Err(Refused::Token);
        }
        let mut saved = BTreeMap::new();
        for name in Name::ALL {
            let value = backup.values.get(&name).map_or("", |v| v.trim());
            if !value.is_empty() {
                check(name, value, &backup.token)?;
            }
            saved.insert(name, value.to_owned());
        }
        write(&file, Some(&backup.token), &saved).map_err(|err| Refused::Save(err.kind()))?;
        self.token = Some(backup.token.clone());
        self.saved = saved;
        Ok(())
    }
}

/// Marks a backup file as SubSync's.
const BACKUP_KIND: &str = "subsync-backup";
const BACKUP_VERSION: u32 = 1;

/// The file the page downloads and uploads (SS-063). Holds every secret:
/// the user keeps it like a password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    #[serde(rename = "subsync")]
    kind: String,
    version: u32,
    pub token: String,
    #[serde(default)]
    values: BTreeMap<Name, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<TranslateSettings>,
}

/// An install token: long enough to guess no faster than
/// `SUBSYNC_TOKEN`'s rule, and only characters a URL path keeps as they are.
pub fn valid_token(token: &str) -> bool {
    token.len() >= crate::MIN_TOKEN_LEN
        && token.len() <= 128
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether `value` may be saved, or tried, for `name`.
pub fn check(name: Name, value: &str, token: &str) -> Result<(), Refused> {
    if name == Name::AioStreams {
        if value.contains(token) {
            return Err(Refused::OwnLink);
        }
        return AioStreams::from_manifest_url(value)
            .map(drop)
            .map_err(Refused::AioStreams);
    }
    // These go into headers and query strings as they are.
    if value
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '\\'))
    {
        return Err(Refused::Value(name));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Read(std::io::ErrorKind),
    Malformed,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(default)]
    values: BTreeMap<Name, String>,
}

fn read(path: &Path) -> Result<File, LoadError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(File::default()),
        Err(err) => return Err(LoadError::Read(err.kind())),
    };
    serde_json::from_slice(&bytes).map_err(|_| LoadError::Malformed)
}

/// Writes the whole file, readable by its owner only; a crash leaves the
/// old file or the new one.
fn write(path: &Path, token: Option<&str>, values: &BTreeMap<Name, String>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(&File {
        token: token.map(str::to_owned),
        values: values.clone(),
    })
    .map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    out.write_all(&json)?;
    out.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const LINK: &str = "https://aio.test/abc/manifest.json";

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "subsync-secrets-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .join("secrets.json")
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    fn changes(pairs: &[(Name, Option<&str>)]) -> BTreeMap<Name, Option<String>> {
        pairs
            .iter()
            .map(|(name, value)| (*name, value.map(str::to_owned)))
            .collect()
    }

    fn entry(secrets: &Secrets, name: Name) -> Entry {
        secrets.view().into_iter().find(|e| e.name == name).unwrap()
    }

    #[test]
    fn the_page_wins_over_the_environment_and_removal_is_kept() {
        let path = temp_file("order");
        let (mut secrets, error) = Secrets::load(
            Some(path.clone()),
            env(&[("SUBDL_API_KEY", " env-key "), ("WYZIE_API_KEY", "")]),
        );
        assert_eq!(error, None, "no file yet is not an error");
        assert_eq!(secrets.get(Name::SubDl), Some("env-key"));
        assert_eq!(secrets.get(Name::Wyzie), None, "empty is unset");
        assert_eq!(entry(&secrets, Name::SubDl).from, Some(Origin::Environment));

        secrets
            .change(
                &changes(&[(Name::SubDl, Some("page-key")), (Name::OpenRouter, None)]),
                TOKEN,
            )
            .unwrap();
        assert_eq!(secrets.get(Name::SubDl), Some("page-key"));

        // A restart reads the file back; a removed key stays removed.
        let (mut again, _) =
            Secrets::load(Some(path.clone()), env(&[("SUBDL_API_KEY", "env-key")]));
        assert_eq!(again.get(Name::SubDl), Some("page-key"));
        assert_eq!(entry(&again, Name::SubDl).from, Some(Origin::Page));
        again
            .change(&changes(&[(Name::SubDl, None)]), TOKEN)
            .unwrap();
        let (again, _) = Secrets::load(Some(path.clone()), env(&[("SUBDL_API_KEY", "env-key")]));
        assert_eq!(again.get(Name::SubDl), None);
        assert_eq!(
            entry(&again, Name::SubDl),
            Entry {
                name: Name::SubDl,
                set: false,
                from: None
            }
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_file_is_private_and_the_view_holds_no_value() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_file("private");
        let (mut secrets, _) = Secrets::load(Some(path.clone()), env(&[]));
        secrets
            .change(
                &changes(&[
                    (Name::AioStreams, Some(LINK)),
                    (Name::OpenRouter, Some("sk-or-secret")),
                ]),
                TOKEN,
            )
            .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(!path.with_extension("json.tmp").exists());
        let view = serde_json::to_string(&secrets.view()).unwrap();
        assert!(!view.contains("sk-or-secret") && !view.contains("aio.test"));
        assert!(view.contains(r#""name":"openrouter","set":true,"from":"page""#));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn bad_values_are_refused_and_nothing_is_saved() {
        let path = temp_file("refused");
        let (mut secrets, _) = Secrets::load(Some(path.clone()), env(&[]));
        let refused = [
            (Name::SubDl, "two words", Refused::Value(Name::SubDl)),
            (Name::Wyzie, "quo\"te", Refused::Value(Name::Wyzie)),
            (
                Name::AioStreams,
                "https://aio.test/abc/configure",
                Refused::AioStreams(ManifestUrlError::NotManifest),
            ),
            (
                Name::AioStreams,
                &format!("https://subs.test/{TOKEN}/manifest.json"),
                Refused::OwnLink,
            ),
        ];
        for (name, value, why) in refused {
            let batch = changes(&[(Name::OpenRouter, Some("ok-key")), (name, Some(value))]);
            assert_eq!(secrets.change(&batch, TOKEN), Err(why), "{value}");
        }
        assert_eq!(
            secrets.get(Name::OpenRouter),
            None,
            "the good one waits too"
        );
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_backup_restores_the_token_and_every_value() {
        let old = temp_file("backup-old");
        let (mut secrets, _) = Secrets::load(
            Some(old.clone()),
            env(&[
                ("WYZIE_API_KEY", "env-wyzie"),
                ("SUBDL_API_KEY", "env-subdl"),
            ]),
        );
        secrets
            .change(
                &changes(&[(Name::AioStreams, Some(LINK)), (Name::SubDl, None)]),
                TOKEN,
            )
            .unwrap();
        let backup = secrets.backup(TOKEN, None);
        let json = serde_json::to_value(&backup).unwrap();
        assert_eq!(json["subsync"], "subsync-backup");
        assert_eq!(json["token"], TOKEN);
        assert_eq!(
            json["values"],
            serde_json::json!({ "aiostreams": LINK, "wyzie": "env-wyzie" }),
            "values in use, from the page or the environment; removed ones left out"
        );

        // A new server whose environment has other keys.
        let new = temp_file("backup-new");
        let (mut fresh, _) = Secrets::load(
            Some(new.clone()),
            env(&[("OPENROUTER_API_KEY", "other"), ("WYZIE_API_KEY", "other")]),
        );
        assert_eq!(fresh.token(), None);
        fresh.restore(&backup).unwrap();
        let (fresh, error) =
            Secrets::load(Some(new.clone()), env(&[("OPENROUTER_API_KEY", "other")]));
        assert_eq!(error, None);
        assert_eq!(fresh.token(), Some(TOKEN));
        assert_eq!(fresh.get(Name::AioStreams), Some(LINK));
        assert_eq!(fresh.get(Name::Wyzie), Some("env-wyzie"));
        assert_eq!(
            fresh.get(Name::OpenRouter),
            None,
            "the backup is the whole truth"
        );

        // A change after the restore keeps the token.
        let (mut fresh, _) = Secrets::load(Some(new.clone()), env(&[]));
        fresh
            .change(&changes(&[(Name::SubDl, Some("k"))]), TOKEN)
            .unwrap();
        let (fresh, _) = Secrets::load(Some(new.clone()), env(&[]));
        assert_eq!(fresh.token(), Some(TOKEN));
        for path in [old, new] {
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn a_bad_backup_is_refused_whole() {
        let path = temp_file("backup-bad");
        let (mut secrets, _) = Secrets::load(Some(path.clone()), env(&[]));
        let good = secrets.backup(TOKEN, None);
        let edited = |edit: fn(&mut serde_json::Value)| {
            let mut json = serde_json::to_value(&good).unwrap();
            edit(&mut json);
            serde_json::from_value::<Backup>(json).unwrap()
        };
        for (backup, refused) in [
            (edited(|j| j["token"] = "short".into()), Refused::Token),
            (
                edited(|j| j["token"] = format!("{TOKEN}/x").into()),
                Refused::Token,
            ),
            (edited(|j| j["subsync"] = "other".into()), Refused::Backup),
            (edited(|j| j["version"] = 2.into()), Refused::Backup),
            (
                edited(|j| j["values"]["subdl"] = "two words".into()),
                Refused::Value(Name::SubDl),
            ),
        ] {
            assert_eq!(secrets.restore(&backup), Err(refused));
        }
        assert!(!path.exists());
        assert_eq!(secrets.token(), None);
    }

    #[test]
    fn a_broken_file_falls_back_to_the_environment() {
        let path = temp_file("broken");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let (secrets, error) =
            Secrets::load(Some(path.clone()), env(&[("SUBSYNC_AIOSTREAMS_URL", LINK)]));
        assert_eq!(error, Some(LoadError::Malformed));
        assert_eq!(secrets.get(Name::AioStreams), Some(LINK));
        let (mut off, _) = Secrets::load(None, env(&[]));
        assert_eq!(
            off.change(&changes(&[(Name::SubDl, Some("k"))]), TOKEN),
            Err(Refused::Off)
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
