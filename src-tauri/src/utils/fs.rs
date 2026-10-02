use std::path::Path;
use std::fs;
use thiserror::Error;
use walkdir::WalkDir;
use crate::models::FileInfo;

#[derive(Error, Debug)]
pub enum FsError {
    #[error("文件不存在：{0}")]
    FileNotFound(String),
    #[error("无法访问文件：{0}")]
    AccessDenied(String),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
}

pub async fn get_file_info(path: &str) -> Result<(u64, String), FsError> {
    let path = Path::new(path);
    
    if !path.exists() {
        return Err(FsError::FileNotFound(path.to_string_lossy().to_string()));
    }

    let metadata = fs::metadata(path)?;
    let size = metadata.len();
    
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok((size, name))
}

pub fn is_directory(path: &str) -> bool {
    Path::new(path).is_dir()
}

pub fn collect_files_from_dir(dir_path: &str) -> Result<Vec<FileInfo>, FsError> {
    let dir_path = Path::new(dir_path);
    
    if !dir_path.exists() || !dir_path.is_dir() {
        return Err(FsError::FileNotFound(dir_path.to_string_lossy().to_string()));
    }

    let mut files = Vec::new();
    
    for entry in WalkDir::new(dir_path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let metadata = fs::metadata(path)?;
        
        if metadata.is_file() {
            let file_path = path.to_string_lossy().to_string();
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let size = metadata.len();
            
            // 计算相对于根目录的相对路径，用于在 Alist 中保持目录结构
            let relative_path = path
                .strip_prefix(dir_path)
                .ok()
                .and_then(|p| p.to_str())
                .unwrap_or("");
            
            files.push(FileInfo {
                path: file_path,
                name: file_name,
                size,
                relative_path: Some(relative_path.to_string()),
            });
        }
    }

    Ok(files)
}

pub fn ensure_dir_exists(path: &Path) -> Result<(), std::io::Error> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

/// 上传完成标记前缀：带此前缀表示本地数据已全部上传、可删除
pub const UPLOADED_DELETE_PREFIX: &str = "delete-";

/// Windows 资源管理器默认按 MAX_PATH=260 处理完整路径，超长后即使改名成功也无法在资源管理器中删除
const MAX_PATH_UTF16: usize = 259;
/// NTFS 单级名称上限 255 个 UTF-16 字符
const MAX_NAME_UTF16: usize = 255;

/// delete- 标记失败类型：Skip 为永久性不满足（重试无意义），Io 为改名被占用等瞬时错误（可延迟重试）
#[derive(Debug)]
pub enum MarkError {
    Skip(String),
    Io(String),
}

impl std::fmt::Display for MarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkError::Skip(m) => write!(f, "{}", m),
            MarkError::Io(m) => write!(f, "{}", m),
        }
    }
}

/// 将本地路径改名为 delete- 前缀，标记已上传完成。
/// 返回新路径；路径不存在、已带前缀（幂等返回原路径）、名称或完整路径超长时返回 MarkError::Skip，
/// 改名被占用（资源管理器/杀毒软件打开中）等瞬时失败返回 MarkError::Io。
pub fn mark_uploaded_delete_prefix(path: &str) -> Result<String, MarkError> {
    let src = Path::new(path);
    if !src.exists() {
        return Err(MarkError::Skip(format!(
            "路径不存在（可能已被移动或删除）: {}",
            path
        )));
    }
    let parent = src
        .parent()
        .ok_or_else(|| MarkError::Skip(format!("无法获取父目录: {}", path)))?;
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| MarkError::Skip(format!("路径名称不是有效 UTF-8: {}", path)))?
        .to_string();

    if name.starts_with(UPLOADED_DELETE_PREFIX) {
        return Ok(path.to_string());
    }

    let base = format!("{}{}", UPLOADED_DELETE_PREFIX, name);
    if base.encode_utf16().count() > MAX_NAME_UTF16 {
        return Err(MarkError::Skip(format!(
            "名称加 delete- 前缀后超过 {} 字符上限: {}",
            MAX_NAME_UTF16, path
        )));
    }

    // 目标已存在时加序号（delete-A-1、delete-A-2...），每一档都做路径长度预检
    let mut candidate = base.clone();
    let mut counter = 1;
    let new_path = loop {
        let candidate_path = parent.join(&candidate);
        let candidate_len = candidate_path
            .to_string_lossy()
            .encode_utf16()
            .count();
        if candidate_len > MAX_PATH_UTF16 {
            return Err(MarkError::Skip(format!(
                "完整路径加 delete- 前缀后超过 {} 字符上限（资源管理器将无法删除）: {}",
                MAX_PATH_UTF16 + 1,
                path
            )));
        }
        if !candidate_path.exists() {
            break candidate_path;
        }
        candidate = format!("{}-{}", base, counter);
        counter += 1;
    };

    fs::rename(src, &new_path).map_err(|e| {
        MarkError::Io(format!(
            "标记改名失败（可能被资源管理器/杀毒软件等占用）: {} -> {}, error={}",
            path,
            new_path.display(),
            e
        ))
    })?;

    Ok(new_path.to_string_lossy().to_string())
}
