//! Catalog transport and persistent downloads. Templates own SBL validation.

use crate::{amf, template::Template};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

// Bound catalog memory and the amount of UI work for a single refresh.
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_CATALOG_ENTRIES: usize = 200;

#[derive(Deserialize)]
struct Catalog {
    version: u32,
    layouts: Vec<CatalogEntry>,
}

#[derive(Deserialize)]
struct CatalogEntry {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    template_url: String,
}

#[derive(Clone)]
pub struct CloudLayout {
    pub id: String,
    pub name: String,
    pub description: String,
    template_url: Url,
}

#[derive(Clone)]
pub struct LocalLayout {
    pub path: PathBuf,
    pub name: String,
}

fn check_url(url: &Url) -> Result<(), String> {
    let loopback = match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if url.host().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("Use a direct HTTPS URL without a username, password or fragment. HTTP is allowed only on localhost for testing.".into());
    }
    Ok(())
}

fn parse_catalog(bytes: &[u8], source: &Url) -> Result<Vec<CloudLayout>, String> {
    let catalog: Catalog = serde_json::from_slice(bytes).map_err(|e| {
        format!(
            "Invalid catalog JSON at line {}, column {}",
            e.line(),
            e.column()
        )
    })?;
    if catalog.version != 1 {
        return Err("Unsupported catalog version; this app supports version 1.".into());
    }
    if catalog.layouts.len() > MAX_CATALOG_ENTRIES {
        return Err(format!(
            "A catalog can contain up to {MAX_CATALOG_ENTRIES} layouts."
        ));
    }
    let mut ids = HashSet::new();
    catalog
        .layouts
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            if entry.id.is_empty()
                || entry.id.len() > 80
                || !entry
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || !ids.insert(entry.id.to_ascii_lowercase())
            {
                return Err(format!(
                    "Layout {} needs an id of 1–80 letters, numbers, hyphens or underscores, unique regardless of letter case.",
                    index + 1
                ));
            }
            if entry.name.trim().is_empty() {
                return Err(format!("Layout {} needs a name.", index + 1));
            }
            let template_url = source
                .join(&entry.template_url)
                .map_err(|_| format!("Layout {} has an invalid template_url.", index + 1))?;
            check_url(&template_url).map_err(|e| format!("Layout {}: {e}", index + 1))?;
            Ok(CloudLayout {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                template_url,
            })
        })
        .collect()
}

fn fetch(url: &Url, limit: usize) -> Result<Vec<u8>, String> {
    check_url(url)?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .max_redirects(0)
        .build()
        .new_agent();
    let mut response = agent.get(url.as_str()).call().map_err(|e| match e {
        ureq::Error::StatusCode(code) => format!("Server returned HTTP {code}. Check the catalog URL and file access."),
        ureq::Error::Timeout(_) => "The request timed out. Check your connection and try again.".into(),
        _ => "Could not retrieve the file. Check the connection and use a direct HTTPS file URL (redirects are not followed).".into(),
    })?;
    if response.status() != 200 {
        return Err(format!(
            "Server returned HTTP {}. Use a direct file URL.",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The download was interrupted or timed out. Try again.".to_string())?;
    if bytes.len() > limit {
        return Err(format!(
            "Download exceeds the {} MiB file budget.",
            limit / (1024 * 1024)
        ));
    }
    Ok(bytes)
}

pub fn fetch_catalog(source: &str) -> Result<Vec<CloudLayout>, String> {
    let url = Url::parse(source.trim())
        .map_err(|_| "Enter a valid catalog URL in Settings.".to_string())?;
    check_url(&url)?;
    let bytes = fetch(&url, MAX_CATALOG_BYTES)?;
    parse_catalog(&bytes, &url)
}

pub fn local_layouts(folder: &Path) -> Result<Vec<LocalLayout>, String> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Could not read the library folder: {e}")),
    };
    let mut layouts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Could not read a library entry: {e}"))?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sbl"))
        {
            layouts.push(LocalLayout {
                name: entry
                    .path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: entry.path(),
            });
        }
    }
    layouts.sort_by_cached_key(|l| l.name.to_lowercase());
    Ok(layouts)
}

