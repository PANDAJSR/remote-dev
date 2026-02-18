use axum::{extract::Query, response::IntoResponse, Json};

use crate::models::file::{
    CopyFileRequest, CreateFolderRequest, DeleteFileRequest, DirQuery, FileContent,
    FileEntry, FileQuery, RenameFileRequest, SaveFileRequest, SaveResponse,
};

// 标准化路径（统一使用正斜杠）
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

pub async fn get_directory_tree(Query(query): Query<DirQuery>) -> impl IntoResponse {
    let path = match query.path {
        Some(p) => p,
        None => {
            match std::env::current_dir() {
                Ok(dir) => dir.to_string_lossy().to_string(),
                Err(_) => {
                    return Json(serde_json::json!({
                        "error": "Failed to get current directory"
                    }))
                    .into_response();
                }
            }
        }
    };

    let tree = build_tree(&path, 0);
    Json(tree).into_response()
}

// 获取指定目录的直接子项（用于懒加载）
pub async fn get_children(Query(query): Query<DirQuery>) -> impl IntoResponse {
    let path = query.path.unwrap_or_default();

    if path.is_empty() {
        return Json(serde_json::json!({
            "error": "Path is required"
        }))
        .into_response();
    }

    let path_obj = std::path::Path::new(&path);

    if !path_obj.is_dir() {
        return Json(serde_json::json!({
            "error": "Path is not a directory"
        }))
        .into_response();
    }

    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let mut children: Vec<FileEntry> = entries
                .filter_map(|entry| {
                    entry.ok().map(|e| {
                        let path = e.path();
                        let path_str = normalize_path(&path.to_string_lossy().to_string());
                        let name = e.file_name().to_string_lossy().to_string();
                        let is_directory = path.is_dir();

                        FileEntry {
                            name,
                            path: path_str,
                            is_directory,
                            children: if is_directory { Some(vec![]) } else { None },
                        }
                    })
                })
                .collect();

            children.sort_by(|a, b| match (a.is_directory, b.is_directory) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            });

            Json(children).into_response()
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to read directory: {}", e)
        }))
        .into_response(),
    }
}

fn build_tree(path: &str, depth: u8) -> FileEntry {
    let path_normalized = normalize_path(path);
    let name = std::path::Path::new(&path_normalized)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path_normalized)
        .to_string();

    let is_directory = std::path::Path::new(path).is_dir();

    let children = if is_directory && depth < 3 {
        match std::fs::read_dir(path) {
            Ok(entries) => {
                let mut children_vec: Vec<FileEntry> = entries
                    .filter_map(|entry| {
                        entry.ok().map(|e| {
                            let child_path = e.path();
                            let path_str = normalize_path(&child_path.to_string_lossy().to_string());
                            build_tree(&path_str, depth + 1)
                        })
                    })
                    .collect();
                children_vec.sort_by(|a, b| match (a.is_directory, b.is_directory) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                });
                Some(children_vec)
            }
            Err(_) => None,
        }
    } else {
        None
    };

    FileEntry {
        name,
        path: path_normalized,
        is_directory,
        children,
    }
}

pub async fn get_file_content(Query(query): Query<FileQuery>) -> impl IntoResponse {
    let path = query.path;

    let path_obj = std::path::Path::new(&path);

    if !path_obj.is_file() {
        return Json(serde_json::json!({
            "error": "Path is not a file or does not exist"
        }))
        .into_response();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let name = path_obj
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let file_content = FileContent {
                path: path.clone(),
                content,
                name,
            };
            Json(file_content).into_response()
        }
        Err(e) => Json(serde_json::json!({
            "error": format!("Failed to read file: {}", e)
        }))
        .into_response(),
    }
}

pub async fn save_file_content(Json(payload): Json<SaveFileRequest>) -> impl IntoResponse {
    let path = payload.path;
    let path_obj = std::path::Path::new(&path);

    if let Some(parent) = path_obj.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Json(SaveResponse {
                success: false,
                message: format!("Failed to create directory: {}", e),
            })
            .into_response();
        }
    }

    match std::fs::write(&path, payload.content) {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File saved successfully".to_string(),
        })
        .into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to write file: {}", e),
        })
        .into_response(),
    }
}

pub async fn rename_file(Json(payload): Json<RenameFileRequest>) -> impl IntoResponse {
    let old_path_obj = std::path::Path::new(&payload.old_path);
    if !old_path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Source path does not exist".to_string(),
        })
        .into_response();
    }

    let new_path_obj = std::path::Path::new(&payload.new_path);
    if let Some(parent) = new_path_obj.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Json(SaveResponse {
                success: false,
                message: format!("Failed to create directory: {}", e),
            })
            .into_response();
        }
    }

    match std::fs::rename(&payload.old_path, &payload.new_path) {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File renamed successfully".to_string(),
        })
        .into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to rename file: {}", e),
        })
        .into_response(),
    }
}

pub async fn delete_file(Query(query): Query<DeleteFileRequest>) -> impl IntoResponse {
    let path = query.path;
    let path_obj = std::path::Path::new(&path);

    if !path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Path does not exist".to_string(),
        })
        .into_response();
    }

    let result = if path_obj.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };

    match result {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File deleted successfully".to_string(),
        })
        .into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to delete file: {}", e),
        })
        .into_response(),
    }
}

pub async fn create_folder(Json(payload): Json<CreateFolderRequest>) -> impl IntoResponse {
    let path = payload.path;
    let path_obj = std::path::Path::new(&path);

    if path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Path already exists".to_string(),
        })
        .into_response();
    }

    match std::fs::create_dir_all(&path) {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "Folder created successfully".to_string(),
        })
        .into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to create folder: {}", e),
        })
        .into_response(),
    }
}

pub async fn copy_file(Json(payload): Json<CopyFileRequest>) -> impl IntoResponse {
    let source_path_obj = std::path::Path::new(&payload.source_path);
    if !source_path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Source path does not exist".to_string(),
        })
        .into_response();
    }

    let target_path_obj = std::path::Path::new(&payload.target_path);
    if let Some(parent) = target_path_obj.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Json(SaveResponse {
                success: false,
                message: format!("Failed to create directory: {}", e),
            })
            .into_response();
        }
    }

    let result = if source_path_obj.is_dir() {
        copy_dir_recursive(&payload.source_path, &payload.target_path)
    } else {
        std::fs::copy(&payload.source_path, &payload.target_path).map(|_| ())
    };

    match result {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File copied successfully".to_string(),
        })
        .into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to copy file: {}", e),
        })
        .into_response(),
    }
}

fn copy_dir_recursive(src: &str, dst: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = std::path::Path::new(dst).join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(src_path.to_str().unwrap(), dst_path.to_str().unwrap())?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
