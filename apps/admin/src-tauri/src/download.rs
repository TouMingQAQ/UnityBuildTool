//! Public artifact browsing and bounded-memory downloads.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

pub type Fault = (u16, String);
fn forbidden() -> Fault {
    (
        403,
        "只能访问构建目录内的普通文件，禁止链接或目录联接".into(),
    )
}
fn io_fault(e: io::Error) -> Fault {
    (
        if e.kind() == io::ErrorKind::NotFound {
            404
        } else {
            500
        },
        if e.kind() == io::ErrorKind::NotFound {
            "文件或文件夹不存在"
        } else {
            "读取失败，请在管理端检查目录权限"
        }
        .into(),
    )
}
fn linked(m: &Metadata) -> bool {
    if m.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
// Inspect all ancestors, including the configured root itself (not just descendants).
fn reject_links(path: &Path) -> Result<(), Fault> {
    let mut current = PathBuf::new();
    for c in path.components() {
        current.push(c.as_os_str());
        if matches!(c, Component::Prefix(_)) {
            continue;
        }
        if linked(&fs::symlink_metadata(&current).map_err(io_fault)?) {
            return Err(forbidden());
        }
    }
    Ok(())
}
pub fn resolve(root: &Path, relative: &str) -> Result<(PathBuf, Metadata), Fault> {
    if relative.contains(['\\', ':', '\0'])
        || relative.starts_with('/')
        || relative.split('/').any(|c| c == "..")
    {
        return Err(forbidden());
    }
    if Path::new(relative)
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(forbidden());
    }
    let absolute = if root.is_absolute() {
        root.to_owned()
    } else {
        std::env::current_dir().map_err(io_fault)?.join(root)
    };
    reject_links(&absolute)?;
    let base = fs::canonicalize(&absolute).map_err(io_fault)?;
    let candidate = base.join(relative);
    reject_links(&candidate)?;
    let real = fs::canonicalize(&candidate).map_err(io_fault)?;
    if !real.starts_with(&base) {
        return Err(forbidden());
    }
    let meta = fs::symlink_metadata(&real).map_err(io_fault)?;
    if linked(&meta) || !(meta.is_file() || meta.is_dir()) {
        return Err(forbidden());
    }
    Ok((real, meta))
}
fn open_checked(root: &Path, relative: &str) -> Result<File, Fault> {
    let (path, _) = resolve(root, relative)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000).share_mode(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path).map_err(io_fault)?;
    let meta = file.metadata().map_err(io_fault)?;
    if linked(&meta) || !meta.is_file() {
        return Err(forbidden());
    }
    // Validate once more after opening. Windows keeps the file from being replaced.
    resolve(root, relative)?;
    Ok(file)
}