/// Keep the exact validated server file, including metadata understood by other readers.
/// Existing versions remain recoverable; selecting unchanged content is idempotent.
pub fn download(layout: &CloudLayout, folder: &Path) -> Result<(Template, PathBuf), String> {
    let bytes = fetch(&layout.template_url, amf::MAX_FILE)?;
    let template = Template::from_bytes(&bytes)
        .map_err(|e| format!("The downloaded layout is not supported: {e}"))?;
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("Could not create the library folder: {e}"))?;
    // The prefix also avoids Windows reserved device names such as CON and NUL.
    let path = folder.join(format!("layout-{}.sbl", layout.id));
    if path.exists() {
        let existing = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut old = Vec::new();
        existing
            .take(amf::MAX_FILE as u64 + 1)
            .read_to_end(&mut old)
            .map_err(|e| e.to_string())?;
        if old == bytes {
            return Ok((template, path));
        }
        let backup = path.with_extension(format!(
            "sbl.backup-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S-%f")
        ));
        std::fs::copy(&path, backup)
            .map_err(|e| format!("Could not back up the previous layout: {e}"))?;
    }
    let temp = path.with_extension("sbl.download");
    let result = std::fs::write(&temp, &bytes).and_then(|()| std::fs::rename(&temp, &path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Could not save the downloaded layout: {e}"));
    }
    Ok((template, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    /// Real HTTP responses exercise transport and persistence without AWS credentials.
    fn server(responses: Vec<(&'static str, u16, Vec<u8>)>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            for (path, status, body) in responses {
                let started = std::time::Instant::now();
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && started.elapsed() < Duration::from_secs(10) =>
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("HTTP fixture did not receive its request: {e}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") && request.len() < 4096 {
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(String::from_utf8(request)
                    .unwrap()
                    .starts_with(&format!("GET {path} HTTP/1.1\r\n")));
                let headers = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).unwrap();
                // A size-limited reader can close the socket before the whole body is sent.
                let _ = socket.write_all(&body);
            }
        });
        (address, handle)
    }

    fn catalog(entries: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"version": 1, "layouts": entries})).unwrap()
    }

    #[test]
    fn catalog_owns_urls_ids_and_version_validation() {
        let base = Url::parse("https://example.com/library/catalog.json").unwrap();
        let entry = serde_json::json!({"id":"summer_1", "name":"Summer", "template_url":"layouts/summer.sbl?signature=fixture"});
        let entries = parse_catalog(&catalog(serde_json::json!([entry.clone()])), &base).unwrap();
        assert_eq!(
            entries[0].template_url.as_str(),
            "https://example.com/library/layouts/summer.sbl?signature=fixture"
        );
        assert_eq!(entries[0].description, "");
        for id in ["", "../outside", "a/b", "a\\b", "x.y"] {
            let mut bad = entry.clone();
            bad["id"] = id.into();
            assert!(parse_catalog(&catalog(serde_json::json!([bad])), &base).is_err());
        }
        assert!(parse_catalog(
            &catalog(serde_json::json!([entry.clone(), entry.clone()])),
            &base
        )
        .is_err());
        let mut case_alias = entry.clone();
        case_alias["id"] = "SUMMER_1".into();
        assert!(parse_catalog(&catalog(serde_json::json!([entry, case_alias])), &base).is_err());
        assert!(parse_catalog(br#"{"version":2,"layouts":[]}"#, &base).is_err());
        assert!(parse_catalog(b"not JSON", &base).is_err());
        for text in [
            "file:///C:/layout.sbl",
            "http://example.com/file",
            "https://user:pass@example.com/a",
            "https://example.com/a#fragment",
        ] {
            assert!(check_url(&Url::parse(text).unwrap()).is_err());
        }
        assert!(check_url(&Url::parse("http://127.0.0.1:1234/a").unwrap()).is_ok());
        assert!(check_url(&Url::parse("http://[::1]:1234/a").unwrap()).is_ok());
        let many = (0..=MAX_CATALOG_ENTRIES).map(|i| serde_json::json!({"id":format!("layout{i}"), "name":"Many", "template_url":"a.sbl"})).collect::<Vec<_>>();
        assert!(parse_catalog(&catalog(serde_json::json!(many)), &base).is_err());
    }

    #[test]
    fn cloud_selection_downloads_exact_file_backs_up_changes_and_reopens_offline() {
        let dir =
            std::env::temp_dir().join(format!("radiate-library-{}-offline", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let folder = dir.join("Downloaded layouts");
        let source = dir.join("fixture.sbl");
        let mut template = Template::builtin(false);
        template.name = "First edition".into();
        template.save(&source).unwrap();
        let first = std::fs::read(&source).unwrap();
        template.name = "Second edition".into();
        template.save(&source).unwrap();
        let second = std::fs::read(&source).unwrap();
        let catalog = catalog(
            serde_json::json!([{"id":"CON", "name":"My design", "description":"An offline test", "template_url":"../layouts/design.sbl?token=fixture"}]),
        );
        let (base, server) = server(vec![
            ("/catalogs/catalog.json", 200, catalog),
            ("/layouts/design.sbl?token=fixture", 200, first.clone()),
            ("/layouts/design.sbl?token=fixture", 200, first.clone()),
            ("/layouts/design.sbl?token=fixture", 200, second.clone()),
            (
                "/layouts/design.sbl?token=fixture",
                200,
                b"not a layout".to_vec(),
            ),
        ]);
        let entries = fetch_catalog(&format!("{base}/catalogs/catalog.json")).unwrap();
        let (loaded, path) = download(&entries[0], &folder).unwrap();
        assert_eq!(loaded.name, "First edition");
        assert_eq!(path, folder.join("layout-CON.sbl"));
        assert_eq!(std::fs::read(&path).unwrap(), first);
        download(&entries[0], &folder).unwrap();
        assert_eq!(
            std::fs::read_dir(&folder).unwrap().count(),
            1,
            "unchanged downloads make no backups"
        );
        download(&entries[0], &folder).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), second);
        let backup = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p != &path)
            .unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), first);
        assert!(download(&entries[0], &folder)
            .err()
            .unwrap()
            .contains("not supported"));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            second,
            "invalid downloads must not replace saved layouts"
        );
        server.join().unwrap();
        let local = local_layouts(&folder).unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(
            Template::load(&local[0].path).unwrap().name,
            "Second edition"
        );
        assert!(local_layouts(&dir.join("not created yet"))
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn http_errors_and_oversized_responses_are_actionable() {
        let (base, server) = server(vec![
            ("/missing", 404, Vec::new()),
            ("/large", 200, vec![b' '; MAX_CATALOG_BYTES + 1]),
            ("/redirect", 302, Vec::new()),
        ]);
        assert!(fetch_catalog(&format!("{base}/missing"))
            .err()
            .unwrap()
            .contains("404"));
        assert!(fetch_catalog(&format!("{base}/large"))
            .err()
            .unwrap()
            .contains("budget"));
        assert!(fetch_catalog(&format!("{base}/redirect")).is_err());
        server.join().unwrap();
    }
}
