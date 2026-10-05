//! The payload folder beside the exe: `payload\manifest.json` and the setup exes it names.
//!
//! The manifest is what makes an offline install trustworthy: a USB stick that lost half a file
//! on the way to the event must be caught before the installer runs, not by the installer.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    #[serde(default)]
    pub built: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

/// A manifest names files in its own folder and nowhere else.
pub fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && name != "."
        && name != ".."
        && name.to_ascii_lowercase().ends_with(".exe")
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    // make-bundle writes plain UTF-8, but a manifest edited in Notepad arrives with a BOM.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let manifest: Manifest = serde_json::from_str(text).map_err(|e| format!("manifest.json could not be read: {e}"))?;
    if manifest.schema != 1 {
        return Err(format!("manifest.json is schema {}, and this setup reads schema 1", manifest.schema));
    }
    let mut seen = std::collections::HashSet::new();
    for item in &manifest.items {
        if !safe_file_name(&item.file) {
            return Err(format!("manifest.json names a file outside the payload folder: {}", item.file));
        }
        if item.sha256.len() != 64 || !item.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("manifest.json has no usable sha256 for {}", item.id));
        }
        if !seen.insert(item.id.as_str()) {
            return Err(format!("manifest.json lists {} twice", item.id));
        }
    }
    Ok(manifest)
}

/// What the payload folder holds, as far as can be told without reading every byte.
#[derive(Debug, Clone, Default)]
pub struct Payload {
    pub dir: PathBuf,
    /// The folder exists.
    pub present: bool,
    pub manifest: Option<Manifest>,
    /// Why the manifest could not be used, in words for the person at the laptop.
    pub problem: Option<String>,
}

pub fn load(dir: &Path) -> Payload {
    let mut payload = Payload { dir: dir.to_path_buf(), present: dir.is_dir(), ..Default::default() };
    if !payload.present {
        return payload;
    }
    match std::fs::read_to_string(dir.join("manifest.json")) {
        Ok(text) => match parse(&text) {
            Ok(manifest) => payload.manifest = Some(manifest),
            Err(problem) => payload.problem = Some(problem),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            payload.problem = Some("The payload folder has no manifest.json, so nothing in it can be checked.".into());
        }
        Err(e) => payload.problem = Some(format!("manifest.json could not be read: {e}")),
    }
    payload
}

/// The quick check, made when the window opens: is the file there and the right length?
pub fn quick_check(dir: &Path, item: &Item) -> Result<PathBuf, String> {
    let path = dir.join(&item.file);
    match std::fs::metadata(&path) {
        Ok(meta) if meta.len() == item.size => Ok(path),
        Ok(meta) => Err(format!(
            "{} is {} bytes and the manifest says {}. The copy is incomplete.",
            item.file,
            meta.len(),
            item.size
        )),
        Err(_) => Err(format!("{} is named in the manifest but is not in the payload folder.", item.file)),
    }
}

pub fn sha256_file(path: &Path, mut progress: impl FnMut(u64)) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        done += read as u64;
        progress(done);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The full check, made just before the installer runs.
pub fn verify(path: &Path, expected_sha256: &str, progress: impl FnMut(u64)) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let actual = sha256_file(path, progress).map_err(|e| format!("{name} could not be read: {e}"))?;
    if actual.eq_ignore_ascii_case(expected_sha256) {
        Ok(())
    } else {
        Err(format!("{name} is damaged: its sha256 is not the one recorded for it. Nothing was installed from it."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-setup-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn manifest_for(file: &str, sha: &str, size: u64) -> String {
        format!(r#"{{"schema":1,"built":"2026-10-05T12:00:00Z","items":[{{"id":"catalyst","version":"2.8.0","file":"{file}","sha256":"{sha}","size":{size}}}]}}"#)
    }

    #[test]
    fn parses_what_make_bundle_writes() {
        let m = parse(&manifest_for("Catalyst_2.8.0_x64-setup.exe", ABC, 3)).unwrap();
        assert_eq!(m.items[0].id, "catalyst");
        assert_eq!(m.items[0].version, "2.8.0");
        assert_eq!(m.items[0].size, 3);
    }

    #[test]
    fn a_bom_does_not_stop_it() {
        let text = format!("\u{feff}{}", manifest_for("a.exe", ABC, 3));
        assert!(parse(&text).is_ok());
    }

    #[test]
    fn refuses_paths_that_leave_the_folder() {
        for bad in ["..\\\\evil.exe", "sub/setup.exe", "C:setup.exe", "setup.bat", ""] {
            let err = parse(&manifest_for(bad, ABC, 3)).unwrap_err();
            assert!(err.contains("outside the payload folder"), "{bad}: {err}");
        }
    }

    #[test]
    fn refuses_a_missing_hash_a_wrong_schema_and_a_duplicate() {
        assert!(parse(&manifest_for("a.exe", "abc", 3)).unwrap_err().contains("sha256"));
        assert!(parse(r#"{"schema":2,"items":[]}"#).unwrap_err().contains("schema 2"));
        let twice = format!(
            r#"{{"schema":1,"items":[{{"id":"a","version":"1","file":"a.exe","sha256":"{ABC}","size":1}},{{"id":"a","version":"1","file":"b.exe","sha256":"{ABC}","size":1}}]}}"#
        );
        assert!(parse(&twice).unwrap_err().contains("twice"));
        assert!(parse("not json").is_err());
    }

    #[test]
    fn hashes_a_file_and_reports_bytes_as_it_goes() {
        let dir = scratch("hash");
        let path = dir.join("abc.exe");
        File::create(&path).unwrap().write_all(b"abc").unwrap();
        let mut seen = 0;
        assert_eq!(sha256_file(&path, |n| seen = n).unwrap(), ABC);
        assert_eq!(seen, 3);
        assert!(verify(&path, &ABC.to_uppercase(), |_| {}).is_ok());
    }

    #[test]
    fn a_damaged_file_is_caught() {
        let dir = scratch("damaged");
        let path = dir.join("abc.exe");
        File::create(&path).unwrap().write_all(b"abd").unwrap();
        let err = verify(&path, ABC, |_| {}).unwrap_err();
        assert!(err.contains("damaged"), "{err}");
    }

    #[test]
    fn the_quick_check_catches_a_short_copy_and_a_missing_file() {
        let dir = scratch("quick");
        File::create(dir.join("a.exe")).unwrap().write_all(b"ab").unwrap();
        let item = |file: &str| Item { id: "a".into(), version: "1".into(), file: file.into(), sha256: ABC.into(), size: 3 };
        assert!(quick_check(&dir, &item("a.exe")).unwrap_err().contains("incomplete"));
        assert!(quick_check(&dir, &item("b.exe")).unwrap_err().contains("not in the payload folder"));
        File::create(dir.join("c.exe")).unwrap().write_all(b"abc").unwrap();
        assert_eq!(quick_check(&dir, &item("c.exe")).unwrap(), dir.join("c.exe"));
    }

    #[test]
    fn loading_a_folder_says_what_is_wrong_with_it() {
        let none = load(&std::env::temp_dir().join("catalyst-setup-no-such-folder"));
        assert!(!none.present && none.manifest.is_none() && none.problem.is_none());

        let empty = scratch("empty-payload");
        let loaded = load(&empty);
        assert!(loaded.present && loaded.manifest.is_none());
        assert!(loaded.problem.unwrap().contains("no manifest.json"));

        let good = scratch("good-payload");
        std::fs::write(good.join("manifest.json"), manifest_for("a.exe", ABC, 3)).unwrap();
        assert_eq!(load(&good).manifest.unwrap().items.len(), 1);
    }
}