#[derive(Clone)]
pub struct Root {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
}
pub fn roots(config: &Value, runtime: &Path) -> Vec<Root> {
    // An envelope is never a download root. Callers must resolve one project first.
    if config.get("projects").is_some() || config["enabled"].as_bool() == Some(false) {
        return vec![];
    }
    let project = config["projectPath"].as_str().unwrap_or("");
    let output = config["outputBase"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("Builds");
    if project.is_empty() {
        return vec![];
    }
    let project = if Path::new(project).is_absolute() {
        PathBuf::from(project)
    } else {
        runtime.join(project)
    };
    let base = if Path::new(output).is_absolute() {
        PathBuf::from(output)
    } else {
        project.join(output)
    };
    let mut result = vec![Root {
        id: "builds".into(),
        name: "构建目录".into(),
        directory: base.clone(),
    }];
    let nodes = config["buildNodes"]
        .as_array()
        .or_else(|| config["queue"].as_array());
    for node in nodes.into_iter().flatten() {
        let legacy = &config["profileConfigs"][node["assetPath"].as_str().unwrap_or("")];
        let custom = node["customBuildDir"]
            .as_str()
            .or_else(|| node["sign"]["customBuildDir"].as_str())
            .or_else(|| legacy["customBuildDir"].as_str())
            .or_else(|| legacy["sign"]["customBuildDir"].as_str())
            .unwrap_or("")
            .trim();
        if custom.is_empty() {
            continue;
        }
        let normalized = custom.replace('\\', "/");
        let normalized = normalized.trim_start_matches('/');
        let dir = if Path::new(custom).is_absolute() {
            PathBuf::from(custom)
        } else if normalized.to_lowercase().starts_with("builds/") {
            base.parent().unwrap_or(&base).join(normalized)
        } else {
            base.join(normalized)
        };
        if result.iter().any(|r| r.directory == dir) {
            continue;
        }
        let hash = Sha256::digest(dir.to_string_lossy().as_bytes());
        let id = format!(
            "node-{}",
            hash[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        result.push(Root {
            id,
            name: format!(
                "{} · 自定义构建目录",
                node["name"].as_str().unwrap_or("节点")
            ),
            directory: dir,
        });
    }
    result
}
pub fn select<'a>(roots: &'a [Root], id: &str) -> Result<&'a Root, Fault> {
    roots
        .iter()
        .find(|r| {
            if id.is_empty() {
                r.id == "builds"
            } else {
                r.id == id
            }
        })
        .ok_or_else(|| (404, "构建目录不存在".into()))
}
pub fn listing(roots: &[Root], id: &str, relative: &str) -> Result<Value, Fault> {
    let public = roots
        .iter()
        .map(|r| json!({"id":r.id,"name":r.name}))
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Ok(json!({"roots":[],"root":"","path":"","entries":[]}));
    }
    let root = select(roots, id)?;
    let (directory, meta) = match resolve(&root.directory, relative) {
        Err((404, _)) if relative.is_empty() => {
            return Ok(json!({"roots":public,"root":root.id,"path":"","entries":[]}))
        }
        other => other?,
    };
    if !meta.is_dir() {
        return Err((400, "请选择文件夹".into()));
    }
    let _guards = lock_ancestors(&directory)?;
    let mut entries = Vec::new();
    for item in fs::read_dir(directory).map_err(io_fault)? {
        let item = item.map_err(io_fault)?;
        if linked(&fs::symlink_metadata(item.path()).map_err(io_fault)?) {
            continue;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        let path = if relative.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", relative.trim_end_matches('/'), name)
        };
        let (_, m) = resolve(&root.directory, &path)?;
        let modified = m
            .modified()
            .ok()
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
        entries.push(json!({"name":name,"path":path,"type":if m.is_dir() {"directory"} else {"file"},"size":if m.is_file(){m.len()}else{0},"modified":modified}));
    }
    entries.sort_by(|a, b| {
        a["type"]
            .as_str()
            .cmp(&b["type"].as_str())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    Ok(json!({"roots":public,"root":root.id,"path":relative,"entries":entries}))
}
// Holding directory handles without FILE_SHARE_DELETE closes Windows junction / rename
// races for the lifetime of a response or ZIP walk. Reparse points are opened themselves.
fn lock_ancestors(path: &Path) -> Result<Vec<File>, Fault> {
    let mut guards = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let mut current = PathBuf::new();
        for part in path.components() {
            current.push(part.as_os_str());
            if matches!(part, Component::Prefix(_)) {
                continue;
            }
            let mut opts = OpenOptions::new();
            opts.read(true)
                .share_mode(1)
                .custom_flags(0x02000000 | 0x00200000);
            let file = opts.open(&current).map_err(io_fault)?;
            if linked(&file.metadata().map_err(io_fault)?) {
                return Err(forbidden());
            }
            guards.push(file);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    Ok(guards)
}
pub struct Download {
    file: File,
    _temporary: Option<NamedTempFile>,
    _guards: Vec<File>,
    pub size: u64,
    pub name: String,
    pub mime: &'static str,
}
impl Read for Download {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}
pub fn file(root: &Root, relative: &str) -> Result<Download, Fault> {
    let (path, m) = resolve(&root.directory, relative)?;
    if !m.is_file() {
        return Err((400, "请选择文件".into()));
    }
    let guards = lock_ancestors(path.parent().ok_or_else(forbidden)?)?;
    let file = open_checked(&root.directory, relative)?;
    let size = file.metadata().map_err(io_fault)?.len();
    Ok(Download {
        file,
        _temporary: None,
        _guards: guards,
        size,
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        mime: "application/octet-stream",
    })
}
pub fn folder(root: &Root, relative: &str) -> Result<Download, Fault> {
    let (directory, m) = resolve(&root.directory, relative)?;
    if !m.is_dir() {
        return Err((400, "请选择文件夹".into()));
    }
    let guards = lock_ancestors(&directory)?;
    let mut temporary = NamedTempFile::new().map_err(io_fault)?;
    {
        let mut zip = ZipWriter::new(temporary.as_file_mut());
        archive(&mut zip, &directory, "", 0)?;
        zip.finish().map_err(|_| (500, "生成 ZIP 失败".into()))?;
    }
    temporary
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(io_fault)?;
    let file = temporary.reopen().map_err(io_fault)?;
    let size = file.metadata().map_err(io_fault)?.len();
    let name = if relative.is_empty() {
        root.name.clone()
    } else {
        directory
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    Ok(Download {
        file,
        _temporary: Some(temporary),
        _guards: guards,
        size,
        name: format!("{name}.zip"),
        mime: "application/zip",
    })
}
fn archive(
    zip: &mut ZipWriter<&mut File>,
    root: &Path,
    prefix: &str,
    depth: usize,
) -> Result<(), Fault> {
    if depth > 128 {
        return Err((413, "目录层级过深，请分别下载文件".into()));
    }
    let (directory, _) = resolve(root, prefix)?;
    let _guards = lock_ancestors(&directory)?;
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(true);
    for item in fs::read_dir(directory).map_err(io_fault)? {
        let item = item.map_err(io_fault)?;
        if linked(&fs::symlink_metadata(item.path()).map_err(io_fault)?) {
            continue;
        }
        let name = item.file_name().to_str().ok_or_else(forbidden)?.to_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let (_, meta) = resolve(root, &relative)?;
        if meta.is_dir() {
            zip.add_directory(format!("{relative}/"), options)
                .map_err(|_| (500, "生成 ZIP 失败".into()))?;
            archive(zip, root, &relative, depth + 1)?;
        } else {
            let mut file = open_checked(root, &relative)?;
            zip.start_file(&relative, options)
                .map_err(|_| (500, "生成 ZIP 失败".into()))?;
            io::copy(&mut file, zip).map_err(io_fault)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envelope_and_disabled_projects_never_become_roots() {
        let runtime = tempfile::tempdir().unwrap();
        assert!(roots(&json!({"projects":[{"projectPath":"secret"}],"projectPath":"legacy","outputBase":"Builds"}), runtime.path()).is_empty());
        assert!(roots(
            &json!({"enabled":false,"projectPath":"secret","outputBase":"Builds"}),
            runtime.path()
        )
        .is_empty());
    }
    #[test]
    fn relative_project_uses_runtime_directly() {
        let runtime = tempfile::tempdir().unwrap();
        let result = roots(
            &json!({"projectPath":"project","outputBase":"Builds"}),
            runtime.path(),
        );
        assert_eq!(result[0].directory, runtime.path().join("project/Builds"));
    }
    #[test]
    fn traversal_and_streaming_zip() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("folder/data.bin"), b"artifact").unwrap();
        for path in [
            "../secret",
            "a/../../secret",
            "/etc/passwd",
            "C:/Windows",
            "a\\b",
            "a\0b",
            "file:stream",
        ] {
            assert_eq!(resolve(dir.path(), path).unwrap_err().0, 403);
        }
        let root = Root {
            id: "builds".into(),
            name: "构建目录".into(),
            directory: dir.path().to_owned(),
        };
        assert_eq!(
            listing(&[root.clone()], "builds", "").unwrap()["entries"][0]["type"],
            "directory"
        );
        let mut zipped = folder(&root, "folder").unwrap();
        let temp_path = zipped._temporary.as_ref().unwrap().path().to_owned();
        let mut archive = zip::ZipArchive::new(zipped.file.try_clone().unwrap()).unwrap();
        let mut text = String::new();
        archive
            .by_name("data.bin")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "artifact");
        drop(archive);
        let mut bytes = vec![];
        zipped.read_to_end(&mut bytes).unwrap();
        assert!(!bytes.is_empty());
        drop(zipped);
        assert!(!temp_path.exists());
    }
    #[cfg(unix)]
    #[test]
    fn links_are_denied() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("safe"), b"secret").unwrap();
        std::os::unix::fs::symlink(dir.path().join("safe"), dir.path().join("link")).unwrap();
        assert_eq!(resolve(dir.path(), "link").unwrap_err().0, 403);
    }
    #[cfg(windows)]
    #[test]
    fn junctions_are_denied() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = dir.path().join("junction");
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(resolve(dir.path(), "junction").unwrap_err().0, 403);
        assert_eq!(resolve(&link, "").unwrap_err().0, 403);
    }
}
