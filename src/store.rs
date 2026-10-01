//! "Database" = a directory of plain `.md` files (+ zip backup / restore).
use anyhow::{bail, Context, Result};
use chrono::Local;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self> {
        let root = match std::env::var_os("RUSTPAD_HOME") {
            Some(p) => PathBuf::from(p),
            None => dirs::data_dir().context("cannot find data dir")?.join("rustpad"),
        };
        for d in ["notes", "backups", "trash"] {
            fs::create_dir_all(root.join(d))?;
        }
        Ok(Self { root })
    }

    pub fn notes_dir(&self) -> PathBuf {
        self.root.join("notes")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.notes_dir().join(format!("{name}.md"))
    }

    /// Make a safe file stem from user input.
    pub fn sanitize(raw: &str) -> Result<String> {
        let s = raw.trim().trim_end_matches(".md");
        let s: String = s
            .chars()
            .map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c })
            .collect();
        let s = s.trim().trim_start_matches('.').to_string();
        if s.is_empty() {
            bail!("empty note name");
        }
        Ok(s)
    }

    /// Note names, newest first.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut v: Vec<(std::time::SystemTime, String)> = vec![];
        for e in fs::read_dir(self.notes_dir())? {
            let e = e?;
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("md") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                    v.push((t, stem.to_string()));
                }
            }
        }
        v.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(v.into_iter().map(|x| x.1).collect())
    }

    pub fn exists(&self, name: &str) -> bool {
        self.path(name).exists()
    }

    pub fn load(&self, name: &str) -> Result<String> {
        Ok(fs::read_to_string(self.path(name))?)
    }

    /// Atomic write: temp file + rename, so a crash never leaves a half-written note.
    pub fn save(&self, name: &str, text: &str) -> Result<()> {
        let tmp = self.notes_dir().join(format!(".{name}.md.tmp"));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, self.path(name))?;
        Ok(())
    }

    pub fn rename(&self, old: &str, new: &str) -> Result<()> {
        if self.exists(new) {
            bail!("'{new}' already exists");
        }
        fs::rename(self.path(old), self.path(new))?;
        Ok(())
    }

    /// Delete = move to trash/ (recoverable by hand).
    pub fn trash(&self, name: &str) -> Result<()> {
        let stamp = Local::now().format("%Y%m%d-%H%M%S");
        let dst = self.root.join("trash").join(format!("{name}.{stamp}.md"));
        fs::rename(self.path(name), dst)?;
        Ok(())
    }

    /// Zip every note. Returns the zip path.
    pub fn backup(&self, out: Option<PathBuf>, prefix: &str) -> Result<PathBuf> {
        let out = out.unwrap_or_else(|| {
            self.backups_dir()
                .join(format!("{prefix}-{}.zip", Local::now().format("%Y%m%d-%H%M%S")))
        });
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut zw = ZipWriter::new(fs::File::create(&out)?);
        let opt = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for name in self.list()? {
            zw.start_file(format!("notes/{name}.md"), opt)?;
            zw.write_all(&fs::read(self.path(&name))?)?;
        }
        zw.finish()?;
        Ok(out)
    }

    pub fn latest_backup(&self) -> Option<PathBuf> {
        let mut v: Vec<_> = fs::read_dir(self.backups_dir())
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("zip"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        v.sort();
        v.pop().map(|x| x.1)
    }

    /// Replace all notes with the contents of a backup zip.
    /// The current state is zipped first (`pre-restore-*.zip`) so restore is reversible.
    pub fn restore(&self, zip_path: &Path) -> Result<usize> {
        let mut ar = ZipArchive::new(fs::File::open(zip_path).context("cannot open zip")?)?;
        let mut files: Vec<(String, Vec<u8>)> = vec![];
        for i in 0..ar.len() {
            let mut e = ar.by_index(i)?;
            if e.is_dir() {
                continue;
            }
            // zip-slip safe: only keep the final file name, only *.md
            let Some(p) = e.enclosed_name() else { continue };
            let Some(fname) = p.file_name().and_then(|f| f.to_str()).map(String::from) else { continue };
            if !fname.ends_with(".md") || fname.starts_with('.') {
                continue;
            }
            let mut buf = vec![];
            e.read_to_end(&mut buf)?;
            files.push((fname, buf));
        }
        if files.is_empty() {
            bail!("no .md notes found in that zip");
        }
        self.backup(None, "pre-restore")?;
        for name in self.list()? {
            fs::remove_file(self.path(&name))?;
        }
        for (fname, data) in &files {
            fs::write(self.notes_dir().join(fname), data)?;
        }
        Ok(files.len())
    }
}
